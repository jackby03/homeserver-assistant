//! Runtime instance management: list, lifecycle, limits, snapshots, logs and
//! an exec WebSocket used by the portal terminal.

use std::collections::BTreeMap;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use hs_auth::Permission;
use hs_core::models::{
    AuditOutcome, ExecOutput, InstanceSpec, InstanceSummary, NetworkSegment, ResourceLimits,
    RuntimeKind, SnapshotSummary,
};
use hs_runtime::RuntimeError;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::error::{HttpError, HttpResult};
use crate::events::ServerEvent;
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;

/// Instance routes.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/instances", get(list_instances).post(create_instance))
        .route(
            "/instances/{name}",
            get(get_instance).delete(delete_instance),
        )
        .route("/instances/{name}/limits", patch(update_limits))
        .route("/instances/{name}/start", post(start_instance))
        .route("/instances/{name}/stop", post(stop_instance))
        .route("/instances/{name}/restart", post(restart_instance))
        .route(
            "/instances/{name}/snapshots",
            get(list_snapshots).post(create_snapshot),
        )
        .route(
            "/instances/{name}/snapshots/{snapshot}",
            axum::routing::delete(delete_snapshot),
        )
        .route(
            "/instances/{name}/snapshots/{snapshot}/restore",
            post(restore_snapshot),
        )
        .route("/instances/{name}/logs", get(instance_logs))
        .route("/instances/{name}/exec", get(exec_socket))
}

/// Provider summary shown next to the instance list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    /// Provider identifier (`incus`, `mock`).
    pub name: String,
    /// Whether the provider is reachable.
    pub available: bool,
}

/// `GET /api/v1/instances`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceListResponse {
    /// Instances known by the provider.
    pub instances: Vec<InstanceSummary>,
    /// Provider status.
    pub provider: ProviderInfo,
}

/// Request body for creating an instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInstanceRequest {
    /// Instance name.
    pub name: String,
    /// Container or VM.
    pub kind: RuntimeKind,
    /// Image alias or OCI reference.
    pub image: String,
    /// Network segment (`lan`, `dmz` or none).
    #[serde(default)]
    pub segment: Option<NetworkSegment>,
    /// Resource limits.
    #[serde(default)]
    pub limits: ResourceLimits,
    /// Environment variables.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Start after creation (default true).
    #[serde(default = "default_true")]
    pub start: bool,
}

fn default_true() -> bool {
    true
}

impl CreateInstanceRequest {
    fn into_spec(self) -> InstanceSpec {
        InstanceSpec {
            name: self.name,
            kind: self.kind,
            image: self.image,
            segment: self.segment,
            limits: self.limits,
            env: self.env,
            start: self.start,
        }
    }
}

/// Query for logs.
#[derive(Debug, Clone, Deserialize)]
pub struct LogsQuery {
    /// Number of lines (default 200, max 5000).
    #[serde(default)]
    pub lines: Option<usize>,
}

async fn list_instances(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<InstanceListResponse>> {
    auth.require(Permission::InstancesRead)?;
    let instances = state.runtime.list().await.map_err(map_runtime_error)?;
    let available = state.runtime.available();
    Ok(Json(InstanceListResponse {
        instances,
        provider: ProviderInfo {
            name: state.runtime.provider_name().to_owned(),
            available,
        },
    }))
}

async fn get_instance(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(name): Path<String>,
) -> HttpResult<Json<InstanceSummary>> {
    auth.require(Permission::InstancesRead)?;
    Ok(Json(
        state.runtime.get(&name).await.map_err(map_runtime_error)?,
    ))
}

async fn create_instance(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(request): Json<CreateInstanceRequest>,
) -> HttpResult<Json<InstanceSummary>> {
    auth.require(Permission::InstancesManage)?;
    let spec = request.into_spec();
    spec.validate().map_err(HttpError::from)?;
    let created = state
        .runtime
        .create(spec.clone())
        .await
        .map_err(map_runtime_error)?;
    state.audit(
        &auth.user().username,
        "instance.create",
        Some(&spec.name),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({
            "kind": format!("{:?}", spec.kind),
            "image": spec.image,
            "segment": spec.segment.map(NetworkSegment::id),
        })),
    );
    state.events.publish_server(ServerEvent::Instance {
        name: created.name.clone(),
        status: format!("{:?}", created.status).to_lowercase(),
    });
    info!(instance = %spec.name, "instance created");
    Ok(Json(created))
}

