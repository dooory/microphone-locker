use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use microphone_locker::audio::{ActiveMicrophone, AudioBackend, AudioError, AudioEvent};
use microphone_locker::config::{ConfigError, LockPolicy};
use microphone_locker::controller::{
    AppCommand, AppController, AppSnapshot, ControllerError, MicrophoneSnapshot,
};
use tempfile::TempDir;

fn microphones_by_id(snapshot: &AppSnapshot) -> HashMap<&str, &MicrophoneSnapshot> {
    snapshot
        .active_microphones
        .iter()
        .map(|m| (m.endpoint_id.as_str(), m))
        .collect()
}

#[derive(Debug, Default)]
struct FakeAudioState {
    microphones: Vec<ActiveMicrophone>,
    set_calls: Vec<(String, f32)>,
}

#[derive(Debug, Clone, Default)]
struct FakeAudioBackend {
    state: Arc<Mutex<FakeAudioState>>,
}

impl FakeAudioBackend {
    fn with_microphones(microphones: Vec<ActiveMicrophone>) -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeAudioState {
                microphones,
                set_calls: Vec::new(),
            })),
        }
    }

    fn set_calls(&self) -> Vec<(String, f32)> {
        self.state.lock().unwrap().set_calls.clone()
    }

    fn clear_set_calls(&self) {
        self.state.lock().unwrap().set_calls.clear();
    }

    fn add_microphone(&self, mic: ActiveMicrophone) {
        self.state.lock().unwrap().microphones.push(mic);
    }

    fn remove_microphone(&self, endpoint_id: &str) {
        self.state
            .lock()
            .unwrap()
            .microphones
            .retain(|m| m.endpoint_id != endpoint_id);
    }

    fn current_volume(&self, endpoint_id: &str) -> Option<f32> {
        self.state
            .lock()
            .unwrap()
            .microphones
            .iter()
            .find(|m| m.endpoint_id == endpoint_id)
            .map(|m| m.volume_scalar)
    }
}

impl AudioBackend for FakeAudioBackend {
    fn list_active_microphones(&self) -> Result<Vec<ActiveMicrophone>, AudioError> {
        Ok(self.state.lock().unwrap().microphones.clone())
    }

    fn set_volume_scalar(&self, endpoint_id: &str, scalar: f32) -> Result<(), AudioError> {
        let mut state = self.state.lock().unwrap();
        state.set_calls.push((endpoint_id.to_string(), scalar));
        if let Some(mic) = state
            .microphones
            .iter_mut()
            .find(|m| m.endpoint_id == endpoint_id)
        {
            mic.volume_scalar = scalar;
            Ok(())
        } else {
            Err(AudioError::MicrophoneNotFound(endpoint_id.to_string()))
        }
    }
}

#[test]
fn first_run_initializes_default_volume_to_80_saves_config_enforces_active_microphones_and_opens_window() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir
        .path()
        .join("MicrophoneLocker")
        .join("config.toml");

    let backend = FakeAudioBackend::with_microphones(vec![
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            friendly_name: "Blue Yeti".to_string(),
            volume_scalar: 0.45,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-b}".to_string(),
            friendly_name: "Headset Mic".to_string(),
            volume_scalar: 1.0,
        },
    ]);

    let controller = AppController::start(backend.clone(), config_path.clone()).unwrap();

    assert!(controller.should_open_window_on_startup());

    let snapshot = controller.snapshot();
    assert_eq!(snapshot.default_volume, 80);
    assert_eq!(snapshot.active_microphones.len(), 2);

    let by_id = microphones_by_id(&snapshot);
    assert_eq!(by_id["{0.0.1.00000000}.{mic-a}"].friendly_name, "Blue Yeti");
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-a}"].lock_policy,
        LockPolicy::FollowDefault
    );
    assert_eq!(by_id["{0.0.1.00000000}.{mic-a}"].effective_target_volume, Some(80));
    assert_eq!(by_id["{0.0.1.00000000}.{mic-b}"].friendly_name, "Headset Mic");
    assert_eq!(by_id["{0.0.1.00000000}.{mic-b}"].effective_target_volume, Some(80));

    assert_eq!(backend.set_calls().len(), 2);
    assert!((backend.current_volume("{0.0.1.00000000}.{mic-a}").unwrap() - 0.80).abs() < 1e-6);
    assert!((backend.current_volume("{0.0.1.00000000}.{mic-b}").unwrap() - 0.80).abs() < 1e-6);

    assert!(config_path.exists());
    let saved_toml = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_toml.contains("default_volume = 80"));
    assert!(saved_toml.contains("{0.0.1.00000000}.{mic-a}"));
    assert!(saved_toml.contains("Blue Yeti"));
}

