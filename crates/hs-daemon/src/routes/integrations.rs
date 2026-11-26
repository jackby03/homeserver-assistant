//! Explicit opt-in integration configuration (Composio).
//!
//! The API never returns the API key. The key is written to the encrypted
//! secret store and the non-secret configuration is persisted separately.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use hs_auth::Permission;
use hs_core::models::AuditOutcome;
use serde::{Deserialize, Serialize};

use crate::error::{HttpError, HttpResult};
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;

/// Integration routes.
pub fn router() -> Router<SharedState> {
    Router::new().route(
        "/integrations/composio",
        get(get_composio).put(update_composio),
    )
}

/// Public Composio status. The API key is represented only by `has_api_key`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioStatus {
    /// Whether the integration is enabled.
    pub enabled: bool,
    /// Base URL used for the gateway.
    pub base_url: String,
    /// Household user id sent to Composio.
    pub user_id: String,
    /// Connected account used for the allow-list.
    pub connected_account_id: Option<String>,
    /// Whether an API key is stored.
    pub has_api_key: bool,
    /// Read-only calendar action slug.
    pub calendar_list_action: String,
    /// Calendar creation action slug.
    pub calendar_create_action: String,
    /// Email send action slug.
    pub email_send_action: String,
}

/// Update payload. The API key is optional: omit it to keep the stored key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateComposioRequest {
    /// Enable or disable the integration.
    pub enabled: bool,
    /// Composio API base URL.
    pub base_url: String,
    /// Household user id.
    pub user_id: String,
    /// Connected account id, if any.
    #[serde(default)]
    pub connected_account_id: Option<String>,
    /// Optional new API key. Never echoed back.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Read-only calendar action slug.
    pub calendar_list_action: String,
    /// Calendar creation action slug.
    pub calendar_create_action: String,
    /// Email send action slug.
    pub email_send_action: String,
}

async fn get_composio(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<ComposioStatus>> {
    auth.require(Permission::AgentRead)?;
    let config = state.config_snapshot().integrations.composio;
    let has_api_key = state
        .secrets
        .contains(&config.api_key_secret)
        .map_err(HttpError::from)?;
    Ok(Json(ComposioStatus {
        enabled: config.enabled,
        base_url: config.base_url,
        user_id: config.user_id,
        connected_account_id: config.connected_account_id,
        has_api_key,
        calendar_list_action: config.calendar_list_action,
        calendar_create_action: config.calendar_create_action,
        email_send_action: config.email_send_action,
    }))
}

async fn update_composio(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(request): Json<UpdateComposioRequest>,
) -> HttpResult<Json<ComposioStatus>> {
    auth.require(Permission::AgentManage)?;
    let api_key = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if api_key.as_ref().is_some_and(|key| key.len() > 512) {
        return Err(HttpError::bad_request("Composio API key is too long"));
    }
    let connected_account_id = request
        .connected_account_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let updated = state
        .config
        .update(|config| {
            config.integrations.composio.enabled = request.enabled;
            config.integrations.composio.base_url = request.base_url.clone();
            config.integrations.composio.user_id = request.user_id.clone();
            config.integrations.composio.connected_account_id = connected_account_id.clone();
            config.integrations.composio.calendar_list_action =
                request.calendar_list_action.clone();
            config.integrations.composio.calendar_create_action =
                request.calendar_create_action.clone();
            config.integrations.composio.email_send_action = request.email_send_action.clone();
            Ok(())
        })
        .map_err(HttpError::from)?;
    let has_new_api_key = api_key.is_some();
    if let Some(key) = api_key {
        state
            .secrets
            .set(&updated.integrations.composio.api_key_secret, &key)
            .map_err(HttpError::from)?;
    }
    state.audit(
        &auth.user().username,
        "integrations.composio_update",
        None,
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({
            "enabled": updated.integrations.composio.enabled,
            "base_url": updated.integrations.composio.base_url,
            "has_new_api_key": has_new_api_key,
            "connected_account": updated.integrations.composio.connected_account_id,
        })),
    );
    let composio = updated.integrations.composio;
    Ok(Json(ComposioStatus {
        enabled: composio.enabled,
        base_url: composio.base_url,
        user_id: composio.user_id,
        connected_account_id: composio.connected_account_id,
        has_api_key: state
            .secrets
            .contains(&composio.api_key_secret)
            .map_err(HttpError::from)?,
        calendar_list_action: composio.calendar_list_action,
        calendar_create_action: composio.calendar_create_action,
        email_send_action: composio.email_send_action,
    }))
}
