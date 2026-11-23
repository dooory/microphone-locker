//! Startup registration trait and domain types for auto-starting with Windows.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("startup registry error: {0}")]
    Registry(String),
}

/// Narrow OS boundary for querying and updating per-user startup registration.
pub trait StartupRegistry {
    fn is_enabled(&self) -> Result<bool, StartupError>;
    fn set_enabled(&self, enabled: bool) -> Result<(), StartupError>;
}

/// Default no-op [`StartupRegistry`] used when startup registration is not wired.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopStartupRegistry;

impl StartupRegistry for NoopStartupRegistry {
    fn is_enabled(&self) -> Result<bool, StartupError> {
        Ok(false)
    }

    fn set_enabled(&self, _enabled: bool) -> Result<(), StartupError> {
        Ok(())
    }
}
