//! AI agent routes: providers, tools, chat and node-based workflows.

use std::collections::BTreeSet;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use hs_agent::UpsertProviderRequest;
use hs_auth::Permission;
use hs_core::models::{
    AgentChatRequest, AgentChatResponse, AuditOutcome, Profile, ProviderRecord, Workflow,
};
use hs_tools::{ToolDescriptor, WorkflowEngine, WorkflowRun};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{HttpError, HttpResult};
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;

/// Agent routes.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route(
            "/agent/providers",
            get(list_providers).post(upsert_provider),
        )
        .route(
            "/agent/providers/{id}",
            axum::routing::delete(delete_provider),
        )
        .route("/agent/providers/{id}/activate", post(activate_provider))
        .route("/agent/tools", get(list_tools))
        .route("/agent/chat", post(chat))
        .route(
            "/agent/workflows",
            get(list_workflows).post(create_workflow),
        )
        .route(
            "/agent/workflows/{id}",
            axum::routing::delete(delete_workflow).put(update_workflow),
        )
        .route("/agent/workflows/{id}/run", post(run_workflow))
}

/// Workflow creation/update payload (the server assigns ids/timestamps).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowBody {
    /// Display name.
    pub name: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// Whether the workflow can run.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Nodes.
    pub nodes: Vec<hs_core::models::WorkflowNode>,
    /// Edges.
    #[serde(default)]
    pub edges: Vec<hs_core::models::WorkflowEdge>,
}

fn default_enabled() -> bool {
    true
}

/// Workflow execution payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunWorkflowBody {
    /// Profile whose permissions apply.
    pub profile_id: Uuid,
    /// Trigger input available to tool nodes as `{{input}}`.
    #[serde(default)]
    pub input: serde_json::Value,
}

async fn list_providers(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<Vec<ProviderRecord>>> {
    auth.require(Permission::AgentRead)?;
    Ok(Json(state.agent.list_providers().map_err(HttpError::from)?))
}

async fn upsert_provider(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(request): Json<UpsertProviderRequest>,
) -> HttpResult<Json<ProviderRecord>> {
    auth.require(Permission::AgentManage)?;
    let record = state
        .agent
        .upsert_provider(request)
        .map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "agent.provider_upsert",
        Some(&record.id.to_string()),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({
            "kind": record.kind.id(),
            "model": record.model,
            "enabled": record.enabled,
        })),
    );
    Ok(Json(record))
}

async fn delete_provider(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::AgentManage)?;
    state
        .agent
        .delete_provider(id)
        .await
        .map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "agent.provider_delete",
        Some(&id.to_string()),
        AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn activate_provider(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::AgentManage)?;
    state
        .agent
        .set_active_provider(id)
        .await
        .map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "agent.provider_activate",
        Some(&id.to_string()),
        AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn list_tools(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<Vec<ToolDescriptor>>> {
    auth.require(Permission::AgentRead)?;
    Ok(Json(state.tools.list()))
}

async fn chat(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(request): Json<AgentChatRequest>,
) -> HttpResult<Json<AgentChatResponse>> {
    auth.require(Permission::AgentManage)?;
    let profile = state
        .profiles
        .get(request.profile_id)
        .map_err(HttpError::from)?;
    state
        .kiosk
        .set_agent_activity(hs_core::events::AgentActivity::Thinking);
    let result = state.agent.chat(request).await;
    match result {
        Ok(response) => {
            state
                .kiosk
                .set_agent_activity(hs_core::events::AgentActivity::Idle);
            state.audit(
                &auth.user().username,
                "agent.chat",
                Some(&profile.id.to_string()),
                AuditOutcome::Success,
                None,
                Some(serde_json::json!({
                    "model": response.model,
                    "tools": response.tool_trace.len(),
                    "prompt_tokens": response.usage.prompt_tokens,
                    "completion_tokens": response.usage.completion_tokens,
                })),
            );
            Ok(Json(response))
        }
        Err(err) => {
            state
                .kiosk
                .set_agent_activity(hs_core::events::AgentActivity::Error);
            state.audit(
                &auth.user().username,
                "agent.chat",
                Some(&profile.id.to_string()),
                AuditOutcome::Failure,
                None,
                Some(serde_json::json!({"error": err.to_string()})),
            );
            Err(HttpError::from(err))
        }
    }
}

async fn list_workflows(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<Vec<Workflow>>> {
    auth.require(Permission::AgentRead)?;
    let workflows = state.db.list_workflows().map_err(HttpError::from)?;
    Ok(Json(workflows))
}

async fn create_workflow(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(body): Json<WorkflowBody>,
) -> HttpResult<Json<Workflow>> {
    auth.require(Permission::AgentManage)?;
    validate_workflow(&body)?;
    let now = chrono::Utc::now();
    let workflow = Workflow {
        id: Uuid::new_v4(),
        name: body.name,
        description: body.description,
        enabled: body.enabled,
        nodes: body.nodes,
        edges: body.edges,
        created_at: now,
        updated_at: now,
    };
    state.db.save_workflow(&workflow).map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "agent.workflow_create",
        Some(&workflow.id.to_string()),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"name": workflow.name, "nodes": workflow.nodes.len()})),
    );
    Ok(Json(workflow))
}

