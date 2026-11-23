//! Windows single-instance guard using a named mutex and named activation event.
//!
//! When a second instance of Microphone Locker is launched, it detects the
//! existing named mutex, grants foreground activation permission via
//! `AllowSetForegroundWindow(ASFW_ANY)`, signals the named show-window event,
//! and exits immediately.

use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent,
    WaitForMultipleObjects,
};
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
use windows::core::{HSTRING, PCWSTR};

const DEFAULT_INSTANCE_PREFIX: &str = "Local\\MicrophoneLocker.SingleInstance";

/// RAII wrapper around an owned Win32 kernel `HANDLE` that closes it on `Drop`.
struct OwnedHandle(HANDLE);

// SAFETY: Win32 kernel mutex/event handles are process-wide and safe to transfer
// across threads when ownership and drop ordering are upheld.
unsafe impl Send for OwnedHandle {}

impl OwnedHandle {
    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: `self.0` is an owned Win32 kernel handle closed exactly once on drop.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

/// Non-owning `Send` wrapper for passing a kernel event `HANDLE` to the listener thread.
#[derive(Clone, Copy)]
struct SendHandle(HANDLE);

// SAFETY: The referenced handle is owned by `SingleInstanceGuard`, which signals
// `shutdown_event` and joins the listener thread before closing any handle.
unsafe impl Send for SendHandle {}

impl SendHandle {
    fn raw(self) -> HANDLE {
        self.0
    }
}

/// Outcome of attempting to acquire the single-instance guard at startup.
pub enum InstanceLock {
    /// This process is the primary running instance and owns the guard.
    Primary(SingleInstanceGuard),
    /// Another instance is already running and has been signaled to focus its window.
    SecondarySignaled,
}

/// Owns the named mutex, activation event, and optional listener thread.
/// Dropping this guard stops the listener thread and closes all Win32 handles.
pub struct SingleInstanceGuard {
    listener: Option<JoinHandle<()>>,
    // `Drop::drop` signals `shutdown_event` and joins `listener` before Rust drops
    // these `OwnedHandle` fields.
    shutdown_event: OwnedHandle,
    show_event: OwnedHandle,
    _mutex: OwnedHandle,
}

impl SingleInstanceGuard {
    /// Acquires the application-wide single-instance guard.
    pub fn acquire() -> Result<InstanceLock, String> {
        Self::acquire_named(DEFAULT_INSTANCE_PREFIX)
    }

    /// Acquires a single-instance guard using a custom kernel object prefix
    /// (useful for isolated tests).
    pub fn acquire_named(prefix: &str) -> Result<InstanceLock, String> {
        let mutex_name = HSTRING::from(format!("{prefix}.Mutex"));
        let event_name = HSTRING::from(format!("{prefix}.ShowEvent"));

        // SAFETY: `mutex_name` is a valid NUL-terminated UTF-16 HSTRING.
        let mutex = OwnedHandle(
            unsafe { CreateMutexW(None, false, PCWSTR(mutex_name.as_ptr())) }
                .map_err(|err| format!("CreateMutexW failed: {err}"))?,
        );
        // SAFETY: Checking thread-local last error immediately after CreateMutexW.
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;

        if already_exists {
            drop(mutex);
            signal_existing_instance(&event_name);
            return Ok(InstanceLock::SecondarySignaled);
        }

        // SAFETY: Creating named auto-reset event for secondary -> primary focus signaling.
        let show_event = OwnedHandle(
            unsafe { CreateEventW(None, false, false, PCWSTR(event_name.as_ptr())) }
                .map_err(|err| format!("CreateEventW(show_event) failed: {err}"))?,
        );

        // SAFETY: Creating unnamed manual-reset event to cleanly wake the listener thread on Drop.
        let shutdown_event = OwnedHandle(
            unsafe { CreateEventW(None, true, false, None) }
                .map_err(|err| format!("CreateEventW(shutdown_event) failed: {err}"))?,
        );

        Ok(InstanceLock::Primary(Self {
            listener: None,
            shutdown_event,
            show_event,
            _mutex: mutex,
        }))
    }

    /// Spawns a background listener thread that invokes `on_activate` whenever a
    /// secondary instance signals the named activation event.
    pub fn start_activation_listener<F>(&mut self, on_activate: F) -> Result<(), String>
    where
        F: Fn() + Send + 'static,
    {
        if self.listener.is_some() {
            return Ok(());
        }

        let shutdown = SendHandle(self.shutdown_event.raw());
        let show = SendHandle(self.show_event.raw());

        let handle = thread::Builder::new()
            .name("single-instance-listener".to_string())
            .spawn(move || {
                let handles = [shutdown.raw(), show.raw()];
                loop {
                    // SAFETY: Both event handles remain valid until `SingleInstanceGuard::drop`
                    // signals `shutdown_event` and joins this thread before closing handles.
                    let wait = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
                    if wait == WAIT_OBJECT_0 {
                        // Shutdown event signaled.
                        break;
                    } else if wait.0 == WAIT_OBJECT_0.0 + 1 {
                        // Show-window event signaled by a secondary instance.
                        on_activate();
                    } else {
                        break;
                    }
                }
            })
            .map_err(|err| format!("failed to spawn single-instance listener: {err}"))?;

        self.listener = Some(handle);
        Ok(())
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        // SAFETY: Wakes the listener thread so `join` completes before `OwnedHandle` fields drop.
        let _ = unsafe { SetEvent(self.shutdown_event.raw()) };
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
    }
}

fn signal_existing_instance(event_name: &HSTRING) {
    // Allow the existing primary instance to bring its window to the foreground.
    // SAFETY: Best-effort Win32 hint from the newly launched foreground process.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };

    // SAFETY: Opens the primary instance's named event with `EVENT_MODIFY_STATE`.
    if let Ok(event) = unsafe { OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(event_name.as_ptr())) }
    {
        let event = OwnedHandle(event);
        let _ = unsafe { SetEvent(event.raw()) };
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn second_instance_signals_primary_and_release_allows_new_primary() {
        let prefix = format!(
            "Local\\MicrophoneLocker.Test.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        let mut primary = match SingleInstanceGuard::acquire_named(&prefix).unwrap() {
            InstanceLock::Primary(guard) => guard,
            InstanceLock::SecondarySignaled => panic!("first acquisition should be primary"),
        };

        let (tx, rx) = mpsc::channel();
        primary
            .start_activation_listener(move || {
                let _ = tx.send(());
            })
            .unwrap();

        // Second acquisition detects existing instance and signals primary.
        match SingleInstanceGuard::acquire_named(&prefix).unwrap() {
            InstanceLock::Primary(_) => panic!("second acquisition should detect existing instance"),
            InstanceLock::SecondarySignaled => {}
        }

        rx.recv_timeout(Duration::from_secs(2))
            .expect("primary listener should receive activation signal");

        // Dropping primary releases the mutex so a subsequent acquisition becomes primary.
        drop(primary);

        match SingleInstanceGuard::acquire_named(&prefix).unwrap() {
            InstanceLock::Primary(_guard) => {}
            InstanceLock::SecondarySignaled => {
                panic!("acquisition after dropping primary should become primary")
            }
        }
    }
}
