//! `hs-daemon` — HomeServer Assistant privileged daemon.
//!
//! This crate is the **composition root** of the appliance: it loads the
//! configuration and secrets, builds the concrete adapters (Incus runtime,
//! system hardware, Home Assistant client, LLM providers), exposes the admin
//! and kiosk HTTP/WebSocket APIs and runs the background tasks (metrics,
//! setup deployment, vision bridge).
//!
//! The binary is `hs-daemon`; the library exposes the app state and routers so
//! integration tests can exercise them without binding sockets.

pub mod adapters;
pub mod assistant;
pub mod bootstrap;
pub mod caddy;
pub mod config;
pub mod cookies;
pub mod dto;
pub mod error;
pub mod events;
pub mod kiosk;
pub mod memory;
pub mod middleware;
pub mod news;
pub mod router;
pub mod routes;
pub mod setup;
pub mod state;
pub mod static_files;
pub mod tasks;
pub mod timers;
pub mod vision_bridge;
pub mod voice;
pub mod weather;

pub use config::DaemonConfig;
pub use error::{HttpError, HttpResult};
pub use events::{EventBus, ServerEvent};
pub use kiosk::KioskManager;
pub use setup::SetupCoordinator;
pub use state::{AppState, SharedState};
pub use voice::{VoiceManager, VoiceStatus, WakeOutcome};
