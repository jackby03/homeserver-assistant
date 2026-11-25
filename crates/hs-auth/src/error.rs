//! Authentication and authorization errors.

use thiserror::Error;

/// Result alias for the auth crate.
pub type AuthResult<T> = Result<T, AuthError>;

/// Errors returned by authentication and authorization operations.
#[derive(Debug, Error)]
pub enum AuthError {
    /// Username or password are wrong. Deliberately does not say which.
    #[error("invalid username or password")]
    InvalidCredentials,

    /// The request has no valid session.
    #[error("authentication required: {0}")]
    Unauthorized(String),

    /// The authenticated principal is not allowed to perform the action.
    #[error("permission denied: {0}")]
    Forbidden(String),

    /// The requested user/session does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// The operation conflicts with the current state.
    #[error("conflict: {0}")]
    Conflict(String),

    /// A field failed validation.
    #[error("invalid request: {0}")]
    Validation(String),

    /// Too many failed login attempts.
    #[error("too many failed login attempts, try again later")]
    TooManyAttempts,

    /// Password hashing or verification failed unexpectedly.
    #[error("password hashing error: {0}")]
    Hashing(String),

    /// Token encoding/decoding failed.
    #[error("token error: {0}")]
    Token(String),

    /// Persistence failed.
    #[error("storage error: {0}")]
    Storage(String),

    /// Internal invariant violation.
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<hs_core::CoreError> for AuthError {
    fn from(err: hs_core::CoreError) -> Self {
        match err {
            hs_core::CoreError::Validation { field, message } => {
                Self::Validation(format!("{field}: {message}"))
            }
            hs_core::CoreError::NotFound(what) => Self::NotFound(what),
            hs_core::CoreError::Conflict(what) => Self::Conflict(what),
            other => Self::Storage(other.to_string()),
        }
    }
}

impl AuthError {
    /// Stable machine-readable error code for the HTTP layer.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidCredentials => "invalid_credentials",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Validation(_) => "invalid_request",
            Self::TooManyAttempts => "too_many_attempts",
            Self::Hashing(_) | Self::Token(_) | Self::Internal(_) => "internal_error",
            Self::Storage(_) => "storage_error",
        }
    }
}