#[test]
fn subsequent_run_loads_saved_config_reconciles_microphones_starts_minimized_and_refreshes_friendly_name() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir
        .path()
        .join("MicrophoneLocker")
        .join("config.toml");

    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(
        &config_path,
        r#"
default_volume = 70

[microphones."{0.0.1.00000000}.{mic-follow}"]
friendly_name = "Old Name"
policy = "follow_default"

[microphones."{0.0.1.00000000}.{mic-custom}"]
friendly_name = "Shure SM7B"
policy = "custom"
target_volume = 55

[microphones."{0.0.1.00000000}.{mic-ignored}"]
friendly_name = "Virtual Cable"
policy = "ignored"
"#,
    )
    .unwrap();

    let backend = FakeAudioBackend::with_microphones(vec![
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-follow}".to_string(),
            friendly_name: "Blue Yeti Pro".to_string(),
            volume_scalar: 0.30,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-custom}".to_string(),
            friendly_name: "Shure SM7B".to_string(),
            volume_scalar: 0.90,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-ignored}".to_string(),
            friendly_name: "Virtual Cable".to_string(),
            volume_scalar: 0.15,
        },
    ]);

    let controller = AppController::start(backend.clone(), config_path.clone()).unwrap();

    assert!(!controller.should_open_window_on_startup());

    let snapshot = controller.snapshot();
    assert_eq!(snapshot.default_volume, 70);

    let by_id = microphones_by_id(&snapshot);
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-follow}"].friendly_name,
        "Blue Yeti Pro"
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-follow}"].effective_target_volume,
        Some(70)
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-custom}"].lock_policy,
        LockPolicy::Custom(55)
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-custom}"].effective_target_volume,
        Some(55)
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-ignored}"].lock_policy,
        LockPolicy::Ignored
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-ignored}"].effective_target_volume,
        None
    );

    assert_eq!(backend.set_calls().len(), 2);
    assert!((backend.current_volume("{0.0.1.00000000}.{mic-follow}").unwrap() - 0.70).abs() < 1e-6);
    assert!((backend.current_volume("{0.0.1.00000000}.{mic-custom}").unwrap() - 0.55).abs() < 1e-6);
    assert!((backend.current_volume("{0.0.1.00000000}.{mic-ignored}").unwrap() - 0.15).abs() < 1e-6);

    let saved_toml = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_toml.contains("Blue Yeti Pro"));
    assert!(!saved_toml.contains("Old Name"));
}

#[test]
fn external_volume_change_reverts_locked_microphone_and_ignores_sub_half_percent_jitter() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        r#"
default_volume = 80

[microphones."{0.0.1.00000000}.{mic-default}"]
friendly_name = "Blue Yeti"
policy = "follow_default"

[microphones."{0.0.1.00000000}.{mic-zero}"]
friendly_name = "Muted Floor Mic"
policy = "custom"
target_volume = 0

[microphones."{0.0.1.00000000}.{mic-max}"]
friendly_name = "Quiet Ribbon Mic"
policy = "custom"
target_volume = 100

