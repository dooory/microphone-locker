//! Application Controller: core domain state and enforcement engine.

use std::collections::BTreeMap;
use std::path::PathBuf;

use thiserror::Error;

use crate::audio::{ActiveMicrophone, AudioBackend, AudioError, AudioEvent};
use crate::config::{AppConfig, ConfigError, LockPolicy, MicrophoneConfig, load_config, save_config};
use crate::startup::{NoopStartupRegistry, StartupError, StartupRegistry};

/// Sub-0.5% scalar tolerance to avoid revert loops from audio driver quantization.
pub const VOLUME_TOLERANCE: f32 = 0.005;

#[derive(Debug, Error)]
pub enum ControllerError {
    #[error(transparent)]
    Audio(#[from] AudioError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Startup(#[from] StartupError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MicrophoneSnapshot {
    pub endpoint_id: String,
    pub friendly_name: String,
    pub lock_policy: LockPolicy,
    pub effective_target_volume: Option<u8>,
    pub current_volume_scalar: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    SetDefaultVolume(u8),
    SetMicrophoneLockPolicy {
        endpoint_id: String,
        lock_policy: LockPolicy,
    },
    SetStartWithWindows(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppSnapshot {
    pub default_volume: u8,
    pub start_with_windows: bool,
    pub active_microphones: Vec<MicrophoneSnapshot>,
}

pub struct AppController<B: AudioBackend, S: StartupRegistry = NoopStartupRegistry> {
    backend: B,
    startup: S,
    config_path: PathBuf,
    config: AppConfig,
    active_microphones: BTreeMap<String, ActiveMicrophone>,
    open_window_on_startup: bool,
    start_with_windows: bool,
}

impl<B: AudioBackend> AppController<B, NoopStartupRegistry> {
    pub fn start(backend: B, config_path: PathBuf) -> Result<Self, ControllerError> {
        Self::start_with_startup(backend, NoopStartupRegistry, config_path)
    }
}

impl<B: AudioBackend, S: StartupRegistry> AppController<B, S> {
    pub fn start_with_startup(
        backend: B,
        startup: S,
        config_path: PathBuf,
    ) -> Result<Self, ControllerError> {
        let (config, open_window_on_startup) = match load_config(&config_path)? {
            Some(existing) => (existing, false),
            None => (AppConfig::initial_default(), true),
        };
        let start_with_windows = startup.is_enabled()?;

        let mut controller = Self {
            backend,
            startup,
            config_path,
            config,
            active_microphones: BTreeMap::new(),
            open_window_on_startup,
            start_with_windows,
        };

        let mut config_dirty = open_window_on_startup;
        if controller.reconcile_active_microphones()? {
            config_dirty = true;
        }

        if config_dirty {
            save_config(&controller.config_path, &controller.config)?;
        }

        Ok(controller)
    }

    pub fn should_open_window_on_startup(&self) -> bool {
        self.open_window_on_startup
    }

    pub fn handle_command(&mut self, command: AppCommand) -> Result<(), ControllerError> {
        match command {
            AppCommand::SetDefaultVolume(percent) => {
                let clamped = percent.min(100);
                if self.config.default_volume == clamped {
                    return Ok(());
                }
                self.config.default_volume = clamped;
                save_config(&self.config_path, &self.config)?;

                let follow_default_mics: Vec<(String, f32)> = self
                    .active_microphones
                    .values()
                    .filter(|mic| {
                        self.config
                            .microphones
                            .get(&mic.endpoint_id)
                            .is_some_and(|entry| entry.lock_policy == LockPolicy::FollowDefault)
                    })
                    .map(|mic| (mic.endpoint_id.clone(), mic.volume_scalar))
                    .collect();

                for (endpoint_id, current_scalar) in follow_default_mics {
                    if let Some(enforced_scalar) =
                        self.enforce_if_needed(&endpoint_id, current_scalar)?
                        && let Some(mic) = self.active_microphones.get_mut(&endpoint_id)
                    {
                        mic.volume_scalar = enforced_scalar;
                    }
                }
            }
            AppCommand::SetMicrophoneLockPolicy {
                endpoint_id,
                lock_policy,
            } => {
                let normalized_policy = match lock_policy {
                    LockPolicy::Custom(target) => LockPolicy::Custom(target.min(100)),
                    other => other,
                };

                let Some(entry) = self.config.microphones.get_mut(&endpoint_id) else {
                    return Ok(());
                };

                if entry.lock_policy == normalized_policy {
                    return Ok(());
                }

                entry.lock_policy = normalized_policy;
                save_config(&self.config_path, &self.config)?;

                if let Some(current_scalar) = self
                    .active_microphones
                    .get(&endpoint_id)
                    .map(|mic| mic.volume_scalar)
                    && let Some(enforced_scalar) =
                        self.enforce_if_needed(&endpoint_id, current_scalar)?
                    && let Some(mic) = self.active_microphones.get_mut(&endpoint_id)
                {
                    mic.volume_scalar = enforced_scalar;
                }
            }
            AppCommand::SetStartWithWindows(enabled) => {
                if self.start_with_windows == enabled {
                    return Ok(());
                }
                self.startup.set_enabled(enabled)?;
                self.start_with_windows = enabled;
            }
        }
        Ok(())
    }

    pub fn handle_event(&mut self, event: AudioEvent) -> Result<(), ControllerError> {
        match event {
            AudioEvent::ExternalVolumeChanged {
                endpoint_id,
                volume_scalar,
            } => {
                if !self.active_microphones.contains_key(&endpoint_id) {
                    return Ok(());
                }
                let effective_scalar =
                    match self.enforce_if_needed(&endpoint_id, volume_scalar)? {
                        Some(enforced) => enforced,
                        None => volume_scalar,
                    };
                if let Some(mic) = self.active_microphones.get_mut(&endpoint_id) {
                    mic.volume_scalar = effective_scalar;
                }
            }
            AudioEvent::MicrophoneConnected(mic) => {
                let config_dirty = self.activate_microphone(mic)?;
                if config_dirty {
                    save_config(&self.config_path, &self.config)?;
                }
            }
            AudioEvent::MicrophoneDisconnected { endpoint_id } => {
                self.active_microphones.remove(&endpoint_id);
            }
            AudioEvent::FriendlyNameChanged {
                endpoint_id,
                friendly_name,
            } => {
                let mut config_dirty = false;
                if let Some(entry) = self.config.microphones.get_mut(&endpoint_id)
                    && entry.friendly_name != friendly_name
                {
                    entry.friendly_name = friendly_name.clone();
                    config_dirty = true;
                }
                if let Some(mic) = self.active_microphones.get_mut(&endpoint_id) {
                    mic.friendly_name = friendly_name;
                }
                if config_dirty {
                    save_config(&self.config_path, &self.config)?;
                }
            }
            AudioEvent::SystemWake => {
                if self.reconcile_active_microphones()? {
                    save_config(&self.config_path, &self.config)?;
                }
            }
        }
        Ok(())
    }

    pub fn snapshot(&self) -> AppSnapshot {
        let active_microphones = self
            .active_microphones
            .values()
            .map(|mic| {
                let lock_policy = self
                    .config
                    .microphones
                    .get(&mic.endpoint_id)
                    .map(|entry| entry.lock_policy)
                    .unwrap_or(LockPolicy::FollowDefault);

                MicrophoneSnapshot {
                    endpoint_id: mic.endpoint_id.clone(),
                    friendly_name: mic.friendly_name.clone(),
                    lock_policy,
                    effective_target_volume: lock_policy
                        .resolve_target_volume(self.config.default_volume),
                    current_volume_scalar: mic.volume_scalar,
                }
            })
            .collect();

        AppSnapshot {
            default_volume: self.config.default_volume,
            start_with_windows: self.start_with_windows,
            active_microphones,
        }
    }

    fn reconcile_active_microphones(&mut self) -> Result<bool, AudioError> {
        let active = self.backend.list_active_microphones()?;
        self.active_microphones.clear();

        let mut config_dirty = false;
        for mic in active {
            if self.activate_microphone(mic)? {
                config_dirty = true;
            }
        }

        Ok(config_dirty)
    }

    fn activate_microphone(&mut self, mut mic: ActiveMicrophone) -> Result<bool, AudioError> {
        let config_dirty =
            self.register_or_refresh_microphone(&mic.endpoint_id, &mic.friendly_name);
        if let Some(enforced_scalar) =
            self.enforce_if_needed(&mic.endpoint_id, mic.volume_scalar)?
        {
            mic.volume_scalar = enforced_scalar;
        }
        self.active_microphones.insert(mic.endpoint_id.clone(), mic);
        Ok(config_dirty)
    }

    fn register_or_refresh_microphone(&mut self, endpoint_id: &str, friendly_name: &str) -> bool {
        match self.config.microphones.get_mut(endpoint_id) {
            Some(entry) => {
                if entry.friendly_name != friendly_name {
                    entry.friendly_name = friendly_name.to_string();
                    true
                } else {
                    false
                }
            }
            None => {
                self.config.microphones.insert(
                    endpoint_id.to_string(),
                    MicrophoneConfig {
                        friendly_name: friendly_name.to_string(),
                        lock_policy: LockPolicy::FollowDefault,
                    },
                );
                true
            }
        }
    }

    fn enforce_if_needed(
        &self,
        endpoint_id: &str,
        actual_scalar: f32,
    ) -> Result<Option<f32>, AudioError> {
        let Some(entry) = self.config.microphones.get(endpoint_id) else {
            return Ok(None);
        };
        let Some(target_percent) = entry
            .lock_policy
            .resolve_target_volume(self.config.default_volume)
        else {
            return Ok(None);
        };

        let target_scalar = volume_percent_to_scalar(target_percent);
        if (actual_scalar - target_scalar).abs() < VOLUME_TOLERANCE {
            return Ok(None);
        }

        self.backend.set_volume_scalar(endpoint_id, target_scalar)?;
        Ok(Some(target_scalar))
    }
}

pub fn volume_percent_to_scalar(percent: u8) -> f32 {
    f32::from(percent.min(100)) / 100.0
}