async fn delete_instance(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(name): Path<String>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::InstancesManage)?;
    state
        .runtime
        .delete(&name)
        .await
        .map_err(map_runtime_error)?;
    state.audit(
        &auth.user().username,
        "instance.delete",
        Some(&name),
        AuditOutcome::Success,
        None,
        None,
    );
    state.events.publish_server(ServerEvent::Instance {
        name: name.clone(),
        status: "deleted".to_owned(),
    });
    Ok(Json(serde_json::json!({"ok": true, "name": name})))
}

async fn start_instance(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(name): Path<String>,
) -> HttpResult<Json<InstanceSummary>> {
    auth.require(Permission::InstancesManage)?;
    state
        .runtime
        .start(&name)
        .await
        .map_err(map_runtime_error)?;
    lifecycle_audit(&state, &auth, "instance.start", &name);
    Ok(Json(
        state.runtime.get(&name).await.map_err(map_runtime_error)?,
    ))
}

async fn stop_instance(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(name): Path<String>,
) -> HttpResult<Json<InstanceSummary>> {
    auth.require(Permission::InstancesManage)?;
    state.runtime.stop(&name).await.map_err(map_runtime_error)?;
    lifecycle_audit(&state, &auth, "instance.stop", &name);
    Ok(Json(
        state.runtime.get(&name).await.map_err(map_runtime_error)?,
    ))
}

async fn restart_instance(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(name): Path<String>,
) -> HttpResult<Json<InstanceSummary>> {
    auth.require(Permission::InstancesManage)?;
    state
        .runtime
        .restart(&name)
        .await
        .map_err(map_runtime_error)?;
    lifecycle_audit(&state, &auth, "instance.restart", &name);
    Ok(Json(
        state.runtime.get(&name).await.map_err(map_runtime_error)?,
    ))
}

fn lifecycle_audit(state: &SharedState, auth: &AdminAuth, action: &str, name: &str) {
    state.audit(
        &auth.user().username,
        action,
        Some(name),
        AuditOutcome::Success,
        None,
        None,
    );
    state.events.publish_server(ServerEvent::Instance {
        name: name.to_owned(),
        status: action.rsplit('.').next().unwrap_or("changed").to_owned(),
    });
}

/// Request body for limits and snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsRequest {
    /// CPU limit in cores.
    pub cpu_limit: Option<f32>,
    /// Memory limit in MiB.
    pub memory_mb: Option<u64>,
}

async fn update_limits(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(name): Path<String>,
    Json(request): Json<LimitsRequest>,
) -> HttpResult<Json<InstanceSummary>> {
    auth.require(Permission::InstancesManage)?;
    let limits = ResourceLimits {
        cpu_limit: request.cpu_limit,
        memory_mb: request.memory_mb,
    };
    limits.validate().map_err(HttpError::from)?;
    state
        .runtime
        .update_limits(&name, limits)
        .await
        .map_err(map_runtime_error)?;
    state.audit(
        &auth.user().username,
        "instance.update_limits",
        Some(&name),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"cpu_limit": limits.cpu_limit, "memory_mb": limits.memory_mb})),
    );
    Ok(Json(
        state.runtime.get(&name).await.map_err(map_runtime_error)?,
    ))
}

async fn list_snapshots(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(name): Path<String>,
) -> HttpResult<Json<Vec<SnapshotSummary>>> {
    auth.require(Permission::InstancesRead)?;
    Ok(Json(
        state
            .runtime
            .list_snapshots(&name)
            .await
            .map_err(map_runtime_error)?,
    ))
}

/// Request body for creating a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRequest {
    /// Snapshot name (letters, digits, hyphens).
    pub name: String,
}

async fn create_snapshot(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(name): Path<String>,
    Json(request): Json<SnapshotRequest>,
) -> HttpResult<Json<SnapshotSummary>> {
    auth.require(Permission::InstancesManage)?;
    let snapshot = state
        .runtime
        .snapshot(&name, &request.name)
        .await
        .map_err(map_runtime_error)?;
    state.audit(
        &auth.user().username,
        "instance.snapshot",
        Some(&name),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"snapshot": request.name})),
    );
    Ok(Json(snapshot))
}

