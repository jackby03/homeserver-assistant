//! Admin WebSocket endpoints: live server events and setup progress.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use hs_auth::Permission;
use tokio::sync::broadcast::error::RecvError;
use tracing::debug;

use crate::error::HttpError;
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;

/// WebSocket routes.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/ws/events", get(events_socket))
        .route("/ws/setup", get(setup_socket))
}

async fn events_socket(
    State(state): State<SharedState>,
    auth: AdminAuth,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, HttpError> {
    auth.require(Permission::DashboardView)?;
    Ok(ws.on_upgrade(move |socket| stream_server_events(socket, state)))
}

async fn setup_socket(
    State(state): State<SharedState>,
    auth: AdminAuth,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, HttpError> {
    auth.require(Permission::DashboardView)?;
    Ok(ws.on_upgrade(move |socket| stream_server_events(socket, state)))
}

async fn stream_server_events(mut socket: WebSocket, state: SharedState) {
    let mut receiver = state.events.subscribe_server();
    loop {
        tokio::select! {
            message = socket.recv() => {
                match message {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    Some(Ok(Message::Text(text))) if text.as_str() == "ping" => {
                        let _ = socket.send(Message::Text("pong".into())).await;
                    }
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
                            Err(err) => debug!(error = %err, "failed to serialize server event"),
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        debug!(skipped, "admin event subscriber lagged");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }
}
