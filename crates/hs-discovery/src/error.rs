//! Error type shared by the discovery crate.

use std::time::Duration;

/// Convenient alias for fallible discovery operations.
pub type DiscoveryResult<T> = Result<T, DiscoveryError>;

/// Errors produced by a discovery protocol adapter.
#[derive(Debug, Clone, thiserror::Error)]
pub enum DiscoveryError {
    /// The protocol socket, daemon or platform helper could not run.
    #[error("discovery transport error: {0}")]
    Transport(String),

    /// The discovery round exceeded its deadline.
    #[error("discovery timed out after {0:?}")]
    Timeout(Duration),

    /// A discovery response could not be understood.
    #[error("failed to parse discovery response: {0}")]
    Parse(String),

    /// Filesystem or process I/O failure.
    #[error("I/O error: {0}")]
    Io(String),
}

impl From<std::io::Error> for DiscoveryError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}
