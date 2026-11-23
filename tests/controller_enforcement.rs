use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use microphone_locker::audio::{ActiveMicrophone, AudioBackend, AudioError, AudioEvent};
use microphone_locker::config::{ConfigError, LockPolicy};
use microphone_locker::controller::{
    AppCommand, AppController, AppSnapshot, ControllerError, DisconnectedMicrophoneSnapshot,
    MicrophoneSnapshot,
};
use microphone_locker::startup::{StartupError, StartupRegistry};
use tempfile::TempDir;

fn microphones_by_id(snapshot: &AppSnapshot) -> HashMap<&str, &MicrophoneSnapshot> {
    snapshot
        .active_microphones
        .iter()
        .map(|m| (m.endpoint_id.as_str(), m))
        .collect()
}

fn disconnected_by_id(snapshot: &AppSnapshot) -> HashMap<&str, &DisconnectedMicrophoneSnapshot> {
    snapshot
        .disconnected_microphones
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

    fn set_external_volume(&self, endpoint_id: &str, scalar: f32) {
        if let Some(mic) = self
            .state
            .lock()
            .unwrap()
            .microphones
            .iter_mut()
            .find(|m| m.endpoint_id == endpoint_id)
        {
            mic.volume_scalar = scalar;
        }
    }

    fn set_friendly_name(&self, endpoint_id: &str, friendly_name: &str) {
        if let Some(mic) = self
            .state
            .lock()
            .unwrap()
            .microphones
            .iter_mut()
            .find(|m| m.endpoint_id == endpoint_id)
        {
            mic.friendly_name = friendly_name.to_string();
        }
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
fn first_run_initializes_default_volume_to_80_saves_config_enforces_active_microphones_and_opens_window()
 {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("MicrophoneLocker").join("config.toml");

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
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-a}"].effective_target_volume,
        Some(80)
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-b}"].friendly_name,
        "Headset Mic"
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-b}"].effective_target_volume,
        Some(80)
    );

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
fn subsequent_run_loads_saved_config_reconciles_microphones_starts_minimized_and_refreshes_friendly_name()
 {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("MicrophoneLocker").join("config.toml");

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
    assert!(
        (backend
            .current_volume("{0.0.1.00000000}.{mic-follow}")
            .unwrap()
            - 0.70)
            .abs()
            < 1e-6
    );
    assert!(
        (backend
            .current_volume("{0.0.1.00000000}.{mic-custom}")
            .unwrap()
            - 0.55)
            .abs()
            < 1e-6
    );
    assert!(
        (backend
            .current_volume("{0.0.1.00000000}.{mic-ignored}")
            .unwrap()
            - 0.15)
            .abs()
            < 1e-6
    );

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
fn setting_default_volume_updates_follow_default_microphones_preserves_custom_and_ignored_and_persists_across_reload()
 {
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
    assert!(
        (backend
            .current_volume("{0.0.1.00000000}.{mic-custom}")
            .unwrap()
            - 0.55)
            .abs()
            < 1e-6
    );
    assert!(
        (backend
            .current_volume("{0.0.1.00000000}.{mic-ignored}")
            .unwrap()
            - 0.30)
            .abs()
            < 1e-6
    );

    let snapshot = controller.snapshot();
    assert_eq!(snapshot.default_volume, 65);
    let by_id = microphones_by_id(&snapshot);
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-a}"].effective_target_volume,
        Some(65)
    );
    assert!((by_id["{0.0.1.00000000}.{mic-a}"].current_volume_scalar - 0.65).abs() < 1e-6);
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-b}"].effective_target_volume,
        Some(65)
    );
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
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-ignored}"].effective_target_volume,
        None
    );

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

#[derive(Debug, Default)]
struct FakeStartupState {
    enabled: bool,
    set_calls: Vec<bool>,
    fail_next_set: bool,
}

#[derive(Debug, Clone, Default)]
struct FakeStartupRegistry {
    state: Arc<Mutex<FakeStartupState>>,
}

impl FakeStartupRegistry {
    fn new(initially_enabled: bool) -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeStartupState {
                enabled: initially_enabled,
                set_calls: Vec::new(),
                fail_next_set: false,
            })),
        }
    }

    fn set_calls(&self) -> Vec<bool> {
        self.state.lock().unwrap().set_calls.clone()
    }

    fn set_fail_next(&self, fail: bool) {
        self.state.lock().unwrap().fail_next_set = fail;
    }
}

impl StartupRegistry for FakeStartupRegistry {
    fn is_enabled(&self) -> Result<bool, StartupError> {
        Ok(self.state.lock().unwrap().enabled)
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), StartupError> {
        let mut state = self.state.lock().unwrap();
        state.set_calls.push(enabled);
        if state.fail_next_set {
            state.fail_next_set = false;
            return Err(StartupError::Registry(
                "simulated registry failure".to_string(),
            ));
        }
        state.enabled = enabled;
        Ok(())
    }
}

