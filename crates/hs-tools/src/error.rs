//! Error type produced by tool invocation and the workflow engine.

use thiserror::Error;

/// Convenient result alias used across this crate.
pub type ToolResult<T> = Result<T, ToolError>;

/// Errors returned while looking up or executing a tool.
#[derive(Debug, Error)]
pub enum ToolError {
    /// The active profile is not allowed to use the requested tool.
    #[error("profile is not allowed to use tool `{tool}`")]
    Forbidden {
        /// Name of the denied tool.
        tool: String,
    },

    /// No tool is registered under the requested name.
    #[error("unknown tool `{0}`")]
    NotFound(String),

    /// The JSON arguments do not satisfy the tool contract.
    #[error("invalid tool arguments: {0}")]
    InvalidArguments(String),

    /// An outbound adapter reported an error.
    #[error("upstream service error: {0}")]
    Upstream(String),

    /// The tool or workflow could not be executed.
    #[error("tool execution failed: {0}")]
    Execution(String),
}
