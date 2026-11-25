//! Error type produced by the profile domain.

use hs_core::CoreError;
use thiserror::Error;

/// Convenient result alias used across this crate.
pub type ProfileResult<T> = Result<T, ProfileError>;

/// Errors returned by [`crate::ProfileService`].
#[derive(Debug, Error)]
pub enum ProfileError {
    /// The requested profile does not exist.
    #[error("profile not found")]
    NotFound,

    /// The request is structurally valid but violates a domain rule.
    #[error("validation failed: {0}")]
    Validation(String),

    /// Local persistence or encrypted secret storage failed.
    #[error("storage error: {0}")]
    Storage(String),
}

impl From<CoreError> for ProfileError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::NotFound(_) => Self::NotFound,
            CoreError::Validation { field, message } => {
                Self::Validation(format!("{field}: {message}"))
            }
            CoreError::Unauthorized(message) | CoreError::Forbidden(message) => {
                Self::Validation(message)
            }
            other => Self::Storage(other.to_string()),
        }
    }
}
