//! Error type produced by the agent service and its LLM adapters.

use hs_core::CoreError;
use hs_profiles::ProfileError;
use hs_tools::ToolError;
use thiserror::Error;

/// Convenient result alias used across this crate.
pub type AgentResult<T> = Result<T, AgentError>;

/// Errors returned by provider management and the agent chat loop.
#[derive(Debug, Error)]
pub enum AgentError {
    /// No enabled LLM provider is configured.
    #[error("no LLM provider is configured")]
    NotConfigured,

    /// The provider failed (network error, malformed response, limit reached).
    #[error("LLM provider error: {0}")]
    Provider(String),

    /// The provider rejected the credentials or the request is unauthorized.
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// The request sent by the caller is invalid.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// A tool invocation failed.
    #[error("tool error: {0}")]
    Tool(String),

    /// Local persistence or encrypted secret storage failed.
    #[error("storage error: {0}")]
    Storage(String),
}

impl From<CoreError> for AgentError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::NotFound(_) => {
                Self::InvalidRequest("the requested record does not exist".to_owned())
            }
            CoreError::Validation { field, message } => {
                Self::InvalidRequest(format!("{field}: {message}"))
            }
            CoreError::Unauthorized(message) | CoreError::Forbidden(message) => {
                Self::Unauthorized(message)
            }
            other => Self::Storage(other.to_string()),
        }
    }
}

impl From<ProfileError> for AgentError {
    fn from(error: ProfileError) -> Self {
        match error {
            ProfileError::NotFound => Self::InvalidRequest("profile not found".to_owned()),
            ProfileError::Validation(message) => Self::InvalidRequest(message),
            ProfileError::Storage(message) => Self::Storage(message),
        }
    }
}

impl From<ToolError> for AgentError {
    fn from(error: ToolError) -> Self {
        Self::Tool(error.to_string())
    }
}
