//! Local IoT discovery routes.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use hs_auth::Permission;
use hs_core::models::{AuditOutcome, DiscoveredDevice};

use crate::error::{HttpError, HttpResult};
use crate::events::ServerEvent;
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;

/// Discovery routes.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/discovery/devices", get(list_devices))
        .route("/discovery/scan", post(scan))
        .route("/discovery/devices/{id}/adopt", post(adopt))
}

async fn list_devices(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<Vec<DiscoveredDevice>>> {
    auth.require(Permission::DashboardView)?;
    let devices = state.discovery_devices.read().await.clone();
    Ok(Json(devices))
}

async fn scan(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<Vec<DiscoveredDevice>>> {
    auth.require(Permission::DashboardView)?;
    let devices = state.discovery.scan().await;
    {
        let mut cache = state.discovery_devices.write().await;
        *cache = devices.clone();
    }
    state.audit(
        &auth.user().username,
        "discovery.scan",
        None,
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"devices": devices.len()})),
    );
    state.events.publish_server(ServerEvent::Discovery {
        count: devices.len(),
    });
    Ok(Json(devices))
}

async fn adopt(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<String>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::DashboardView)?;
    let updated = {
        let mut cache = state.discovery_devices.write().await;
        let Some(device) = cache.iter_mut().find(|device| device.id == id) else {
            return Err(HttpError::not_found(format!("device `{id}`")));
        };
        device.adopted = true;
        device.clone()
    };
    state.audit(
        &auth.user().username,
        "discovery.adopt",
        Some(&id),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"ip": updated.ip, "vendor": updated.vendor})),
    );
    Ok(Json(serde_json::json!({"ok": true, "device": updated})))
}
