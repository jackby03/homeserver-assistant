//! Voice assistant configuration and status routes.
//!
//! The assistant name is the wake word (`"Hola Aura, ..."`). Changing it here
//! updates the kiosk hint and the local wake-word matcher immediately; no
//! restart and no cloud service are involved.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use hs_auth::Permission;
use hs_core::config::VoiceConfig;
use hs_core::models::AuditOutcome;
use serde::{Deserialize, Serialize};

use crate::error::{HttpError, HttpResult};
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;
use crate::voice::VoiceStatus;

/// Voice routes.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/voice/config", get(get_voice).put(update_voice))
        .route("/voice/status", get(voice_status))
}

/// Combined configuration + runtime status payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfigResponse {
    /// Persisted voice configuration.
    pub config: VoiceConfig,
    /// Runtime status (listening window, activations).
    pub status: VoiceStatus,
}

async fn get_voice(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<VoiceConfigResponse>> {
    auth.require(Permission::AgentRead)?;
    Ok(Json(VoiceConfigResponse {
        config: state.voice.voice_config(),
        status: state.voice.status(),
    }))
}

async fn voice_status(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<VoiceStatus>> {
    auth.require(Permission::AgentRead)?;
    Ok(Json(state.voice.status()))
}

async fn update_voice(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(config): Json<VoiceConfig>,
) -> HttpResult<Json<VoiceConfigResponse>> {
    auth.require(Permission::AgentManage)?;
    config.validate().map_err(HttpError::from)?;
    let updated = state
        .config
        .update(|system| {
            system.voice = config.clone();
            Ok(())
        })
        .map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "voice.update",
        Some(&updated.voice.assistant_name),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({
            "assistant_name": updated.voice.assistant_name,
            "wake_word_enabled": updated.voice.wake_word_enabled,
            "activation_mode": updated.voice.activation_mode.id(),
            "sensitivity": updated.voice.sensitivity,
            "language": updated.voice.language,
        })),
    );
    // Close any open listening window so the old name cannot be used.
    state.voice.begin_query();
    state.kiosk.notify(
        hs_core::events::NotificationLevel::Info,
        format!(
            "Asistente de voz actualizado: «Hola {}»",
            updated.voice.assistant_name
        ),
    );
    Ok(Json(VoiceConfigResponse {
        config: updated.voice,
        status: state.voice.status(),
    }))
}