#[test]
fn start_with_windows_queries_enables_disables_and_preserves_state_on_registry_error() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let backend = FakeAudioBackend::default();
    let startup = FakeStartupRegistry::new(false);

    let mut controller =
        AppController::start_with_startup(backend, startup.clone(), config_path).unwrap();

    assert!(!controller.snapshot().start_with_windows);
    assert!(startup.set_calls().is_empty());

    // Enabling updates registry and snapshot.
    controller
        .handle_command(AppCommand::SetStartWithWindows(true))
        .unwrap();
    assert!(controller.snapshot().start_with_windows);
    assert_eq!(startup.set_calls(), vec![true]);

    // Redundant enable is a no-op.
    controller
        .handle_command(AppCommand::SetStartWithWindows(true))
        .unwrap();
    assert_eq!(startup.set_calls(), vec![true]);

    // Registry error leaves snapshot unchanged and surfaces ControllerError::Startup.
    startup.set_fail_next(true);
    let err = controller
        .handle_command(AppCommand::SetStartWithWindows(false))
        .unwrap_err();
    assert!(matches!(err, ControllerError::Startup(_)));
    assert!(controller.snapshot().start_with_windows);

    // Disabling succeeds when registry is healthy.
    controller
        .handle_command(AppCommand::SetStartWithWindows(false))
        .unwrap();
    assert!(!controller.snapshot().start_with_windows);
    assert_eq!(startup.set_calls(), vec![true, false, false]);
}

#[test]
fn start_with_windows_initial_state_reflects_existing_registry_entry() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let backend = FakeAudioBackend::default();
    let startup = FakeStartupRegistry::new(true);

    let controller = AppController::start_with_startup(backend, startup, config_path).unwrap();

    assert!(controller.snapshot().start_with_windows);
}

#[test]
fn system_wake_reconciles_all_active_locked_microphones_and_refreshes_device_roster() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        r#"
default_volume = 75

[microphones."{0.0.1.00000000}.{mic-follow}"]
friendly_name = "Blue Yeti"
policy = "follow_default"

[microphones."{0.0.1.00000000}.{mic-custom}"]
friendly_name = "Shure SM7B"
policy = "custom"
target_volume = 50

[microphones."{0.0.1.00000000}.{mic-ignored}"]
friendly_name = "Virtual Cable"
policy = "ignored"

[microphones."{0.0.1.00000000}.{mic-unplugged}"]
friendly_name = "Travel Headset"
policy = "custom"
target_volume = 60
"#,
    )
    .unwrap();

    let backend = FakeAudioBackend::with_microphones(vec![
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-follow}".to_string(),
            friendly_name: "Blue Yeti".to_string(),
            volume_scalar: 0.75,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-custom}".to_string(),
            friendly_name: "Shure SM7B".to_string(),
            volume_scalar: 0.50,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-ignored}".to_string(),
            friendly_name: "Virtual Cable".to_string(),
            volume_scalar: 0.30,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-unplugged}".to_string(),
            friendly_name: "Travel Headset".to_string(),
            volume_scalar: 0.60,
        },
    ]);

    let mut controller = AppController::start(backend.clone(), config_path.clone()).unwrap();
    backend.clear_set_calls();

    // Simulate changes occurring while Windows was asleep:
    // - locked Microphones drifted
    // - ignored Microphone changed
    // - one Microphone was unplugged
    // - a brand-new Microphone was plugged in
    // - an existing Microphone's friendly name changed
    backend.set_external_volume("{0.0.1.00000000}.{mic-follow}", 0.20);
    backend.set_friendly_name("{0.0.1.00000000}.{mic-follow}", "Blue Yeti X");
    backend.set_external_volume("{0.0.1.00000000}.{mic-custom}", 0.95);
    backend.set_external_volume("{0.0.1.00000000}.{mic-ignored}", 0.10);
    backend.remove_microphone("{0.0.1.00000000}.{mic-unplugged}");
    backend.add_microphone(ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-new}".to_string(),
        friendly_name: "Dock USB Mic".to_string(),
        volume_scalar: 0.40,
    });

    controller.handle_event(AudioEvent::SystemWake).unwrap();

    assert_eq!(
        backend.set_calls(),
        vec![
            ("{0.0.1.00000000}.{mic-follow}".to_string(), 0.75),
            ("{0.0.1.00000000}.{mic-custom}".to_string(), 0.50),
            ("{0.0.1.00000000}.{mic-new}".to_string(), 0.75),
        ]
    );
    assert!(
        (backend
            .current_volume("{0.0.1.00000000}.{mic-follow}")
            .unwrap()
            - 0.75)
            .abs()
            < 1e-6
    );
    assert!(
        (backend
            .current_volume("{0.0.1.00000000}.{mic-custom}")
            .unwrap()
            - 0.50)
            .abs()
            < 1e-6
    );
    assert!(
        (backend
            .current_volume("{0.0.1.00000000}.{mic-ignored}")
            .unwrap()
            - 0.10)
            .abs()
            < 1e-6
    );
    assert!(
        (backend
            .current_volume("{0.0.1.00000000}.{mic-new}")
            .unwrap()
            - 0.75)
            .abs()
            < 1e-6
    );

    let snapshot = controller.snapshot();
    assert_eq!(snapshot.active_microphones.len(), 4);
    let by_id = microphones_by_id(&snapshot);
    assert!(!by_id.contains_key("{0.0.1.00000000}.{mic-unplugged}"));
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-follow}"].friendly_name,
        "Blue Yeti X"
    );
    assert!((by_id["{0.0.1.00000000}.{mic-ignored}"].current_volume_scalar - 0.10).abs() < 1e-6);
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-new}"].lock_policy,
        LockPolicy::FollowDefault
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-new}"].effective_target_volume,
        Some(75)
    );

    let saved_toml = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_toml.contains("Blue Yeti X"));
    assert!(saved_toml.contains("Dock USB Mic"));
    assert!(saved_toml.contains("Travel Headset"));
}