async fn delete_snapshot(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path((name, snapshot)): Path<(String, String)>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::InstancesManage)?;
    state
        .runtime
        .delete_snapshot(&name, &snapshot)
        .await
        .map_err(map_runtime_error)?;
    state.audit(
        &auth.user().username,
        "instance.snapshot_delete",
        Some(&name),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"snapshot": snapshot})),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn restore_snapshot(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path((name, snapshot)): Path<(String, String)>,
) -> HttpResult<Json<InstanceSummary>> {
    auth.require(Permission::InstancesManage)?;
    state
        .runtime
        .restore(&name, &snapshot)
        .await
        .map_err(map_runtime_error)?;
    state.audit(
        &auth.user().username,
        "instance.snapshot_restore",
        Some(&name),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"snapshot": snapshot})),
    );
    Ok(Json(
        state.runtime.get(&name).await.map_err(map_runtime_error)?,
    ))
}

async fn instance_logs(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(name): Path<String>,
    Query(query): Query<LogsQuery>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::InstancesRead)?;
    let lines = query.lines.unwrap_or(200).clamp(1, 5000);
    let logs = state
        .runtime
        .logs(&name, lines)
        .await
        .map_err(map_runtime_error)?;
    Ok(Json(serde_json::json!({ "name": name, "logs": logs })))
}

// ---------------------------------------------------------------------------
// Exec WebSocket
// ---------------------------------------------------------------------------

/// Client -> server messages for the exec socket.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ExecClientMessage {
    /// Run a command line (split on whitespace) inside the instance.
    Input { data: String },
    /// Run a command with explicit argv.
    Command { command: Vec<String> },
}

/// Server -> client messages for the exec socket.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ExecServerMessage {
    Output { data: String },
    Error { message: String },
    Exit { code: Option<i32> },
}

async fn exec_socket(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(name): Path<String>,
    ws: WebSocketUpgrade,
) -> HttpResult<impl IntoResponse> {
    auth.require(Permission::InstancesManage)?;
    Ok(ws.on_upgrade(move |socket| handle_exec(socket, state, name)))
}

async fn handle_exec(mut socket: WebSocket, state: SharedState, name: String) {
    while let Some(result) = socket.recv().await {
        let message = match result {
            Ok(message) => message,
            Err(err) => {
                warn!(instance = %name, error = %err, "exec websocket closed");
                break;
            }
        };
        let Message::Text(text) = message else {
            continue;
        };
        let request: ExecClientMessage = match serde_json::from_str(&text) {
            Ok(request) => request,
            Err(err) => {
                let payload = serde_json::to_string(&ExecServerMessage::Error {
                    message: format!("invalid request: {err}"),
                })
                .unwrap_or_default();
                if socket.send(Message::Text(payload.into())).await.is_err() {
                    break;
                }
                continue;
            }
        };
        let command = match request {
            ExecClientMessage::Input { data } => data
                .split_whitespace()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>(),
            ExecClientMessage::Command { command } => command,
        };
        if command.is_empty() {
            continue;
        }

        let output: ExecOutput = match state.runtime.exec(&name, &command).await {
            Ok(output) => output,
            Err(err) => {
                let payload = serde_json::to_string(&ExecServerMessage::Error {
                    message: err.to_string(),
                })
                .unwrap_or_default();
                let _ = socket.send(Message::Text(payload.into())).await;
                continue;
            }
        };

        let mut combined = String::new();
        if !output.stdout.is_empty() {
            combined.push_str(&output.stdout);
        }
        if !output.stderr.is_empty() {
            if !combined.is_empty() && !combined.ends_with('\n') {
                combined.push('\n');
            }
            combined.push_str(&output.stderr);
        }
        if !combined.is_empty() {
            let payload = serde_json::to_string(&ExecServerMessage::Output { data: combined })
                .unwrap_or_default();
            if socket.send(Message::Text(payload.into())).await.is_err() {
                break;
            }
        }
        let payload = serde_json::to_string(&ExecServerMessage::Exit {
            code: output.exit_code,
        })
        .unwrap_or_default();
        if socket.send(Message::Text(payload.into())).await.is_err() {
            break;
        }
    }
}

fn map_runtime_error(err: RuntimeError) -> HttpError {
    use axum::http::StatusCode;
    let status = match &err {
        RuntimeError::NotFound(_) => StatusCode::NOT_FOUND,
        RuntimeError::Conflict(_) => StatusCode::CONFLICT,
        RuntimeError::Invalid(_) => StatusCode::BAD_REQUEST,
        RuntimeError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        RuntimeError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
        RuntimeError::Backend(_) => StatusCode::BAD_GATEWAY,
    };
    HttpError::new(status, "runtime_error", err.to_string())
}
