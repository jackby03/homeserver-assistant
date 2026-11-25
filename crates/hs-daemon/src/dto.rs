//! Data transfer objects returned by the HTTP API.
//!
//! Domain models are stored/serialized internally; API responses use explicit
//! views so that sensitive fields (password hashes, session ids, embeddings)
//! can never leak by accident.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use hs_core::models::{Role, User};

/// Public representation of an administrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserView {
    /// User id.
    pub id: Uuid,
    /// Login name.
    pub username: String,
    /// Display name.
    pub display_name: String,
    /// RBAC role.
    pub role: Role,
    /// Whether the account is disabled.
    pub disabled: bool,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last successful login.
    pub last_login_at: Option<DateTime<Utc>>,
}

impl From<&User> for UserView {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            role: user.role,
            disabled: user.disabled,
            created_at: user.created_at,
            last_login_at: user.last_login_at,
        }
    }
}

/// Health/version payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Always `ok` when the daemon answers.
    pub status: String,
    /// Crate version.
    pub version: String,
    /// Seconds since daemon start.
    pub uptime_secs: u64,
    /// Whether first boot completed.
    pub initialized: bool,
    /// Whether the runtime provider is a development fake.
    pub fake_runtime: bool,
}

/// Response returned after login/admin bootstrap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResponse {
    /// Authenticated user.
    pub user: UserView,
    /// CSRF token to send in `X-HS-CSRF` for mutating requests.
    pub csrf_token: String,
}

/// `GET /api/v1/system/version`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionResponse {
    /// Daemon version.
    pub version: String,
    /// Git revision if provided at build time.
    pub revision: String,
    /// Uptime in seconds.
    pub uptime_secs: u64,
}

/// Generic acknowledgement payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AckResponse {
    /// Whether the operation was applied.
    pub ok: bool,
}

impl AckResponse {
    /// Successful acknowledgement.
    #[must_use]
    pub fn ok() -> Self {
        Self { ok: true }
    }
}
