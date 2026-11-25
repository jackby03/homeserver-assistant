//! Public endpoints: health and first-boot status.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::dto::HealthResponse;
use crate::setup::SetupStatus;
use crate::state::SharedState;

/// Public routes (no authentication).
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/health", get(health))
        .route("/setup/status", get(setup_status))
}

/// Liveness/readiness probe. Never fails while the process is answering.
async fn health(State(state): State<SharedState>) -> Json<HealthResponse> {
    let uptime = chrono::Utc::now()
        .signed_duration_since(state.started_at)
        .num_seconds()
        .max(0) as u64;
    Json(HealthResponse {
        status: "ok".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        uptime_secs: uptime,
        initialized: state.is_initialized(),
        fake_runtime: state.fake_runtime,
    })
}

/// First-boot status consumed by the wizard redirect logic.
async fn setup_status(State(state): State<SharedState>) -> Json<SetupStatus> {
    Json(state.setup.status(&state.config_snapshot()))
}