async fn update_workflow(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
    Json(body): Json<WorkflowBody>,
) -> HttpResult<Json<Workflow>> {
    auth.require(Permission::AgentManage)?;
    validate_workflow(&body)?;
    let existing = state
        .db
        .workflow(id)
        .map_err(HttpError::from)?
        .ok_or_else(|| HttpError::not_found(format!("workflow {id}")))?;
    let workflow = Workflow {
        id,
        name: body.name,
        description: body.description,
        enabled: body.enabled,
        nodes: body.nodes,
        edges: body.edges,
        created_at: existing.created_at,
        updated_at: chrono::Utc::now(),
    };
    state.db.save_workflow(&workflow).map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "agent.workflow_update",
        Some(&id.to_string()),
        AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(workflow))
}

async fn delete_workflow(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::AgentManage)?;
    state.db.delete_workflow(id).map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "agent.workflow_delete",
        Some(&id.to_string()),
        AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn run_workflow(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
    Json(body): Json<RunWorkflowBody>,
) -> HttpResult<Json<WorkflowRun>> {
    auth.require(Permission::AgentManage)?;
    let workflow = state
        .db
        .workflow(id)
        .map_err(HttpError::from)?
        .ok_or_else(|| HttpError::not_found(format!("workflow {id}")))?;
    let profile: Profile = state
        .profiles
        .get(body.profile_id)
        .map_err(HttpError::from)?;
    let engine = WorkflowEngine::new(state.tools.clone());
    let run = engine
        .run(&workflow, &profile, body.input)
        .await
        .map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "agent.workflow_run",
        Some(&id.to_string()),
        if run.success {
            AuditOutcome::Success
        } else {
            AuditOutcome::Failure
        },
        None,
        Some(serde_json::json!({"profile": profile.name, "steps": run.steps.len()})),
    );
    Ok(Json(run))
}

fn validate_workflow(body: &WorkflowBody) -> Result<(), HttpError> {
    if body.name.trim().is_empty() {
        return Err(HttpError::bad_request("workflow name is required"));
    }
    if body.nodes.is_empty() {
        return Err(HttpError::bad_request("workflow needs at least one node"));
    }
    if body.nodes.len() > 64 {
        return Err(HttpError::bad_request("workflow supports at most 64 nodes"));
    }
    let mut ids = BTreeSet::new();
    for node in &body.nodes {
        if node.id.trim().is_empty() {
            return Err(HttpError::bad_request("every node needs an id"));
        }
        if !ids.insert(node.id.clone()) {
            return Err(HttpError::bad_request(format!(
                "duplicate node id `{}`",
                node.id
            )));
        }
    }
    for edge in &body.edges {
        if !ids.contains(&edge.from) || !ids.contains(&edge.to) {
            return Err(HttpError::bad_request(
                "workflow edges must reference existing nodes",
            ));
        }
    }
    Ok(())
}
