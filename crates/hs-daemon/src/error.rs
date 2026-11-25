//! HTTP error mapping.
//!
//! Domain crates return their own error enums. This module converts them into
//! [`ApiError`] with the right status code and a stable machine-readable code,
//! so the TypeScript client can handle errors uniformly.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use hs_auth::AuthError;
use hs_core::api::ApiError;
use hs_core::CoreError;

/// Error type returned by HTTP handlers.
///
/// The JSON body is boxed to keep `Result<T, HttpError>` small (clippy
/// `result_large_err`); handlers return this type by value.
#[derive(Debug)]
pub struct HttpError {
    /// HTTP status code.
    pub status: StatusCode,
    /// JSON body.
    pub body: Box<ApiError>,
}

impl HttpError {
    /// Build an error with an explicit status.
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            body: Box::new(ApiError::new(code, message)),
        }
    }

    /// 400 Bad Request.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }

    /// 401 Unauthorized.
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    /// 403 Forbidden.
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    /// 404 Not Found.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    /// 409 Conflict.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    /// 429 Too Many Requests.
    pub fn too_many_requests(message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "too_many_attempts", message)
    }

    /// 503 Service Unavailable.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
            message,
        )
    }

    /// 500 Internal Server Error. Details are not exposed to clients.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }

    /// Attach structured details to the response body.
    #[must_use]
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.body = Box::new(self.body.as_ref().clone().with_details(details));
        self
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (self.status, Json(*self.body)).into_response()
    }
}

impl From<CoreError> for HttpError {
    fn from(err: CoreError) -> Self {
        let status = match &err {
            CoreError::Validation { .. } | CoreError::Config(_) => StatusCode::BAD_REQUEST,
            CoreError::NotFound(_) => StatusCode::NOT_FOUND,
            CoreError::Conflict(_) => StatusCode::CONFLICT,
            CoreError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            CoreError::Forbidden(_) => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            body: Box::new(ApiError::from(err)),
        }
    }
}

impl From<AuthError> for HttpError {
    fn from(err: AuthError) -> Self {
        let status = match &err {
            AuthError::InvalidCredentials | AuthError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AuthError::Forbidden(_) => StatusCode::FORBIDDEN,
            AuthError::NotFound(_) => StatusCode::NOT_FOUND,
            AuthError::Conflict(_) => StatusCode::CONFLICT,
            AuthError::Validation(_) => StatusCode::BAD_REQUEST,
            AuthError::TooManyAttempts => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(status, err.code(), err.to_string())
    }
}

impl From<hs_runtime::RuntimeError> for HttpError {
    fn from(err: hs_runtime::RuntimeError) -> Self {
        use hs_runtime::RuntimeError;
        let status = match &err {
            RuntimeError::NotFound(_) => StatusCode::NOT_FOUND,
            RuntimeError::Conflict(_) => StatusCode::CONFLICT,
            RuntimeError::Invalid(_) => StatusCode::BAD_REQUEST,
            RuntimeError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            RuntimeError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
            RuntimeError::Backend(_) => StatusCode::BAD_GATEWAY,
        };
        Self::new(status, err.code(), err.to_string())
    }
}

impl From<hs_profiles::ProfileError> for HttpError {
    fn from(err: hs_profiles::ProfileError) -> Self {
        use hs_profiles::ProfileError;
        let status = match &err {
            ProfileError::NotFound => StatusCode::NOT_FOUND,
            ProfileError::Validation(_) => StatusCode::BAD_REQUEST,
            ProfileError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(status, "profile_error", err.to_string())
    }
}

impl From<hs_tools::ToolError> for HttpError {
    fn from(err: hs_tools::ToolError) -> Self {
        use hs_tools::ToolError;
        let status = match &err {
            ToolError::Forbidden { .. } => StatusCode::FORBIDDEN,
            ToolError::NotFound(_) => StatusCode::NOT_FOUND,
            ToolError::InvalidArguments(_) => StatusCode::BAD_REQUEST,
            ToolError::Upstream(_) | ToolError::Execution(_) => StatusCode::BAD_GATEWAY,
        };
        Self::new(status, "tool_error", err.to_string())
    }
}

impl From<hs_agent::AgentError> for HttpError {
    fn from(err: hs_agent::AgentError) -> Self {
        use hs_agent::AgentError;
        let status = match &err {
            AgentError::NotConfigured => StatusCode::SERVICE_UNAVAILABLE,
            AgentError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            AgentError::Unauthorized(_) => StatusCode::BAD_GATEWAY,
            AgentError::Provider(_) | AgentError::Tool(_) => StatusCode::BAD_GATEWAY,
            AgentError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(status, "agent_error", err.to_string())
    }
}

impl From<hs_network::NetworkError> for HttpError {
    fn from(err: hs_network::NetworkError) -> Self {
        use hs_network::NetworkError;
        let status = match &err {
            NetworkError::Invalid(_) => StatusCode::BAD_REQUEST,
            NetworkError::Unsupported(_) => StatusCode::NOT_IMPLEMENTED,
            NetworkError::Apply(_) | NetworkError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(status, "network_error", err.to_string())
    }
}

impl From<serde_json::Error> for HttpError {
    fn from(err: serde_json::Error) -> Self {
        Self::bad_request(format!("invalid JSON body: {err}"))
    }
}

/// Result alias for handlers.
pub type HttpResult<T> = Result<T, HttpError>;
