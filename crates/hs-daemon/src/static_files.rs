//! Static file serving for the admin SPA.
//!
//! The UI is built with Vite into `ui/dist`. Hashed assets live under
//! `/assets`; every other non-API path returns `index.html` so client-side
//! routing works after a refresh. When the bundle is missing (development), a
//! helpful placeholder is served instead of a 404.

use axum::extract::State;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tower_http::services::ServeDir;

use crate::state::SharedState;

/// Static asset + SPA fallback router.
pub fn spa_router(state: SharedState) -> Router<SharedState> {
    Router::new()
        .nest_service(
            "/assets",
            ServeDir::new(state.daemon.static_dir.join("assets")),
        )
        .route("/", get(index))
        .fallback(get(index))
}

async fn index(State(state): State<SharedState>) -> Response {
    let path = state.daemon.static_dir.join("index.html");
    match tokio::fs::read_to_string(&path).await {
        Ok(html) => Html(html).into_response(),
        Err(_) => missing_ui(&state).into_response(),
    }
}

fn missing_ui(state: &SharedState) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>HomeServer Assistant</title></head>\
         <body style=\"background:#060a12;color:#cbd5e1;font-family:system-ui;display:grid;\
         place-items:center;height:100vh;margin:0\"><main style=\"text-align:center;max-width:34rem\">\
         <h1 style=\"font-weight:600\">HomeServer Assistant</h1>\
         <p>The administration UI bundle was not found.</p>\
         <p>Build it with <code>cd ui &amp;&amp; pnpm install &amp;&amp; pnpm build</code> \
         or run the Vite dev server and use its proxy.</p>\
         <p style=\"opacity:.55\">Expected directory: {}</p>\
         <p style=\"opacity:.55\">API health: <a style=\"color:#6ee7ff\" \
         href=\"/api/v1/health\">/api/v1/health</a></p>\
         </main></body></html>",
        state.daemon.static_dir.display()
    )
}
