//! Audit trail route.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use hs_auth::Permission;
use hs_core::models::AuditEntry;
use serde::Deserialize;

use crate::error::HttpResult;
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;

/// Audit routes.
pub fn router() -> Router<SharedState> {
    Router::new().route("/audit", get(list_audit))
}

/// Query parameters for the audit endpoint.
#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    /// Maximum number of entries (default 50, max 500).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `GET /api/v1/audit?limit=50`
async fn list_audit(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Query(query): Query<AuditQuery>,
) -> HttpResult<Json<Vec<AuditEntry>>> {
    auth.require(Permission::AuditView)?;
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    Ok(Json(
        state
            .auth
            .list_audit(limit)
            .map_err(crate::error::HttpError::from)?,
    ))
}
