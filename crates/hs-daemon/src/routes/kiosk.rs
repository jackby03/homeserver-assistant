//! Wall kiosk API (loopback listener with token authentication).
//!
//! The kiosk UI is served from `http://127.0.0.1:3000/kiosk`. The page handler
//! sets an HttpOnly cookie with the per-install kiosk token; subsequent API and
//! WebSocket calls are authenticated with that cookie (or `X-Kiosk-Token` for
//! the Vite development server). The API only exposes kiosk-safe operations.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hs_core::events::{AgentActivity, AvatarState, MusicState};
use hs_core::models::{AgentChatRequest, ChatMessage, ChatRole, Profile};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tracing::debug;

use crate::cookies::{header_value, kiosk_cookie, KIOSK_COOKIE};
use crate::error::{HttpError, HttpResult};
use crate::state::SharedState;

/// Kiosk state response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KioskStateResponse {
    /// Device name.
    pub device_name: String,
    /// Active profile.
    pub profile: Option<Profile>,
    /// Current avatar state.
    pub avatar_state: AvatarState,
    /// Gaze point if someone is tracked.
    pub tracking: Option<hs_core::events::TrackingPoint>,
    /// Music state.
    pub music: MusicState,
    /// Network health.
    pub network: hs_core::events::NetworkStatus,
    /// Agent activity.
    pub agent_activity: AgentActivity,
    /// Home Assistant scenes (best effort).
    pub scenes: Vec<SceneSummary>,
}

/// Minimal scene descriptor for the kiosk grid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneSummary {
    /// Entity id (`scene.movie_night`).
    pub entity_id: String,
    /// Friendly name.
    pub name: String,
    /// Current state.
    pub state: String,
}

/// Profile activation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivateProfileBody {
    /// Profile to activate.
    pub profile_id: uuid::Uuid,
}

/// Listening indicator request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListeningBody {
    /// Whether the microphone is capturing.
    pub active: bool,
}

/// Agent query request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentQueryBody {
    /// User utterance (already transcribed by the browser).
    pub text: String,
}

/// Volume request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeBody {
    /// Volume level 0..=100.
    pub volume: u8,
}

/// Face recognition request produced locally by the optional InsightFace helper.
///
/// The embedding never leaves the appliance: the helper runs on the host, posts
/// the vector to the loopback kiosk listener and the daemon performs the cosine
/// match against encrypted enrollments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecognizeBody {
    /// Face embedding vector.
    pub embedding: Vec<f32>,
    /// Similarity threshold (defaults to 0.62, clamped to 0.30..=0.99).
    #[serde(default)]
    pub threshold: Option<f32>,
}

/// Build the kiosk listener router (UI + API).
pub fn router(state: SharedState) -> Router {
    let api = Router::new()
        .route("/kiosk/state", get(state_snapshot))
        .route("/kiosk/profile/activate", post(activate_profile))
        .route("/kiosk/listening", post(listening))
        .route("/kiosk/agent/query", post(agent_query))
        .route("/kiosk/music/play", post(music_play))
        .route("/kiosk/music/pause", post(music_pause))
        .route("/kiosk/music/volume", post(music_volume))
        .route("/kiosk/scenes/{entity_id}/activate", post(activate_scene))
        .route("/kiosk/recognize", post(recognize_face))
        .route("/kiosk/ws", get(kiosk_ws))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_kiosk_token,
        ));

    Router::new()
        .nest("/api/v1", api)
        .route("/kiosk", get(kiosk_page))
        .route("/", get(kiosk_page))
        .nest_service(
            "/assets",
            tower_http::services::ServeDir::new(state.daemon.static_dir.join("assets")),
        )
        .route("/favicon.ico", get(favicon))
        .fallback(get(kiosk_page))
        .with_state(state)
}

