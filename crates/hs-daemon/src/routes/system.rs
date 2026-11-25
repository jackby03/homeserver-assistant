//! Host metrics routes.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use hs_auth::Permission;
use hs_system::{AudioSink, Overview, PeripheralStatus};

use crate::dto::VersionResponse;
use crate::error::HttpResult;
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;

/// System routes.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/system/overview", get(overview))
        .route("/system/version", get(version))
        .route("/system/peripherals", get(peripherals))
        .route("/system/audio/sinks", get(audio_sinks))
        .route("/system/power/policy", get(power_policy))
}

/// `GET /api/v1/system/overview`
async fn overview(State(state): State<SharedState>, auth: AdminAuth) -> HttpResult<Json<Overview>> {
    auth.require(Permission::DashboardView)?;
    let overview = state
        .system
        .overview()
        .await
        .map_err(crate::error::HttpError::from)?;
    Ok(Json(overview))
}

/// `GET /api/v1/system/version`
async fn version(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<VersionResponse>> {
    auth.require(Permission::DashboardView)?;
    let uptime = chrono::Utc::now()
        .signed_duration_since(state.started_at)
        .num_seconds()
        .max(0) as u64;
    Ok(Json(VersionResponse {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        revision: option_env!("HS_GIT_REVISION")
            .unwrap_or("unknown")
            .to_owned(),
        uptime_secs: uptime,
    }))
}

/// `GET /api/v1/system/peripherals`
async fn peripherals(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<PeripheralStatus>> {
    auth.require(Permission::DashboardView)?;
    Ok(Json(state.system.peripherals()))
}

/// `GET /api/v1/system/audio/sinks`
async fn audio_sinks(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<Vec<AudioSink>>> {
    auth.require(Permission::DashboardView)?;
    Ok(Json(state.system.audio_sinks()))
}

/// `GET /api/v1/system/power/policy`
async fn power_policy(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<hs_system::PowerPolicyReport>> {
    auth.require(Permission::DashboardView)?;
    Ok(Json(state.system.power_policy()))
}
