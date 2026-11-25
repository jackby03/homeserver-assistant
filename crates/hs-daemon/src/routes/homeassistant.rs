//! Home Assistant integration routes.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use hs_auth::Permission;
use hs_core::models::AuditOutcome;
use hs_homeassistant::{HaClient, HaProvisioner, ProvisionRequest, ReqwestTransport};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::info;

use crate::error::{HttpError, HttpResult};
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;

/// Home Assistant routes.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/homeassistant/status", get(status))
        .route("/homeassistant/provision", post(provision))
        .route("/homeassistant/config", put(configure))
        .route("/homeassistant/entities", get(entities))
        .route("/homeassistant/scenes", get(scenes))
        .route(
            "/homeassistant/services/{domain}/{service}",
            post(call_service),
        )
        .route(
            "/homeassistant/scenes/{entity_id}/activate",
            post(activate_scene),
        )
}

/// Home Assistant status payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaStatusResponse {
    /// Whether a URL/token was configured.
    pub configured: bool,
    /// Whether the REST API answered.
    pub running: bool,
    /// Home Assistant version when available.
    pub version: Option<String>,
    /// Base URL.
    pub url: Option<String>,
    /// Runtime instance summary when provisioned through the appliance.
    pub instance: Option<hs_core::models::InstanceSummary>,
}

/// Provisioning request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionBody {
    /// Instance name (default `home-assistant`).
    #[serde(default = "default_ha_name")]
    pub name: String,
    /// Port exposed by Home Assistant (default 8123).
    #[serde(default = "default_ha_port")]
    pub port: u16,
}

fn default_ha_name() -> String {
    "home-assistant".to_owned()
}

fn default_ha_port() -> u16 {
    8123
}

/// Manual configuration request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaConfigBody {
    /// Home Assistant base URL, e.g. `http://192.168.1.20:8123`.
    pub url: String,
    /// Long-lived access token.
    pub token: String,
}

async fn status(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<HaStatusResponse>> {
    auth.require(Permission::DashboardView)?;
    let config = state.config_snapshot();
    let url = config.services.home_assistant_url.clone();
    let configured = state.ha.is_configured().await;

    let (running, version) = if configured {
        match state.ha.status().await {
            Ok(status) => (status.running, status.version),
            Err(_) => (false, None),
        }
    } else {
        (false, None)
    };

    let instance = state.runtime.get("home-assistant").await.ok();

    Ok(Json(HaStatusResponse {
        configured,
        running,
        version,
        url,
        instance,
    }))
}

async fn provision(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(body): Json<ProvisionBody>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::AppsDeploy)?;
    let config = state.config_snapshot();
    let provisioner = HaProvisioner::new(state.runtime.clone());
    let report = provisioner
        .provision(ProvisionRequest {
            name: body.name.clone(),
            timezone: config.device.timezone.clone(),
            port: body.port,
        })
        .await
        .map_err(HttpError::from)?;

    let url = report.url.clone();
    state
        .config
        .update(|config| {
            config.services.home_assistant_url = Some(url.clone());
            Ok(())
        })
        .map_err(HttpError::from)?;

    state.audit(
        &auth.user().username,
        "homeassistant.provision",
        Some(&body.name),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"url": report.url})),
    );
    info!(instance = %body.name, "home assistant provisioned");
    Ok(Json(serde_json::json!({
        "instance": report.instance,
        "url": report.url,
    })))
}

async fn configure(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(body): Json<HaConfigBody>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::AgentManage)?;
    hs_core::validation::http_url(&body.url).map_err(HttpError::from)?;
    state
        .secrets
        .set("homeassistant.token", &body.token)
        .map_err(HttpError::from)?;
    state
        .config
        .update(|config| {
            config.services.home_assistant_url = Some(body.url.clone());
            Ok(())
        })
        .map_err(HttpError::from)?;
    let client = HaClient::new(Arc::new(ReqwestTransport::new(&body.url, &body.token)));
    state.ha.configure(Arc::new(client)).await;
    state.audit(
        &auth.user().username,
        "homeassistant.configure",
        None,
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"url": body.url})),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn entities(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::DashboardView)?;
    let entities = state.ha.entities().await.map_err(map_ha_error)?;
    Ok(Json(serde_json::json!({ "entities": entities })))
}

async fn scenes(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::DashboardView)?;
    let entities = state.ha.entities().await.map_err(map_ha_error)?;
    let scenes: Vec<_> = entities
        .into_iter()
        .filter(|entity| entity.domain == "scene")
        .collect();
    Ok(Json(serde_json::json!({ "scenes": scenes })))
}

async fn call_service(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path((domain, service)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::DashboardView)?;
    let data = body.get("data").cloned().unwrap_or(body);
    state
        .ha
        .call_service(&domain, &service, data)
        .await
        .map_err(map_ha_error)?;
    state.audit(
        &auth.user().username,
        "homeassistant.call_service",
        Some(&format!("{domain}.{service}")),
        AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn activate_scene(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(entity_id): Path<String>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::DashboardView)?;
    state
        .ha
        .call_service(
            "scene",
            "turn_on",
            serde_json::json!({"entity_id": entity_id}),
        )
        .await
        .map_err(map_ha_error)?;
    Ok(Json(serde_json::json!({"ok": true})))
}

fn map_ha_error(err: hs_homeassistant::HaError) -> HttpError {
    use axum::http::StatusCode;
    match err {
        hs_homeassistant::HaError::Unavailable(_) => {
            HttpError::unavailable("Home Assistant is not configured or unreachable")
        }
        hs_homeassistant::HaError::Unauthorized => {
            HttpError::new(StatusCode::BAD_GATEWAY, "ha_unauthorized", err.to_string())
        }
        _ => HttpError::new(
            StatusCode::BAD_GATEWAY,
            "homeassistant_error",
            err.to_string(),
        ),
    }
}
