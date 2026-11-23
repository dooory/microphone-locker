#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    if let Err(err) = run() {
        eprintln!("microphone-locker error: {err}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread;

    use microphone_locker::config::default_config_path;
    use microphone_locker::controller::AppController;
    use microphone_locker::ui;
    use microphone_locker::windows_audio::WindowsAudioBackend;

    let config_path = default_config_path()?;
    let (backend, event_rx) = WindowsAudioBackend::start()?;
    let controller = AppController::start(backend, config_path)?;
    let open_window_on_startup = controller.should_open_window_on_startup();

    let controller = Arc::new(Mutex::new(controller));
    let weak_controller = Arc::downgrade(&controller);
    let egui_ctx_slot = Arc::new(OnceLock::<eframe::egui::Context>::new());
    let audio_egui_ctx = Arc::clone(&egui_ctx_slot);

    // Process real-time Core Audio events independently of window visibility so
    // volume enforcement continues while minimized to the system tray.
    let audio_event_thread = thread::Builder::new()
        .name("audio-event-loop".to_string())
        .spawn(move || {
            for event in event_rx {
                let Some(ctrl) = weak_controller.upgrade() else {
                    break;
                };
                match ctrl.lock() {
                    Ok(mut ctrl) => {
                        if let Err(err) = ctrl.handle_event(event) {
                            eprintln!("event handling error: {err}");
                        }
                    }
                    Err(_) => break,
                }
                if let Some(ctx) = audio_egui_ctx.get() {
                    ctx.request_repaint();
                }
            }
        })?;

    let ui_result = ui::run_app(controller, open_window_on_startup, egui_ctx_slot);

    // Dropping the last Arc<Mutex<AppController>> inside run_app drops
    // WindowsAudioBackend, which stops the COM worker and closes event_rx.
    let _ = audio_event_thread.join();

    ui_result
}

#[cfg(not(windows))]
fn main() {
    eprintln!("microphone-locker only supports Windows.");
    std::process::exit(1);
}