#[test]
fn setting_custom_target_volume_enforces_immediately_persists_to_toml_and_is_unaffected_by_default_volume_changes()
 {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

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
    ]);

    let mut controller = AppController::start(backend.clone(), config_path.clone()).unwrap();
    backend.clear_set_calls();

    // Assign a custom Target Volume of 55% to mic-a.
    controller
        .handle_command(AppCommand::SetMicrophoneLockPolicy {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            lock_policy: LockPolicy::Custom(55),
        })
        .unwrap();

    assert_eq!(
        backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a}".to_string(), 0.55)]
    );
    assert!((backend.current_volume("{0.0.1.00000000}.{mic-a}").unwrap() - 0.55).abs() < 1e-6);

    let snapshot = controller.snapshot();
    let by_id = microphones_by_id(&snapshot);
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-a}"].lock_policy,
        LockPolicy::Custom(55)
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-a}"].effective_target_volume,
        Some(55)
    );
    assert!((by_id["{0.0.1.00000000}.{mic-a}"].current_volume_scalar - 0.55).abs() < 1e-6);

    // Changing Default Volume affects mic-b (FollowDefault) but leaves mic-a (Custom) untouched.
    backend.clear_set_calls();
    controller
        .handle_command(AppCommand::SetDefaultVolume(70))
        .unwrap();

    assert_eq!(
        backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-b}".to_string(), 0.70)]
    );
    assert!((backend.current_volume("{0.0.1.00000000}.{mic-a}").unwrap() - 0.55).abs() < 1e-6);

    // Values above 100% clamp to 100%.
    backend.clear_set_calls();
    controller
        .handle_command(AppCommand::SetMicrophoneLockPolicy {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            lock_policy: LockPolicy::Custom(150),
        })
        .unwrap();
    assert_eq!(
        backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a}".to_string(), 1.0)]
    );
    assert_eq!(
        microphones_by_id(&controller.snapshot())["{0.0.1.00000000}.{mic-a}"].lock_policy,
        LockPolicy::Custom(100)
    );

    // Set back to 55% and verify TOML round-trip persistence across restart.
    controller
        .handle_command(AppCommand::SetMicrophoneLockPolicy {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            lock_policy: LockPolicy::Custom(55),
        })
        .unwrap();

    let saved_toml = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_toml.contains("policy = \"custom\""));
    assert!(saved_toml.contains("target_volume = 55"));

    let reload_backend = FakeAudioBackend::with_microphones(vec![
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            friendly_name: "Blue Yeti".to_string(),
            volume_scalar: 0.90,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-b}".to_string(),
            friendly_name: "Headset Mic".to_string(),
            volume_scalar: 0.70,
        },
    ]);
    let reloaded = AppController::start(reload_backend.clone(), config_path).unwrap();
    assert_eq!(
        reload_backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a}".to_string(), 0.55)]
    );
    let reloaded_snapshot = reloaded.snapshot();
    let reloaded_by_id = microphones_by_id(&reloaded_snapshot);
    assert_eq!(
        reloaded_by_id["{0.0.1.00000000}.{mic-a}"].lock_policy,
        LockPolicy::Custom(55)
    );
    assert_eq!(
        reloaded_by_id["{0.0.1.00000000}.{mic-a}"].effective_target_volume,
        Some(55)
    );
}

#[test]
fn resetting_custom_microphone_to_default_enforces_current_default_volume_and_persists() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        r#"
default_volume = 70

[microphones."{0.0.1.00000000}.{mic-a}"]
friendly_name = "Shure SM7B"
policy = "custom"
target_volume = 40
"#,
    )
    .unwrap();

    let backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
        friendly_name: "Shure SM7B".to_string(),
        volume_scalar: 0.40,
    }]);

    let mut controller = AppController::start(backend.clone(), config_path.clone()).unwrap();
    backend.clear_set_calls();

    // Reset to Default immediately enforces current Default Volume (70%).
    controller
        .handle_command(AppCommand::SetMicrophoneLockPolicy {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            lock_policy: LockPolicy::FollowDefault,
        })
        .unwrap();

    assert_eq!(
        backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a}".to_string(), 0.70)]
    );
    assert!((backend.current_volume("{0.0.1.00000000}.{mic-a}").unwrap() - 0.70).abs() < 1e-6);

    let snapshot = controller.snapshot();
    assert_eq!(
        snapshot.active_microphones[0].lock_policy,
        LockPolicy::FollowDefault
    );
    assert_eq!(
        snapshot.active_microphones[0].effective_target_volume,
        Some(70)
    );
    assert!((snapshot.active_microphones[0].current_volume_scalar - 0.70).abs() < 1e-6);

    // Subsequent Default Volume changes now propagate to mic-a.
    backend.clear_set_calls();
    controller
        .handle_command(AppCommand::SetDefaultVolume(60))
        .unwrap();
    assert_eq!(
        backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a}".to_string(), 0.60)]
    );

    let saved_toml = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_toml.contains("policy = \"follow_default\""));
    assert!(!saved_toml.contains("target_volume"));

    // Verify TOML round-trip across restart.
    let reload_backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
        friendly_name: "Shure SM7B".to_string(),
        volume_scalar: 0.25,
    }]);
    let reloaded = AppController::start(reload_backend.clone(), config_path).unwrap();
    assert_eq!(
        reloaded.snapshot().active_microphones[0].lock_policy,
        LockPolicy::FollowDefault
    );
    assert_eq!(
        reload_backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a}".to_string(), 0.60)]
    );
}

