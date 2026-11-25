//! Router composition for the two listeners.
//!
//! - [`admin_router`] is mounted behind Caddy on `:8443` (loopback `:8080`).
//! - [`kiosk_router`] is the wall tablet listener on `127.0.0.1:3000`.

use axum::Router;

use crate::routes;
use crate::state::SharedState;
use crate::static_files;

/// Admin portal API + SPA.
pub fn admin_router(state: SharedState) -> Router {
    let api = Router::new()
        .merge(routes::public::router())
        .merge(routes::auth::router())
        .merge(routes::system::router())
        .merge(routes::audit::router())
        .merge(routes::users::router())
        .merge(routes::instances::router())
        .merge(routes::apps::router())
        .merge(routes::network::router())
        .merge(routes::discovery::router())
        .merge(routes::homeassistant::router())
        .merge(routes::agent::router())
        .merge(routes::profiles::router())
        .merge(routes::ws::router())
        .merge(routes::setup::router(state.clone()));

    Router::new()
        .nest("/api/v1", api)
        .merge(static_files::spa_router(state.clone()))
        .with_state(state)
}

/// Wall kiosk UI + API.
pub fn kiosk_router(state: SharedState) -> Router {
    routes::kiosk::router(state)
}
