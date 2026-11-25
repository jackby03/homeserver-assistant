//! HTTP API error contract shared with the TypeScript client.
//!
//! The daemon implements `IntoResponse` for [`ApiError`]; domain crates only use
//! [`CoreError`]. Routes convert domain errors with `impl From<CoreError>`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::CoreError;

/// JSON envelope returned for every non-2xx response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    /// Error payload.
    pub error: ApiErrorBody,
}

/// Machine and human readable error description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    /// Stable error code (`invalid_request`, `not_found`, ...).
    pub code: String,
    /// Human readable English message.
    pub message: String,
    /// Optional structured details (never secrets).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ApiError {
    /// Create an error with a code and message.
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: ApiErrorBody {
                code: code.into(),
                message: message.into(),
                details: None,
            },
        }
    }

    /// Attach structured details.
    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.error.details = Some(details);
        self
    }

    /// Validate the body shape used by the API tests.
    #[must_use]
    pub fn is_error(&self) -> bool {
        !self.error.code.is_empty() && !self.error.message.is_empty()
    }
}

impl From<CoreError> for ApiError {
    fn from(err: CoreError) -> Self {
        Self::new(err.code(), err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_errors_map_to_stable_codes() {
        let error: ApiError = CoreError::not_found("instance `foo`").into();
        assert_eq!(error.error.code, "not_found");
        assert!(error.error.message.contains("foo"));
        assert!(error.is_error());
    }
}