[microphones."{0.0.1.00000000}.{mic-ignored}"]
friendly_name = "Virtual Cable"
policy = "ignored"
"#,
    )
    .unwrap();

    let backend = FakeAudioBackend::with_microphones(vec![
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-default}".to_string(),
            friendly_name: "Blue Yeti".to_string(),
            volume_scalar: 0.80,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-zero}".to_string(),
            friendly_name: "Muted Floor Mic".to_string(),
            volume_scalar: 0.0,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-max}".to_string(),
            friendly_name: "Quiet Ribbon Mic".to_string(),
            volume_scalar: 1.0,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-ignored}".to_string(),
            friendly_name: "Virtual Cable".to_string(),
            volume_scalar: 0.40,
        },
    ]);

    let mut controller = AppController::start(backend.clone(), config_path).unwrap();
    backend.clear_set_calls();

    // Sub-0.5% jitter around 80% should be ignored without calling set_volume_scalar.
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-default}".to_string(),
            volume_scalar: 0.804,
        })
        .unwrap();
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-default}".to_string(),
            volume_scalar: 0.796,
        })
        .unwrap();
    assert!(backend.set_calls().is_empty());

    // Meaningful external changes on locked Microphones revert immediately.
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-default}".to_string(),
            volume_scalar: 0.35,
        })
        .unwrap();
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-zero}".to_string(),
            volume_scalar: 0.25,
        })
        .unwrap();
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-max}".to_string(),
            volume_scalar: 0.90,
        })
        .unwrap();

    // Ignored Microphone is not reverted.
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-ignored}".to_string(),
            volume_scalar: 0.65,
        })
        .unwrap();

    assert_eq!(
        backend.set_calls(),
        vec![
            ("{0.0.1.00000000}.{mic-default}".to_string(), 0.80),
            ("{0.0.1.00000000}.{mic-zero}".to_string(), 0.0),
            ("{0.0.1.00000000}.{mic-max}".to_string(), 1.0),
        ]
    );

    let snapshot = controller.snapshot();
    let ignored_mic = snapshot
        .active_microphones
        .iter()
        .find(|m| m.endpoint_id == "{0.0.1.00000000}.{mic-ignored}")
        .unwrap();
    assert!((ignored_mic.current_volume_scalar - 0.65).abs() < 1e-6);
}

#[test]
fn newly_connected_microphone_is_auto_locked_to_default_volume_and_keyed_by_endpoint_id() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        r#"
default_volume = 75

[microphones."{0.0.1.00000000}.{mic-1}"]
friendly_name = "Blue Yeti"
policy = "custom"
target_volume = 50
"#,
    )
    .unwrap();

    let backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-1}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.50,
    }]);

    let mut controller = AppController::start(backend.clone(), config_path.clone()).unwrap();
    backend.clear_set_calls();

    // Plug in a second identical-model Microphone with a distinct Windows Audio Endpoint ID.
    let new_mic = ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-2}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.30,
    };
    backend.add_microphone(new_mic.clone());
    controller
        .handle_event(AudioEvent::MicrophoneConnected(new_mic))
        .unwrap();

    assert_eq!(
        backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-2}".to_string(), 0.75)]
    );

    let snapshot = controller.snapshot();
    assert_eq!(snapshot.active_microphones.len(), 2);

    let by_id = microphones_by_id(&snapshot);
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-1}"].lock_policy,
        LockPolicy::Custom(50)
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-2}"].lock_policy,
        LockPolicy::FollowDefault
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-2}"].effective_target_volume,
        Some(75)
    );

    let saved_toml = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_toml.contains("{0.0.1.00000000}.{mic-2}"));
    assert!(saved_toml.contains("policy = \"follow_default\""));
}

#[test]
fn friendly_name_update_refreshes_stored_name_without_altering_lock_policy() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
        friendly_name: "USB Audio Device".to_string(),
        volume_scalar: 0.80,
    }]);

    let mut controller = AppController::start(backend, config_path.clone()).unwrap();

    controller
        .handle_event(AudioEvent::FriendlyNameChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            friendly_name: "Desk Podcast Mic".to_string(),
        })
        .unwrap();

    let snapshot = controller.snapshot();
    assert_eq!(
        snapshot.active_microphones[0].friendly_name,
        "Desk Podcast Mic"
    );
    assert_eq!(
        snapshot.active_microphones[0].lock_policy,
        LockPolicy::FollowDefault
    );

    let saved_toml = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_toml.contains("Desk Podcast Mic"));
    assert!(!saved_toml.contains("USB Audio Device"));
}

