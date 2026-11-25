//! Error type shared by the Home Assistant crate.

/// Convenient alias for fallible Home Assistant operations.
pub type HaResult<T> = Result<T, HaError>;

/// Errors produced by the Home Assistant client, service and provisioner.
#[derive(Debug, thiserror::Error)]
pub enum HaError {
    /// The Home Assistant instance is not configured, unreachable or not
    /// running yet.
    #[error("Home Assistant is unavailable: {0}")]
    Unavailable(String),

    /// Home Assistant rejected the long-lived access token.
    #[error("Home Assistant rejected the credentials (401 Unauthorized)")]
    Unauthorized,

    /// Home Assistant returned an unexpected HTTP status or rejected the
    /// request.
    #[error("Home Assistant HTTP error: {0}")]
    Http(String),

    /// The response body could not be parsed into the expected shape.
    #[error("failed to parse Home Assistant response: {0}")]
    Parse(String),
}
