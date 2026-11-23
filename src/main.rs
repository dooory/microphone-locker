#[cfg(windows)]
fn main() {
    if let Err(err) = run() {
        eprintln!("microphone-locker error: {err}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use microphone_locker::audio::AudioEvent;
    use microphone_locker::config::default_config_path;
    use microphone_locker::controller::AppController;
    use microphone_locker::windows_audio::WindowsAudioBackend;

    let config_path = default_config_path()?;
    let (backend, event_rx) = WindowsAudioBackend::start()?;
    let mut controller = AppController::start(backend, config_path.clone())?;

    let snapshot = controller.snapshot();
    println!(
        "Microphone Locker running (Default Volume: {}%, config: {})",
        snapshot.default_volume,
        config_path.display()
    );
    for mic in &snapshot.active_microphones {
        match mic.effective_target_volume {
            Some(target) => println!(
                "  Locked: {} -> {}% ({})",
                mic.friendly_name, target, mic.endpoint_id
            ),
            None => println!(
                "  Ignored: {} ({})",
                mic.friendly_name, mic.endpoint_id
            ),
        }
    }

    for event in event_rx {
        match &event {
            AudioEvent::MicrophoneConnected(mic) => {
                println!(
                    "Microphone connected: {} ({})",
                    mic.friendly_name, mic.endpoint_id
                );
            }
            AudioEvent::MicrophoneDisconnected { endpoint_id } => {
                println!("Microphone disconnected: {endpoint_id}");
            }
            AudioEvent::FriendlyNameChanged {
                endpoint_id,
                friendly_name,
            } => {
                println!("Microphone renamed: {friendly_name} ({endpoint_id})");
            }
            AudioEvent::ExternalVolumeChanged { .. } => {}
        }

        if let Err(err) = controller.handle_event(event) {
            eprintln!("event handling error: {err}");
        }
    }

    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("microphone-locker only supports Windows.");
    std::process::exit(1);
}
