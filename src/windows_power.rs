//! Windows power-resume monitor listening for `WM_POWERBROADCAST` wake events.
//!
//! Creates a lightweight hidden top-level window on a dedicated message-loop
//! thread (top-level rather than `HWND_MESSAGE`, because Windows only delivers
//! `WM_POWERBROADCAST` to top-level windows) and invokes a callback whenever
//! the system resumes from sleep.

use std::ffi::c_void;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetMessageW,
    GetWindowLongPtrW, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PostMessageW,
    PostQuitMessage, RegisterClassExW, SetWindowLongPtrW, TranslateMessage, WINDOW_EX_STYLE,
    WM_CLOSE, WM_DESTROY, WM_POWERBROADCAST, WNDCLASSEXW, WS_OVERLAPPED,
};
use windows::core::w;

type WakeCallback = Box<dyn Fn() + Send + 'static>;

/// Owns the hidden Windows power-resume listener thread; dropping it stops the
/// message loop and joins the thread cleanly.
pub struct PowerMonitor {
    hwnd: isize,
    worker: Option<JoinHandle<()>>,
}

impl PowerMonitor {
    /// Starts the hidden power-resume monitor window thread.
    pub fn start<F>(on_wake: F) -> Result<Self, String>
    where
        F: Fn() + Send + 'static,
    {
        let (init_tx, init_rx) = mpsc::sync_channel::<Result<isize, String>>(1);

        let worker = thread::Builder::new()
            .name("windows-power-monitor".to_string())
            .spawn(move || {
                run_power_window_loop(Box::new(on_wake), init_tx);
            })
            .map_err(|err| format!("failed to spawn power monitor thread: {err}"))?;

        match init_rx.recv() {
            Ok(Ok(hwnd)) => Ok(Self {
                hwnd,
                worker: Some(worker),
            }),
            Ok(Err(err)) => {
                let _ = worker.join();
                Err(err)
            }
            Err(_) => {
                let _ = worker.join();
                Err("power monitor thread exited before window creation".to_string())
            }
        }
    }
}

impl Drop for PowerMonitor {
    fn drop(&mut self) {
        if self.hwnd != 0 {
            let hwnd = HWND(self.hwnd as *mut c_void);
            // SAFETY: `hwnd` was created on the monitor thread; posting `WM_CLOSE`
            // triggers `DestroyWindow` -> `PostQuitMessage(0)` on that thread.
            let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn is_wake_broadcast(event: u32) -> bool {
    matches!(event, PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND)
}

fn run_power_window_loop(on_wake: WakeCallback, init_tx: mpsc::SyncSender<Result<isize, String>>) {
    let class_name = w!("MicrophoneLockerPowerMonitorWindow");

    // SAFETY: `GetModuleHandleW(None)` returns the current executable's HMODULE.
    let hmodule = match unsafe { GetModuleHandleW(None) } {
        Ok(m) => m,
        Err(err) => {
            let _ = init_tx.send(Err(format!("GetModuleHandleW failed: {err}")));
            return;
        }
    };
    let hinstance = HINSTANCE(hmodule.0);

    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(power_wnd_proc),
        hInstance: hinstance,
        lpszClassName: class_name,
        ..Default::default()
    };

    // SAFETY: `wc` is properly initialized and `class_name` is a static NUL-terminated UTF-16 string.
    let atom = unsafe { RegisterClassExW(&wc) };
    if atom == 0 {
        // SAFETY: Checking thread-local last error immediately after RegisterClassExW.
        let last_err = unsafe { GetLastError() };
        if last_err != ERROR_CLASS_ALREADY_EXISTS {
            let _ = init_tx.send(Err(format!(
                "RegisterClassExW failed with Win32 error {}",
                last_err.0
            )));
            return;
        }
    }

    // Create a hidden top-level window (no WS_VISIBLE) so Windows delivers WM_POWERBROADCAST.
    // SAFETY: Valid registered class name and current process HINSTANCE.
    let hwnd = match unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("Microphone Locker Power Monitor"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance),
            None,
        )
    } {
        Ok(hwnd) => hwnd,
        Err(err) => {
            let _ = init_tx.send(Err(format!("CreateWindowExW failed: {err}")));
            return;
        }
    };

    let callback_ptr = Box::into_raw(Box::new(on_wake));
    // SAFETY: Storing owned heap pointer in `GWLP_USERDATA`; freed in `WM_DESTROY`.
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, callback_ptr as isize);
    }

    let _ = init_tx.send(Ok(hwnd.0 as isize));

    let mut msg = MSG::default();
    // SAFETY: Standard Win32 message loop for the hidden window on this thread.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn power_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_POWERBROADCAST => {
            if is_wake_broadcast(wparam.0 as u32) {
                // SAFETY: `GWLP_USERDATA` points to a valid `Box<WakeCallback>` until `WM_DESTROY`.
                let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WakeCallback;
                if !ptr.is_null() {
                    let callback = unsafe { &*ptr };
                    callback();
                }
            }
            LRESULT(1)
        }
        WM_CLOSE => {
            // SAFETY: Destroys this thread's hidden window and triggers `WM_DESTROY`.
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: Take ownership back from `GWLP_USERDATA` once and drop it cleanly.
            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WakeCallback;
            if !ptr.is_null() {
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Box::from_raw(ptr));
                }
            }
            // SAFETY: Signals the thread's `GetMessageW` loop to exit cleanly.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: Forward all other messages to the default window procedure.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use windows::Win32::UI::WindowsAndMessaging::{PBT_APMPOWERSTATUSCHANGE, PBT_APMSUSPEND};

    use super::*;

    #[test]
    fn is_wake_broadcast_matches_only_resume_events() {
        assert!(is_wake_broadcast(PBT_APMRESUMEAUTOMATIC));
        assert!(is_wake_broadcast(PBT_APMRESUMESUSPEND));
        assert!(!is_wake_broadcast(PBT_APMSUSPEND));
        assert!(!is_wake_broadcast(PBT_APMPOWERSTATUSCHANGE));
    }

    #[test]
    fn power_monitor_invokes_callback_on_power_resume_broadcast_and_shuts_down_cleanly() {
        let (tx, rx) = mpsc::channel();
        let monitor = PowerMonitor::start(move || {
            let _ = tx.send(());
        })
        .expect("power monitor should start");

        let hwnd = HWND(monitor.hwnd as *mut c_void);
        // Ignore suspend broadcasts.
        unsafe {
            let _ = PostMessageW(
                Some(hwnd),
                WM_POWERBROADCAST,
                WPARAM(PBT_APMSUSPEND as usize),
                LPARAM(0),
            );
        }
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());

        // Deliver a resume broadcast and verify callback fires.
        unsafe {
            let _ = PostMessageW(
                Some(hwnd),
                WM_POWERBROADCAST,
                WPARAM(PBT_APMRESUMEAUTOMATIC as usize),
                LPARAM(0),
            );
        }
        rx.recv_timeout(Duration::from_secs(2))
            .expect("wake callback should fire on PBT_APMRESUMEAUTOMATIC");

        drop(monitor);
    }
}
