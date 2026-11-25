//! Error types shared by every crate in the workspace.
//!
//! Every crate exposes a single `thiserror` enum. Errors are converted into an
//! HTTP-safe representation only at the API boundary (`hs_core::api`).

use thiserror::Error;

/// Convenient alias used across the workspace.
pub type CoreResult<T> = Result<T, CoreError>;

/// Errors produced by the core domain layer.
#[derive(Debug, Error)]
pub enum CoreError {
    /// The configuration is syntactically valid JSON but semantically invalid.
    #[error("invalid configuration: {0}")]
    Config(String),

    /// A single field failed validation.
    #[error("validation failed for `{field}`: {message}")]
    Validation {
        /// Field name (snake_case, as exposed by the API).
        field: &'static str,
        /// Human readable explanation.
        message: String,
    },

    /// Local persistence failed (redb, filesystem, JSON payloads).
    #[error("storage error: {0}")]
    Storage(String),

    /// Serialization or deserialization failure.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Cryptographic failure (encryption, key loading, encoding).
    #[error("cryptography error: {0}")]
    Crypto(String),

    /// Filesystem or network I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The requested entity does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// The requested operation conflicts with the current state.
    #[error("conflict: {0}")]
    Conflict(String),

    /// Authentication is missing or invalid.
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// The authenticated principal lacks permission for the operation.
    #[error("forbidden: {0}")]
    Forbidden(String),

    /// An invariant was violated; this is a bug, not user error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl CoreError {
    /// Build a validation error for a specific field.
    #[must_use]
    pub fn validation(field: &'static str, message: impl Into<String>) -> Self {
        Self::Validation {
            field,
            message: message.into(),
        }
    }

    /// Build a not-found error.
    #[must_use]
    pub fn not_found(what: impl Into<String>) -> Self {
        Self::NotFound(what.into())
    }

    /// Stable machine-readable error code used by the HTTP API.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation { .. } | Self::Config(_) => "invalid_request",
            Self::Storage(_) => "storage_error",
            Self::Serialization(_) => "serialization_error",
            Self::Crypto(_) => "crypto_error",
            Self::Io(_) => "io_error",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::Internal(_) => "internal_error",
        }
    }
}

impl From<redb::Error> for CoreError {
    fn from(err: redb::Error) -> Self {
        Self::Storage(err.to_string())
    }
}

impl From<redb::DatabaseError> for CoreError {
    fn from(err: redb::DatabaseError) -> Self {
        Self::Storage(err.to_string())
    }
}

impl From<redb::TransactionError> for CoreError {
    fn from(err: redb::TransactionError) -> Self {
        Self::Storage(err.to_string())
    }
}

impl From<redb::TableError> for CoreError {
    fn from(err: redb::TableError) -> Self {
        Self::Storage(err.to_string())
    }
}

impl From<redb::StorageError> for CoreError {
    fn from(err: redb::StorageError) -> Self {
        Self::Storage(err.to_string())
    }
}

impl From<redb::CommitError> for CoreError {
    fn from(err: redb::CommitError) -> Self {
        Self::Storage(err.to_string())
    }
}

impl From<redb::CompactionError> for CoreError {
    fn from(err: redb::CompactionError) -> Self {
        Self::Storage(err.to_string())
    }
}