#[test]
fn restart_reloads_persisted_first_run_config_and_starts_minimized() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let first_backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.40,
    }]);
    let first_controller = AppController::start(first_backend, config_path.clone()).unwrap();
    assert!(first_controller.should_open_window_on_startup());

    // Simulate restarting the app after external drift while closed.
    let restart_backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.25,
    }]);
    let restarted = AppController::start(restart_backend.clone(), config_path).unwrap();

    assert!(!restarted.should_open_window_on_startup());
    assert_eq!(restarted.snapshot().default_volume, 80);
    assert_eq!(
        restart_backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a}".to_string(), 0.80)]
    );
}

#[test]
fn corrupt_config_toml_returns_parse_error() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");
    std::fs::write(&config_path, "default_volume = [not valid toml").unwrap();

    let backend = FakeAudioBackend::default();
    let err = AppController::start(backend, config_path)
        .err()
        .expect("corrupt config should fail to load");

    assert!(matches!(
        err,
        ControllerError::Config(ConfigError::Parse { .. })
    ));
}

#[test]
fn disconnecting_microphone_removes_it_from_active_list_and_reconnecting_restores_lock_policy() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        r#"
default_volume = 80

[microphones."{0.0.1.00000000}.{mic-usb}"]
friendly_name = "Blue Yeti"
policy = "custom"
target_volume = 55
"#,
    )
    .unwrap();

    let backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-usb}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.55,
    }]);

    let mut controller = AppController::start(backend.clone(), config_path.clone()).unwrap();
    backend.clear_set_calls();

    // Unplug the Microphone.
    backend.remove_microphone("{0.0.1.00000000}.{mic-usb}");
    controller
        .handle_event(AudioEvent::MicrophoneDisconnected {
            endpoint_id: "{0.0.1.00000000}.{mic-usb}".to_string(),
        })
        .unwrap();

    assert!(controller.snapshot().active_microphones.is_empty());

    // Any stale volume event after disconnect is a no-op and does not error.
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-usb}".to_string(),
            volume_scalar: 0.20,
        })
        .unwrap();
    assert!(backend.set_calls().is_empty());

    // Replugging the same Endpoint ID immediately enforces its preserved custom Target Volume.
    let reconnected = ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-usb}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.90,
    };
    backend.add_microphone(reconnected.clone());
    controller
        .handle_event(AudioEvent::MicrophoneConnected(reconnected))
        .unwrap();

    assert_eq!(
        backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-usb}".to_string(), 0.55)]
    );
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.active_microphones.len(), 1);
    assert_eq!(
        snapshot.active_microphones[0].lock_policy,
        LockPolicy::Custom(55)
    );
    assert_eq!(
        snapshot.active_microphones[0].effective_target_volume,
        Some(55)
    );
}

#[test]
fn setting_default_volume_updates_follow_default_microphones_preserves_custom_and_ignored_and_persists_across_reload() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        r#"
default_volume = 80

[microphones."{0.0.1.00000000}.{mic-a}"]
friendly_name = "Blue Yeti"
policy = "follow_default"

[microphones."{0.0.1.00000000}.{mic-b}"]
friendly_name = "Headset Mic"
policy = "follow_default"

[microphones."{0.0.1.00000000}.{mic-custom}"]
friendly_name = "Shure SM7B"
policy = "custom"
target_volume = 55