#[test]
fn marking_microphone_as_ignored_stops_enforcement_persists_across_restart_and_unignoring_re_enforces()
 {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-virtual}".to_string(),
        friendly_name: "Virtual Cable".to_string(),
        volume_scalar: 0.80,
    }]);

    let mut controller = AppController::start(backend.clone(), config_path.clone()).unwrap();
    backend.clear_set_calls();

    // Mark as Ignored: does not touch volume.
    controller
        .handle_command(AppCommand::SetMicrophoneLockPolicy {
            endpoint_id: "{0.0.1.00000000}.{mic-virtual}".to_string(),
            lock_policy: LockPolicy::Ignored,
        })
        .unwrap();

    assert!(backend.set_calls().is_empty());
    let snapshot = controller.snapshot();
    assert_eq!(
        snapshot.active_microphones[0].lock_policy,
        LockPolicy::Ignored
    );
    assert_eq!(snapshot.active_microphones[0].effective_target_volume, None);

    // External volume changes are ignored while Ignored, and Default Volume changes leave it untouched.
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-virtual}".to_string(),
            volume_scalar: 0.35,
        })
        .unwrap();
    controller
        .handle_command(AppCommand::SetDefaultVolume(65))
        .unwrap();
    assert!(backend.set_calls().is_empty());
    assert!(
        (controller.snapshot().active_microphones[0].current_volume_scalar - 0.35).abs() < 1e-6
    );

    let saved_toml = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_toml.contains("policy = \"ignored\""));

    // Restart with volume at 0.35: Ignored persists and startup does not alter volume.
    let restart_backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-virtual}".to_string(),
        friendly_name: "Virtual Cable".to_string(),
        volume_scalar: 0.35,
    }]);
    let mut restarted = AppController::start(restart_backend.clone(), config_path.clone()).unwrap();
    assert!(restart_backend.set_calls().is_empty());
    assert_eq!(
        restarted.snapshot().active_microphones[0].lock_policy,
        LockPolicy::Ignored
    );

    // Un-ignore returns the Microphone to Locked (FollowDefault) and immediately enforces Default Volume (65%).
    restarted
        .handle_command(AppCommand::SetMicrophoneLockPolicy {
            endpoint_id: "{0.0.1.00000000}.{mic-virtual}".to_string(),
            lock_policy: LockPolicy::FollowDefault,
        })
        .unwrap();

    assert_eq!(
        restart_backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-virtual}".to_string(), 0.65)]
    );
    let unignored_snapshot = restarted.snapshot();
    assert_eq!(
        unignored_snapshot.active_microphones[0].lock_policy,
        LockPolicy::FollowDefault
    );
    assert_eq!(
        unignored_snapshot.active_microphones[0].effective_target_volume,
        Some(65)
    );
    assert!((unignored_snapshot.active_microphones[0].current_volume_scalar - 0.65).abs() < 1e-6);

    // External volume changes now revert again.
    restart_backend.clear_set_calls();
    restarted
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-virtual}".to_string(),
            volume_scalar: 0.20,
        })
        .unwrap();
    assert_eq!(
        restart_backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-virtual}".to_string(), 0.65)]
    );

    // Un-ignored state persists across another restart.
    let final_backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-virtual}".to_string(),
        friendly_name: "Virtual Cable".to_string(),
        volume_scalar: 0.10,
    }]);
    let final_controller = AppController::start(final_backend.clone(), config_path).unwrap();
    assert_eq!(
        final_controller.snapshot().active_microphones[0].lock_policy,
        LockPolicy::FollowDefault
    );
    assert_eq!(
        final_backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-virtual}".to_string(), 0.65)]
    );
}

#[test]
fn setting_lock_policy_for_unknown_or_unchanged_microphone_is_noop() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.80,
    }]);

    let mut controller = AppController::start(backend.clone(), config_path).unwrap();
    backend.clear_set_calls();

    // Unknown endpoint ID is a clean no-op.
    controller
        .handle_command(AppCommand::SetMicrophoneLockPolicy {
            endpoint_id: "{0.0.1.00000000}.{does-not-exist}".to_string(),
            lock_policy: LockPolicy::Custom(42),
        })
        .unwrap();
    assert!(backend.set_calls().is_empty());

    // Re-applying the existing LockPolicy is a no-op.
    controller
        .handle_command(AppCommand::SetMicrophoneLockPolicy {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            lock_policy: LockPolicy::FollowDefault,
        })
        .unwrap();
    assert!(backend.set_calls().is_empty());
}

#[test]
fn unignoring_custom_microphone_restores_previous_custom_target_volume_across_pause_and_restart() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.80,
    }]);

    let mut controller = AppController::start(backend.clone(), config_path.clone()).unwrap();
    // Set custom Target Volume to 45% (Default Volume is 80%).
    controller
        .handle_command(AppCommand::SetMicrophoneLockPolicy {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            lock_policy: LockPolicy::Custom(45),
        })
        .unwrap();
    backend.clear_set_calls();

    // Pause, then mark Ignored: remembers 45% in config.toml.
    controller
        .handle_command(AppCommand::SetPaused(true))
        .unwrap();
    controller
        .handle_command(AppCommand::SetMicrophoneIgnored {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            ignored: true,
        })
        .unwrap();
    assert!(backend.set_calls().is_empty());
    assert_eq!(
        controller.snapshot().active_microphones[0].lock_policy,
        LockPolicy::Ignored
    );
    let saved_ignored = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_ignored.contains("policy = \"ignored\""));
    assert!(saved_ignored.contains("previous_custom_target_volume = 45"));

    // Un-ignore while still Paused: restores Custom(45) without enforcing until resumed.
    controller
        .handle_command(AppCommand::SetMicrophoneIgnored {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            ignored: false,
        })
        .unwrap();
    assert!(backend.set_calls().is_empty());
    assert_eq!(
        controller.snapshot().active_microphones[0].lock_policy,
        LockPolicy::Custom(45)
    );

    // Drift volume while paused, then Resume: reconciles back to 45% (not Default Volume 80%).
    backend.set_external_volume("{0.0.1.00000000}.{mic-a}", 0.95);
    controller
        .handle_command(AppCommand::SetPaused(false))
        .unwrap();
    assert_eq!(
        backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a}".to_string(), 0.45)]
    );

    // Ignoring again and restarting preserves previous_custom_target_volume across restart.
    controller
        .handle_command(AppCommand::SetMicrophoneIgnored {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            ignored: true,
        })
        .unwrap();
    let restart_backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.90,
    }]);
    let mut restarted = AppController::start(restart_backend.clone(), config_path.clone()).unwrap();
    assert!(restart_backend.set_calls().is_empty());
    assert_eq!(
        restarted.snapshot().active_microphones[0].lock_policy,
        LockPolicy::Ignored
    );

    // Un-ignoring after restart restores Custom(45), enforces 45%, and clears previous_custom_target_volume.
    restarted
        .handle_command(AppCommand::SetMicrophoneIgnored {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            ignored: false,
        })
        .unwrap();
    assert_eq!(
        restart_backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a}".to_string(), 0.45)]
    );
    assert_eq!(
        restarted.snapshot().active_microphones[0].lock_policy,
        LockPolicy::Custom(45)
    );
    let saved_restored = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_restored.contains("policy = \"custom\""));
    assert!(saved_restored.contains("target_volume = 45"));
    assert!(!saved_restored.contains("previous_custom_target_volume"));
}

