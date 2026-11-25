//! Application catalog and deployment routes.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use hs_apps::builtin_catalog;
use hs_auth::Permission;
use hs_core::models::{AppRecord, AppTemplate, AuditOutcome, DeployAppRequest};
use uuid::Uuid;

use crate::error::{HttpError, HttpResult};
use crate::events::ServerEvent;
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;

/// Application routes.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/apps/catalog", get(catalog))
        .route("/apps", get(list_apps).post(deploy_app))
        .route("/apps/{id}", axum::routing::delete(remove_app))
        .route("/apps/{id}/start", post(start_app))
        .route("/apps/{id}/stop", post(stop_app))
}

async fn catalog(auth: AdminAuth) -> HttpResult<Json<Vec<AppTemplate>>> {
    auth.require(Permission::AppsRead)?;
    Ok(Json(builtin_catalog()))
}

async fn list_apps(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<Vec<AppRecord>>> {
    auth.require(Permission::AppsRead)?;
    Ok(Json(state.apps.list().map_err(HttpError::from)?))
}

async fn deploy_app(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(request): Json<DeployAppRequest>,
) -> HttpResult<Json<AppRecord>> {
    auth.require(Permission::AppsDeploy)?;
    let record = state.apps.deploy(request).await.map_err(HttpError::from)?;
    if let Err(err) = crate::caddy::sync_app(&state, &record).await {
        tracing::warn!(error = %err, app = %record.name, "caddy route not written");
        state.events.publish_server(ServerEvent::Notification {
            level: "warning".to_owned(),
            message: format!(
                "application `{}` deployed but the Caddy route failed: {err}",
                record.name
            ),
        });
    }
    state.audit(
        &auth.user().username,
        "app.deploy",
        Some(&record.name),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({
            "segment": record.segment.id(),
            "image": record.image,
            "hostname": record.hostname,
        })),
    );
    state.events.publish_server(ServerEvent::Notification {
        level: "success".to_owned(),
        message: format!("application `{}` deployed", record.name),
    });
    Ok(Json(record))
}

async fn remove_app(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::AppsDeploy)?;
    state.apps.remove(id).await.map_err(HttpError::from)?;
    if let Err(err) = crate::caddy::remove_app(&state, id).await {
        tracing::warn!(error = %err, %id, "caddy route not removed");
    }
    state.audit(
        &auth.user().username,
        "app.remove",
        Some(&id.to_string()),
        AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn start_app(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
) -> HttpResult<Json<AppRecord>> {
    auth.require(Permission::AppsDeploy)?;
    let record = state.apps.start(id).await.map_err(HttpError::from)?;
    if let Err(err) = crate::caddy::sync_app(&state, &record).await {
        tracing::warn!(error = %err, app = %record.name, "caddy route not refreshed");
    }
    state.audit(
        &auth.user().username,
        "app.start",
        Some(&record.name),
        AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(record))
}

async fn stop_app(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
) -> HttpResult<Json<AppRecord>> {
    auth.require(Permission::AppsDeploy)?;
    let record = state.apps.stop(id).await.map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "app.stop",
        Some(&record.name),
        AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(record))
}
