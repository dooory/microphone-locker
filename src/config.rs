//! TOML configuration persistence for Microphone Locker.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const INITIAL_DEFAULT_VOLUME: u8 = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "policy", content = "target_volume", rename_all = "snake_case")]
pub enum LockPolicy {
    FollowDefault,
    Custom(u8),
    Ignored,
}

impl LockPolicy {
    pub fn resolve_target_volume(self, default_volume: u8) -> Option<u8> {
        match self {
            LockPolicy::FollowDefault => Some(default_volume.min(100)),
            LockPolicy::Custom(target) => Some(target.min(100)),
            LockPolicy::Ignored => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicrophoneConfig {
    pub friendly_name: String,
    #[serde(flatten)]
    pub lock_policy: LockPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_custom_target_volume: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub default_volume: u8,
    #[serde(default)]
    pub microphones: BTreeMap<String, MicrophoneConfig>,
}

impl AppConfig {
    pub fn initial_default() -> Self {
        Self {
            default_volume: INITIAL_DEFAULT_VOLUME,
            microphones: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("APPDATA environment variable is not set")]
    MissingAppDataDir,
    #[error("I/O error accessing config at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config TOML at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("failed to serialize config TOML: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Returns `%APPDATA%\MicrophoneLocker\config.toml`.
pub fn default_config_path() -> Result<PathBuf, ConfigError> {
    let app_data = std::env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .ok_or(ConfigError::MissingAppDataDir)?;
    Ok(PathBuf::from(app_data)
        .join("MicrophoneLocker")
        .join("config.toml"))
}

pub fn load_config(path: &Path) -> Result<Option<AppConfig>, ConfigError> {
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut config: AppConfig = toml::from_str(&contents).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source: Box::new(source),
    })?;
    config.default_volume = config.default_volume.min(100);
    for entry in config.microphones.values_mut() {
        if let LockPolicy::Custom(target) = entry.lock_policy {
            entry.lock_policy = LockPolicy::Custom(target.min(100));
        }
        if let Some(target) = entry.previous_custom_target_volume {
            entry.previous_custom_target_volume = Some(target.min(100));
        }
    }
    Ok(Some(config))
}

pub fn save_config(path: &Path, config: &AppConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let serialized = toml::to_string_pretty(config)?;
    fs::write(path, serialized).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}
