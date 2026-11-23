//! Audio backend trait and domain types for Windows Core Audio abstraction.

use thiserror::Error;

/// Snapshot of an active Windows audio capture endpoint reported by the backend.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveMicrophone {
    pub endpoint_id: String,
    pub friendly_name: String,
    pub volume_scalar: f32,
}

/// Asynchronous events emitted by the audio backend.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    ExternalVolumeChanged {
        endpoint_id: String,
        volume_scalar: f32,
    },
    MicrophoneConnected(ActiveMicrophone),
    MicrophoneDisconnected {
        endpoint_id: String,
    },
    FriendlyNameChanged {
        endpoint_id: String,
        friendly_name: String,
    },
    SystemWake,
}

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("microphone endpoint not found: {0}")]
    MicrophoneNotFound(String),
    #[error("audio backend error: {0}")]
    Backend(String),
}

/// Narrow OS boundary for interacting with Windows Core Audio capture endpoints.
pub trait AudioBackend {
    fn list_active_microphones(&self) -> Result<Vec<ActiveMicrophone>, AudioError>;
    fn set_volume_scalar(&self, endpoint_id: &str, scalar: f32) -> Result<(), AudioError>;
}
