//! Error type shared by the network crate.

/// Convenient alias for fallible network operations.
pub type NetworkResult<T> = Result<T, NetworkError>;

/// Errors produced while resolving, rendering or applying network settings.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    /// The configuration is syntactically valid but cannot be applied safely
    /// (for example a custom DNS endpoint using plaintext HTTP).
    #[error("invalid network configuration: {0}")]
    Invalid(String),

    /// The operation is not supported on this platform.
    #[error("unsupported on this platform: {0}")]
    Unsupported(String),

    /// Rendering the artefact or running the platform tool failed.
    #[error("failed to apply network configuration: {0}")]
    Apply(String),

    /// Filesystem or process I/O failure.
    #[error("I/O error: {0}")]
    Io(String),
}

impl From<std::io::Error> for NetworkError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}