[microphones."{0.0.1.00000000}.{mic-ignored}"]
friendly_name = "Virtual Cable"
policy = "ignored"
"#,
    )
    .unwrap();

    let backend = FakeAudioBackend::with_microphones(vec![
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            friendly_name: "Blue Yeti".to_string(),
            volume_scalar: 0.80,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-b}".to_string(),
            friendly_name: "Headset Mic".to_string(),
            volume_scalar: 0.80,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-custom}".to_string(),
            friendly_name: "Shure SM7B".to_string(),
            volume_scalar: 0.55,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-ignored}".to_string(),
            friendly_name: "Virtual Cable".to_string(),
            volume_scalar: 0.30,
        },
    ]);

    let mut controller = AppController::start(backend.clone(), config_path.clone()).unwrap();
    backend.clear_set_calls();

    controller
        .handle_command(AppCommand::SetDefaultVolume(65))
        .unwrap();

    assert_eq!(
        backend.set_calls(),
        vec![
            ("{0.0.1.00000000}.{mic-a}".to_string(), 0.65),
            ("{0.0.1.00000000}.{mic-b}".to_string(), 0.65),
        ]
    );
    assert!((backend.current_volume("{0.0.1.00000000}.{mic-a}").unwrap() - 0.65).abs() < 1e-6);
    assert!((backend.current_volume("{0.0.1.00000000}.{mic-b}").unwrap() - 0.65).abs() < 1e-6);
    assert!((backend.current_volume("{0.0.1.00000000}.{mic-custom}").unwrap() - 0.55).abs() < 1e-6);
    assert!((backend.current_volume("{0.0.1.00000000}.{mic-ignored}").unwrap() - 0.30).abs() < 1e-6);

    let snapshot = controller.snapshot();
    assert_eq!(snapshot.default_volume, 65);
    let by_id = microphones_by_id(&snapshot);
    assert_eq!(by_id["{0.0.1.00000000}.{mic-a}"].effective_target_volume, Some(65));
    assert!((by_id["{0.0.1.00000000}.{mic-a}"].current_volume_scalar - 0.65).abs() < 1e-6);
    assert_eq!(by_id["{0.0.1.00000000}.{mic-b}"].effective_target_volume, Some(65));
    assert!((by_id["{0.0.1.00000000}.{mic-b}"].current_volume_scalar - 0.65).abs() < 1e-6);
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-custom}"].lock_policy,
        LockPolicy::Custom(55)
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-custom}"].effective_target_volume,
        Some(55)
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-ignored}"].lock_policy,
        LockPolicy::Ignored
    );
    assert_eq!(by_id["{0.0.1.00000000}.{mic-ignored}"].effective_target_volume, None);

    // Reload from disk with drifted microphone volumes and verify 65% persists and is enforced.
    let reload_backend = FakeAudioBackend::with_microphones(vec![
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            friendly_name: "Blue Yeti".to_string(),
            volume_scalar: 0.20,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-b}".to_string(),
            friendly_name: "Headset Mic".to_string(),
            volume_scalar: 0.95,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-custom}".to_string(),
            friendly_name: "Shure SM7B".to_string(),
            volume_scalar: 0.55,
        },
    ]);
    let reloaded = AppController::start(reload_backend.clone(), config_path).unwrap();
    assert!(!reloaded.should_open_window_on_startup());
    assert_eq!(reloaded.snapshot().default_volume, 65);
    assert_eq!(
        reload_backend.set_calls(),
        vec![
            ("{0.0.1.00000000}.{mic-a}".to_string(), 0.65),
            ("{0.0.1.00000000}.{mic-b}".to_string(), 0.65),
        ]
    );
}

#[test]
fn setting_default_volume_clamps_above_100_and_is_noop_when_unchanged() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.80,
    }]);

    let mut controller = AppController::start(backend.clone(), config_path.clone()).unwrap();
    backend.clear_set_calls();

    // Setting the same Default Volume (80%) is a no-op.
    controller
        .handle_command(AppCommand::SetDefaultVolume(80))
        .unwrap();
    assert!(backend.set_calls().is_empty());

    // Values above 100 clamp to 100%.
    controller
        .handle_command(AppCommand::SetDefaultVolume(150))
        .unwrap();
    assert_eq!(controller.snapshot().default_volume, 100);
    assert_eq!(
        backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a}".to_string(), 1.0)]
    );

    // Repeating 100% is a no-op.
    backend.clear_set_calls();
    controller
        .handle_command(AppCommand::SetDefaultVolume(100))
        .unwrap();
    assert!(backend.set_calls().is_empty());

    // Setting 0% enforces 0.0 and persists 0.
    controller
        .handle_command(AppCommand::SetDefaultVolume(0))
        .unwrap();
    assert_eq!(controller.snapshot().default_volume, 0);
    assert_eq!(
        backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a}".to_string(), 0.0)]
    );
    let saved_toml = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_toml.contains("default_volume = 0"));
}