/// Serve the SPA shell and set the kiosk token cookie.
async fn kiosk_page(State(state): State<SharedState>) -> Response {
    let index = state.daemon.static_dir.join("index.html");
    let html = match tokio::fs::read_to_string(&index).await {
        Ok(html) => html,
        Err(_) => missing_ui_page(&state),
    };
    let mut response = Html(html).into_response();
    response.headers_mut().append(
        axum::http::header::SET_COOKIE,
        header_value(kiosk_cookie(&state.kiosk_token())),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

async fn favicon() -> Response {
    axum::http::StatusCode::NO_CONTENT.into_response()
}

fn missing_ui_page(state: &SharedState) -> String {
    format!(
        "<!doctype html><html><head><meta charset=utf-8><title>Kiosk</title></head>\
         <body style=\"background:#060a12;color:#cbd5e1;font-family:system-ui;display:grid;\
         place-items:center;height:100vh;margin:0\"><div style=\"text-align:center\">\
         <h1>HomeServer Assistant</h1><p>The UI bundle was not found.</p>\
         <p>Run <code>cd ui &amp;&amp; pnpm build</code> or start the Vite dev server.</p>\
         <p style=\"opacity:.6\">Expected at {}</p></div></body></html>",
        state.daemon.static_dir.display()
    )
}

/// Reject kiosk API calls without the per-install token.
async fn require_kiosk_token(
    State(state): State<SharedState>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, HttpError> {
    let expected = state.kiosk_token();
    let provided = headers
        .get("x-kiosk-token")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
        .or_else(|| cookie_value(&headers, KIOSK_COOKIE));

    let Some(provided) = provided else {
        return Err(HttpError::unauthorized("kiosk token is required"));
    };
    if expected.is_empty() || !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        return Err(HttpError::unauthorized("invalid kiosk token"));
    }
    Ok(next.run(request).await)
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())?;
    for cookie in cookie_header.split(';') {
        if let Some((cookie_name, value)) = cookie.trim().split_once('=') {
            if cookie_name == name && !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

async fn build_scene_list(state: &SharedState) -> Vec<SceneSummary> {
    match state.ha.entities().await {
        Ok(entities) => entities
            .into_iter()
            .filter(|entity| entity.domain == "scene")
            .map(|entity| SceneSummary {
                entity_id: entity.entity_id,
                name: entity.name,
                state: entity.state,
            })
            .collect(),
        Err(err) => {
            debug!(error = %err, "scenes unavailable");
            Vec::new()
        }
    }
}

async fn state_snapshot(State(state): State<SharedState>) -> Json<KioskStateResponse> {
    let snapshot = state.kiosk.snapshot();
    let scenes = build_scene_list(&state).await;
    Json(KioskStateResponse {
        device_name: snapshot.device_name,
        profile: snapshot.profile,
        avatar_state: snapshot.avatar_state,
        tracking: snapshot.tracking,
        music: snapshot.music,
        network: snapshot.network,
        agent_activity: snapshot.agent_activity,
        scenes,
    })
}

async fn activate_profile(
    State(state): State<SharedState>,
    Json(body): Json<ActivateProfileBody>,
) -> HttpResult<Json<Profile>> {
    let profile = state
        .profiles
        .get(body.profile_id)
        .map_err(HttpError::from)?;
    state.kiosk.activate_profile(profile.clone());
    state.audit(
        "kiosk",
        "kiosk.profile_activate",
        Some(&profile.id.to_string()),
        hs_core::models::AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(profile))
}

async fn listening(
    State(state): State<SharedState>,
    Json(body): Json<ListeningBody>,
) -> Json<serde_json::Value> {
    state.kiosk.set_avatar_state(if body.active {
        AvatarState::Listening
    } else {
        AvatarState::Idle
    });
    state.kiosk.set_agent_activity(if body.active {
        AgentActivity::Listening
    } else {
        AgentActivity::Idle
    });
    Json(serde_json::json!({"ok": true}))
}

async fn agent_query(
    State(state): State<SharedState>,
    Json(body): Json<AgentQueryBody>,
) -> HttpResult<Json<hs_core::models::AgentChatResponse>> {
    let text = body.text.trim();
    if text.is_empty() {
        return Err(HttpError::bad_request("text is required"));
    }
    let snapshot = state.kiosk.snapshot();
    let Some(profile) = snapshot.profile else {
        return Err(HttpError::conflict(
            "no profile is active; select a profile before talking to the assistant",
        ));
    };

    state.kiosk.set_avatar_state(AvatarState::Thinking);
    state.kiosk.set_agent_activity(AgentActivity::Thinking);

    let request = AgentChatRequest {
        profile_id: profile.id,
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: text.to_owned(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }],
        use_tools: true,
    };

    let result = state.agent.chat(request).await;
    match result {
        Ok(response) => {
            state.kiosk.set_agent_activity(AgentActivity::Idle);
            state.kiosk.set_avatar_state(AvatarState::Speaking);
            state.kiosk.set_avatar_state(AvatarState::Idle);
            state.audit(
                "kiosk",
                "kiosk.agent_query",
                Some(&profile.id.to_string()),
                hs_core::models::AuditOutcome::Success,
                None,
                Some(serde_json::json!({
                    "tools": response.tool_trace.len(),
                    "model": response.model,
                })),
            );
            Ok(Json(response))
        }
        Err(err) => {
            state.kiosk.set_agent_activity(AgentActivity::Error);
            state.kiosk.set_avatar_state(AvatarState::Idle);
            state.audit(
                "kiosk",
                "kiosk.agent_query",
                Some(&profile.id.to_string()),
                hs_core::models::AuditOutcome::Failure,
                None,
                Some(serde_json::json!({"error": err.to_string()})),
            );
            Err(HttpError::from(err))
        }
    }
}

async fn first_media_player(state: &SharedState) -> Option<String> {
    let entities = state.ha.entities().await.ok()?;
    entities
        .into_iter()
        .find(|entity| entity.domain == "media_player")
        .map(|entity| entity.entity_id)
}

async fn music_play(State(state): State<SharedState>) -> HttpResult<Json<serde_json::Value>> {
    let Some(entity_id) = first_media_player(&state).await else {
        return Err(HttpError::unavailable(
            "no Home Assistant media player is available; configure Home Assistant or Music Assistant first",
        ));
    };
    state
        .ha
        .call_service(
            "media_player",
            "media_play",
            serde_json::json!({"entity_id": entity_id}),
        )
        .await
        .map_err(map_ha_error)?;
    let mut music = state.kiosk.snapshot().music;
    music.playing = true;
    state.kiosk.set_music(music);
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn music_pause(State(state): State<SharedState>) -> HttpResult<Json<serde_json::Value>> {
    let Some(entity_id) = first_media_player(&state).await else {
        return Err(HttpError::unavailable(
            "no Home Assistant media player is available",
        ));
    };
    state
        .ha
        .call_service(
            "media_player",
            "media_pause",
            serde_json::json!({"entity_id": entity_id}),
        )
        .await
        .map_err(map_ha_error)?;
    let mut music = state.kiosk.snapshot().music;
    music.playing = false;
    state.kiosk.set_music(music);
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn music_volume(
    State(state): State<SharedState>,
    Json(body): Json<VolumeBody>,
) -> HttpResult<Json<serde_json::Value>> {
    if body.volume > 100 {
        return Err(HttpError::bad_request("volume must be between 0 and 100"));
    }
    state
        .system
        .set_volume(body.volume)
        .map_err(HttpError::from)?;
    let mut music = state.kiosk.snapshot().music;
    music.volume = body.volume;
    state.kiosk.set_music(music);
    Ok(Json(serde_json::json!({"ok": true, "volume": body.volume})))
}

async fn activate_scene(
    State(state): State<SharedState>,
    Path(entity_id): Path<String>,
) -> HttpResult<Json<serde_json::Value>> {
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

/// `POST /api/v1/kiosk/recognize`
///
/// Matches a locally computed face embedding against enrolled profiles and
/// activates the best match above the threshold.
async fn recognize_face(
    State(state): State<SharedState>,
    Json(body): Json<RecognizeBody>,
) -> HttpResult<Json<serde_json::Value>> {
    let threshold = body.threshold.unwrap_or(0.62).clamp(0.30, 0.99);
    let matched = state
        .profiles
        .match_face(&body.embedding, threshold)
        .map_err(HttpError::from)?;
    match matched {
        Some(profile) => {
            state.kiosk.activate_profile(profile.clone());
            state.audit(
                "kiosk",
                "kiosk.face_recognized",
                Some(&profile.id.to_string()),
                hs_core::models::AuditOutcome::Success,
                None,
                Some(serde_json::json!({"threshold": threshold})),
            );
            Ok(Json(serde_json::json!({
                "matched": true,
                "profile": profile,
            })))
        }
        None => Ok(Json(serde_json::json!({
            "matched": false,
            "profile": null,
        }))),
    }
}

/// Kiosk WebSocket: streams `KioskEvent` JSON objects.
async fn kiosk_ws(State(state): State<SharedState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| kiosk_socket(socket, state))
}

async fn kiosk_socket(mut socket: WebSocket, state: SharedState) {
    let mut receiver = state.events.subscribe_kiosk();
    // Send the snapshot first so a reconnecting client doesn't wait for a change.
    let snapshot = state.kiosk.snapshot();
    let initial = serde_json::json!({
        "type": "snapshot",
        "device_name": snapshot.device_name,
        "profile": snapshot.profile,
        "avatar_state": snapshot.avatar_state,
        "music": snapshot.music,
        "network": snapshot.network,
        "agent_activity": snapshot.agent_activity,
    });
    if socket
        .send(Message::Text(initial.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            message = socket.recv() => {
                match message {
                    Some(Ok(Message::Text(text))) if text.as_str() == "ping" => {
                        let _ = socket.send(Message::Text("pong".into())).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
            event = receiver.recv() => {
                match event {
                    Ok(event) => {
                        match serde_json::to_string(&event) {
                            Ok(payload) => {
                                if socket.send(Message::Text(payload.into())).await.is_err() {
                                    break;
                                }
                            }
                            Err(err) => debug!(error = %err, "failed to serialize kiosk event"),
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        debug!(skipped, "kiosk event subscriber lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
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