#[test]
fn pausing_suspends_enforcement_preserves_lock_policies_and_still_tracks_devices() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        r#"
default_volume = 80

[microphones."{0.0.1.00000000}.{mic-follow}"]
friendly_name = "Blue Yeti"
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
            friendly_name: "Blue Yeti".to_string(),
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

    let mut controller = AppController::start(backend.clone(), config_path).unwrap();
    backend.clear_set_calls();

    assert!(!controller.snapshot().paused);

    controller
        .handle_command(AppCommand::SetPaused(true))
        .unwrap();
    assert!(controller.snapshot().paused);

    // External volume changes on locked Microphones must not be reverted while Paused.
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-follow}".to_string(),
            volume_scalar: 0.25,
        })
        .unwrap();
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-custom}".to_string(),
            volume_scalar: 0.90,
        })
        .unwrap();

    // Newly connected Microphone is tracked & registered without enforcing volume while Paused.
    let new_mic = ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-new}".to_string(),
        friendly_name: "Headset Mic".to_string(),
        volume_scalar: 0.40,
    };
    backend.add_microphone(new_mic.clone());
    controller
        .handle_event(AudioEvent::MicrophoneConnected(new_mic))
        .unwrap();

    // Disconnecting a Microphone still updates the active list while Paused.
    backend.remove_microphone("{0.0.1.00000000}.{mic-ignored}");
    controller
        .handle_event(AudioEvent::MicrophoneDisconnected {
            endpoint_id: "{0.0.1.00000000}.{mic-ignored}".to_string(),
        })
        .unwrap();

    assert!(backend.set_calls().is_empty());

    let snapshot = controller.snapshot();
    assert!(snapshot.paused);
    assert_eq!(snapshot.active_microphones.len(), 3);

    let by_id = microphones_by_id(&snapshot);
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-follow}"].lock_policy,
        LockPolicy::FollowDefault
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-follow}"].effective_target_volume,
        Some(80)
    );
    assert!((by_id["{0.0.1.00000000}.{mic-follow}"].current_volume_scalar - 0.25).abs() < 1e-6);

    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-custom}"].lock_policy,
        LockPolicy::Custom(55)
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-custom}"].effective_target_volume,
        Some(55)
    );
    assert!((by_id["{0.0.1.00000000}.{mic-custom}"].current_volume_scalar - 0.90).abs() < 1e-6);

    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-new}"].lock_policy,
        LockPolicy::FollowDefault
    );
    assert!((by_id["{0.0.1.00000000}.{mic-new}"].current_volume_scalar - 0.40).abs() < 1e-6);
}

#[test]
fn resuming_from_paused_immediately_reconciles_active_locked_microphones_and_leaves_ignored_untouched()
 {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        r#"
default_volume = 80

[microphones."{0.0.1.00000000}.{mic-a-follow}"]
friendly_name = "Blue Yeti"
policy = "follow_default"

[microphones."{0.0.1.00000000}.{mic-b-custom}"]
friendly_name = "Shure SM7B"
policy = "custom"
target_volume = 55

[microphones."{0.0.1.00000000}.{mic-c-on-target}"]
friendly_name = "Desk Mic"
policy = "follow_default"

[microphones."{0.0.1.00000000}.{mic-d-ignored}"]
friendly_name = "Virtual Cable"
policy = "ignored"
"#,
    )
    .unwrap();

    let backend = FakeAudioBackend::with_microphones(vec![
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-a-follow}".to_string(),
            friendly_name: "Blue Yeti".to_string(),
            volume_scalar: 0.80,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-b-custom}".to_string(),
            friendly_name: "Shure SM7B".to_string(),
            volume_scalar: 0.55,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-c-on-target}".to_string(),
            friendly_name: "Desk Mic".to_string(),
            volume_scalar: 0.80,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-d-ignored}".to_string(),
            friendly_name: "Virtual Cable".to_string(),
            volume_scalar: 0.30,
        },
    ]);

    let mut controller = AppController::start(backend.clone(), config_path).unwrap();
    backend.clear_set_calls();

    controller
        .handle_command(AppCommand::SetPaused(true))
        .unwrap();

    // Drift volumes and connect a new Microphone while Paused.
    backend.set_external_volume("{0.0.1.00000000}.{mic-a-follow}", 0.20);
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-a-follow}".to_string(),
            volume_scalar: 0.20,
        })
        .unwrap();
    backend.set_external_volume("{0.0.1.00000000}.{mic-b-custom}", 0.95);
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-b-custom}".to_string(),
            volume_scalar: 0.95,
        })
        .unwrap();
    backend.set_external_volume("{0.0.1.00000000}.{mic-d-ignored}", 0.70);
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-d-ignored}".to_string(),
            volume_scalar: 0.70,
        })
        .unwrap();

    let new_mic = ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-e-new}".to_string(),
        friendly_name: "Headset Mic".to_string(),
        volume_scalar: 0.10,
    };
    backend.add_microphone(new_mic.clone());
    controller
        .handle_event(AudioEvent::MicrophoneConnected(new_mic))
        .unwrap();
    assert!(backend.set_calls().is_empty());

    // Resuming immediately reconciles all drifted active locked Microphones.
    controller
        .handle_command(AppCommand::SetPaused(false))
        .unwrap();

    assert!(!controller.snapshot().paused);
    assert_eq!(
        backend.set_calls(),
        vec![
            ("{0.0.1.00000000}.{mic-a-follow}".to_string(), 0.80),
            ("{0.0.1.00000000}.{mic-b-custom}".to_string(), 0.55),
            ("{0.0.1.00000000}.{mic-e-new}".to_string(), 0.80),
        ]
    );

    let snapshot = controller.snapshot();
    let by_id = microphones_by_id(&snapshot);
    assert!((by_id["{0.0.1.00000000}.{mic-a-follow}"].current_volume_scalar - 0.80).abs() < 1e-6);
    assert!((by_id["{0.0.1.00000000}.{mic-b-custom}"].current_volume_scalar - 0.55).abs() < 1e-6);
    assert!((by_id["{0.0.1.00000000}.{mic-d-ignored}"].current_volume_scalar - 0.70).abs() < 1e-6);
    assert!((by_id["{0.0.1.00000000}.{mic-e-new}"].current_volume_scalar - 0.80).abs() < 1e-6);

    // Subsequent external drift reverts immediately again.
    backend.clear_set_calls();
    controller
        .handle_event(AudioEvent::ExternalVolumeChanged {
            endpoint_id: "{0.0.1.00000000}.{mic-a-follow}".to_string(),
            volume_scalar: 0.35,
        })
        .unwrap();
    assert_eq!(
        backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a-follow}".to_string(), 0.80)]
    );
}

#[test]
fn paused_state_is_never_persisted_to_config_and_resets_to_active_on_restart() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.80,
    }]);

    let mut controller = AppController::start(backend.clone(), config_path.clone()).unwrap();
    backend.clear_set_calls();

    controller
        .handle_command(AppCommand::SetPaused(true))
        .unwrap();
    assert!(controller.snapshot().paused);

    // Updating Default Volume while Paused persists config without enforcing audio volume.
    controller
        .handle_command(AppCommand::SetDefaultVolume(65))
        .unwrap();
    assert!(backend.set_calls().is_empty());

    let saved_toml = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_toml.contains("default_volume = 65"));
    assert!(
        !saved_toml.contains("paused"),
        "Paused must never be persisted to config.toml"
    );

    // Restarting while previously Paused always resumes active enforcement and reconciles.
    let restart_backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.25,
    }]);
    let restarted = AppController::start(restart_backend.clone(), config_path).unwrap();

    assert!(!restarted.snapshot().paused);
    assert_eq!(restarted.snapshot().default_volume, 65);
    assert_eq!(
        restart_backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a}".to_string(), 0.65)]
    );
}

#[test]
fn setting_microphone_lock_policy_while_paused_persists_without_enforcing_until_resumed() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.80,
    }]);

    let mut controller = AppController::start(backend.clone(), config_path.clone()).unwrap();
    backend.clear_set_calls();

    controller
        .handle_command(AppCommand::SetPaused(true))
        .unwrap();

    // Updating LockPolicy while Paused persists to config without enforcing audio volume.
    controller
        .handle_command(AppCommand::SetMicrophoneLockPolicy {
            endpoint_id: "{0.0.1.00000000}.{mic-a}".to_string(),
            lock_policy: LockPolicy::Custom(45),
        })
        .unwrap();
    assert!(backend.set_calls().is_empty());
    assert_eq!(
        controller.snapshot().active_microphones[0].lock_policy,
        LockPolicy::Custom(45)
    );
    let saved_toml = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_toml.contains("policy = \"custom\""));
    assert!(saved_toml.contains("target_volume = 45"));

    // Resuming immediately enforces the new Custom Target Volume.
    controller
        .handle_command(AppCommand::SetPaused(false))
        .unwrap();
    assert_eq!(
        backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-a}".to_string(), 0.45)]
    );
}

#[test]
fn resuming_from_paused_reconciles_silent_volume_drift_without_volume_changed_event() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        r#"
default_volume = 80

[microphones."{0.0.1.00000000}.{mic-follow}"]
friendly_name = "Blue Yeti"
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
            friendly_name: "Blue Yeti".to_string(),
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

    let mut controller = AppController::start(backend.clone(), config_path).unwrap();
    backend.clear_set_calls();

    controller
        .handle_command(AppCommand::SetPaused(true))
        .unwrap();

    // Simulate external volume drift while Paused with NO AudioEvent::ExternalVolumeChanged delivered.
    backend.set_external_volume("{0.0.1.00000000}.{mic-follow}", 0.15);
    backend.set_external_volume("{0.0.1.00000000}.{mic-custom}", 0.95);
    backend.set_external_volume("{0.0.1.00000000}.{mic-ignored}", 0.65);

    // Resuming must re-query the backend and reconcile all drifted locked Microphones.
    controller
        .handle_command(AppCommand::SetPaused(false))
        .unwrap();

    assert!(!controller.snapshot().paused);
    assert_eq!(
        backend.set_calls(),
        vec![
            ("{0.0.1.00000000}.{mic-follow}".to_string(), 0.80),
            ("{0.0.1.00000000}.{mic-custom}".to_string(), 0.55),
        ]
    );

    let snapshot = controller.snapshot();
    let by_id = microphones_by_id(&snapshot);
    assert!((by_id["{0.0.1.00000000}.{mic-follow}"].current_volume_scalar - 0.80).abs() < 1e-6);
    assert!((by_id["{0.0.1.00000000}.{mic-custom}"].current_volume_scalar - 0.55).abs() < 1e-6);
    assert!((by_id["{0.0.1.00000000}.{mic-ignored}"].current_volume_scalar - 0.65).abs() < 1e-6);
}

#[test]
fn unplugged_microphones_move_to_disconnected_preserving_lock_policy_and_replug_restores_custom_and_ignored()
 {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        r#"
default_volume = 75

[microphones."{0.0.1.00000000}.{mic-custom}"]
friendly_name = "Blue Yeti"
policy = "custom"
target_volume = 55

[microphones."{0.0.1.00000000}.{mic-ignored}"]
friendly_name = "Virtual Cable"
policy = "ignored"

[microphones."{0.0.1.00000000}.{mic-already-offline}"]
friendly_name = "Travel Headset"
policy = "follow_default"
"#,
    )
    .unwrap();

    let backend = FakeAudioBackend::with_microphones(vec![
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-custom}".to_string(),
            friendly_name: "Blue Yeti".to_string(),
            volume_scalar: 0.55,
        },
        ActiveMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-ignored}".to_string(),
            friendly_name: "Virtual Cable".to_string(),
            volume_scalar: 0.30,
        },
    ]);

    let mut controller = AppController::start(backend.clone(), config_path).unwrap();
    backend.clear_set_calls();

    // Microphone saved in config but not plugged in at startup is listed as Disconnected.
    let initial = controller.snapshot();
    assert_eq!(initial.active_microphones.len(), 2);
    assert_eq!(initial.disconnected_microphones.len(), 1);
    let initial_disc = disconnected_by_id(&initial);
    assert_eq!(
        initial_disc["{0.0.1.00000000}.{mic-already-offline}"].friendly_name,
        "Travel Headset"
    );
    assert_eq!(
        initial_disc["{0.0.1.00000000}.{mic-already-offline}"].lock_policy,
        LockPolicy::FollowDefault
    );
    assert_eq!(
        initial_disc["{0.0.1.00000000}.{mic-already-offline}"].effective_target_volume,
        Some(75)
    );

    // Unplug both active Microphones.
    backend.remove_microphone("{0.0.1.00000000}.{mic-custom}");
    controller
        .handle_event(AudioEvent::MicrophoneDisconnected {
            endpoint_id: "{0.0.1.00000000}.{mic-custom}".to_string(),
        })
        .unwrap();

    backend.remove_microphone("{0.0.1.00000000}.{mic-ignored}");
    controller
        .handle_event(AudioEvent::MicrophoneDisconnected {
            endpoint_id: "{0.0.1.00000000}.{mic-ignored}".to_string(),
        })
        .unwrap();

    let after_disconnect = controller.snapshot();
    assert!(after_disconnect.active_microphones.is_empty());
    assert_eq!(after_disconnect.disconnected_microphones.len(), 3);

    let disc = disconnected_by_id(&after_disconnect);
    assert_eq!(
        disc["{0.0.1.00000000}.{mic-custom}"].friendly_name,
        "Blue Yeti"
    );
    assert_eq!(
        disc["{0.0.1.00000000}.{mic-custom}"].lock_policy,
        LockPolicy::Custom(55)
    );
    assert_eq!(
        disc["{0.0.1.00000000}.{mic-custom}"].effective_target_volume,
        Some(55)
    );
    assert_eq!(
        disc["{0.0.1.00000000}.{mic-ignored}"].friendly_name,
        "Virtual Cable"
    );
    assert_eq!(
        disc["{0.0.1.00000000}.{mic-ignored}"].lock_policy,
        LockPolicy::Ignored
    );
    assert_eq!(
        disc["{0.0.1.00000000}.{mic-ignored}"].effective_target_volume,
        None
    );

    // Replug Custom Microphone with drifted volume -> moves back to Active and immediately enforces 55%.
    let custom_replugged = ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-custom}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.90,
    };
    backend.add_microphone(custom_replugged.clone());
    controller
        .handle_event(AudioEvent::MicrophoneConnected(custom_replugged))
        .unwrap();

    assert_eq!(
        backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-custom}".to_string(), 0.55)]
    );

    // Replug Ignored Microphone -> moves back to Active, keeps Ignored, does NOT alter volume.
    backend.clear_set_calls();
    let ignored_replugged = ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-ignored}".to_string(),
        friendly_name: "Virtual Cable".to_string(),
        volume_scalar: 0.25,
    };
    backend.add_microphone(ignored_replugged.clone());
    controller
        .handle_event(AudioEvent::MicrophoneConnected(ignored_replugged))
        .unwrap();

    assert!(backend.set_calls().is_empty());

    let after_replug = controller.snapshot();
    assert_eq!(after_replug.active_microphones.len(), 2);
    assert_eq!(after_replug.disconnected_microphones.len(), 1);
    assert_eq!(
        after_replug.disconnected_microphones[0].endpoint_id,
        "{0.0.1.00000000}.{mic-already-offline}"
    );

    let active_by_id = microphones_by_id(&after_replug);
    assert_eq!(
        active_by_id["{0.0.1.00000000}.{mic-custom}"].lock_policy,
        LockPolicy::Custom(55)
    );
    assert_eq!(
        active_by_id["{0.0.1.00000000}.{mic-custom}"].effective_target_volume,
        Some(55)
    );
    assert_eq!(
        active_by_id["{0.0.1.00000000}.{mic-ignored}"].lock_policy,
        LockPolicy::Ignored
    );
    assert_eq!(
        active_by_id["{0.0.1.00000000}.{mic-ignored}"].effective_target_volume,
        None
    );
    assert!(
        (active_by_id["{0.0.1.00000000}.{mic-ignored}"].current_volume_scalar - 0.25).abs() < 1e-6
    );
}

#[test]
fn forgetting_disconnected_microphone_deletes_it_from_state_and_config_so_replug_follows_default_volume()
 {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        r#"
default_volume = 70

[microphones."{0.0.1.00000000}.{mic-active}"]
friendly_name = "Desk Mic"
policy = "custom"
target_volume = 60

[microphones."{0.0.1.00000000}.{mic-old-custom}"]
friendly_name = "Old USB Mic"
policy = "custom"
target_volume = 35

[microphones."{0.0.1.00000000}.{mic-old-ignored}"]
friendly_name = "Old Virtual Cable"
policy = "ignored"
"#,
    )
    .unwrap();

    let backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-active}".to_string(),
        friendly_name: "Desk Mic".to_string(),
        volume_scalar: 0.60,
    }]);

    let mut controller = AppController::start(backend.clone(), config_path.clone()).unwrap();
    backend.clear_set_calls();

    // Forget the disconnected custom Microphone.
    controller
        .handle_command(AppCommand::ForgetMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-old-custom}".to_string(),
        })
        .unwrap();

    // Removed from disconnected snapshot, other disconnected entry preserved.
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.disconnected_microphones.len(), 1);
    assert_eq!(
        snapshot.disconnected_microphones[0].endpoint_id,
        "{0.0.1.00000000}.{mic-old-ignored}"
    );

    // Permanently removed from config.toml on disk.
    let saved_toml = std::fs::read_to_string(&config_path).unwrap();
    assert!(!saved_toml.contains("{0.0.1.00000000}.{mic-old-custom}"));
    assert!(!saved_toml.contains("Old USB Mic"));
    assert!(saved_toml.contains("{0.0.1.00000000}.{mic-old-ignored}"));
    assert!(saved_toml.contains("{0.0.1.00000000}.{mic-active}"));

    // Reloading from disk confirms the forgotten Microphone stays gone.
    let reload_backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-active}".to_string(),
        friendly_name: "Desk Mic".to_string(),
        volume_scalar: 0.60,
    }]);
    let mut reloaded = AppController::start(reload_backend.clone(), config_path.clone()).unwrap();
    assert_eq!(reloaded.snapshot().disconnected_microphones.len(), 1);
    assert_eq!(
        reloaded.snapshot().disconnected_microphones[0].endpoint_id,
        "{0.0.1.00000000}.{mic-old-ignored}"
    );

    // Replugging the forgotten Microphone later treats it as newly discovered (FollowDefault -> 70%).
    let replugged = ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-old-custom}".to_string(),
        friendly_name: "Old USB Mic".to_string(),
        volume_scalar: 0.20,
    };
    reload_backend.add_microphone(replugged.clone());
    reloaded
        .handle_event(AudioEvent::MicrophoneConnected(replugged))
        .unwrap();

    assert_eq!(
        reload_backend.set_calls(),
        vec![("{0.0.1.00000000}.{mic-old-custom}".to_string(), 0.70)]
    );
    let after_replug = reloaded.snapshot();
    let by_id = microphones_by_id(&after_replug);
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-old-custom}"].lock_policy,
        LockPolicy::FollowDefault
    );
    assert_eq!(
        by_id["{0.0.1.00000000}.{mic-old-custom}"].effective_target_volume,
        Some(70)
    );
}

#[test]
fn forgetting_active_or_unknown_microphone_is_noop() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        r#"
default_volume = 80

[microphones."{0.0.1.00000000}.{mic-active}"]
friendly_name = "Blue Yeti"
policy = "custom"
target_volume = 50
"#,
    )
    .unwrap();

    let backend = FakeAudioBackend::with_microphones(vec![ActiveMicrophone {
        endpoint_id: "{0.0.1.00000000}.{mic-active}".to_string(),
        friendly_name: "Blue Yeti".to_string(),
        volume_scalar: 0.50,
    }]);

    let mut controller = AppController::start(backend, config_path.clone()).unwrap();

    // Attempting to forget an active Microphone does not remove its saved Lock Policy.
    controller
        .handle_command(AppCommand::ForgetMicrophone {
            endpoint_id: "{0.0.1.00000000}.{mic-active}".to_string(),
        })
        .unwrap();

    // Attempting to forget an unknown Endpoint ID is a clean no-op.
    controller
        .handle_command(AppCommand::ForgetMicrophone {
            endpoint_id: "{0.0.1.00000000}.{does-not-exist}".to_string(),
        })
        .unwrap();

    let snapshot = controller.snapshot();
    assert_eq!(snapshot.active_microphones.len(), 1);
    assert_eq!(
        snapshot.active_microphones[0].lock_policy,
        LockPolicy::Custom(50)
    );
    let saved_toml = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved_toml.contains("{0.0.1.00000000}.{mic-active}"));
    assert!(saved_toml.contains("target_volume = 50"));
}
