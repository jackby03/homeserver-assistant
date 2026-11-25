//! Shared application state for the HTTP/WebSocket layer.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use hs_agent::AgentService;
use hs_apps::AppService;
use hs_auth::AuthService;
use hs_core::state::{ConfigManager, Database, SecretStore};
use hs_discovery::DiscoveryService;
use hs_homeassistant::HaService;
use hs_profiles::ProfileService;
use hs_runtime::RuntimeProvider;
use hs_system::SystemService;
use hs_tools::ToolRegistry;

use crate::config::DaemonConfig;
use crate::events::EventBus;
use crate::kiosk::KioskManager;
use crate::setup::SetupCoordinator;

/// Shared state passed to every handler.
///
/// All fields are behind `Arc`/`Clone` handles; handlers never mutate the
/// struct itself. Services are the only owners of mutable domain state.
pub struct AppState {
    /// Resolved runtime configuration (paths, binds, dev flags).
    pub daemon: DaemonConfig,
    /// Validated appliance configuration (device, network, DNS, agent, ...).
    pub config: Arc<ConfigManager>,
    /// Embedded database (users, sessions, profiles, apps, audit, secrets).
    pub db: Arc<Database>,
    /// Secret vault facade.
    pub secrets: Arc<SecretStore>,
    /// Authentication and authorization service.
    pub auth: Arc<AuthService>,
    /// Host metrics and hardware control.
    pub system: Arc<SystemService>,
    /// Container/VM provider (Incus in production, mock in dev).
    pub runtime: Arc<dyn RuntimeProvider>,
    /// Application catalog and deployment engine.
    pub apps: Arc<AppService>,
    /// Local IoT discovery.
    pub discovery: Arc<DiscoveryService>,
    /// Home Assistant integration.
    pub ha: Arc<HaService>,
    /// Household profiles and biometrics.
    pub profiles: Arc<ProfileService>,
    /// Agentic tool registry (permission enforcement).
    pub tools: Arc<ToolRegistry>,
    /// LLM agent loop.
    pub agent: Arc<AgentService>,
    /// Kiosk state machine and event bus.
    pub kiosk: Arc<KioskManager>,
    /// Always-on voice assistant (wake-word gate and listening window).
    pub voice: Arc<crate::voice::VoiceManager>,
    /// Admin portal broadcast bus (same bus instance as `kiosk`).
    pub events: EventBus,
    /// Last discovery scan result (merged devices).
    pub discovery_devices: tokio::sync::RwLock<Vec<hs_core::models::DiscoveredDevice>>,
    /// First-boot setup progress.
    pub setup: Arc<SetupCoordinator>,
    /// Daemon start timestamp (uptime in the portal).
    pub started_at: DateTime<Utc>,
    /// Whether the runtime provider is a mock (development mode).
    pub fake_runtime: bool,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("data_dir", &self.daemon.data_dir)
            .field("admin_bind", &self.daemon.admin_bind)
            .field("kiosk_bind", &self.daemon.kiosk_bind)
            .field("dev_mode", &self.daemon.dev_mode)
            .field("fake_runtime", &self.fake_runtime)
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Whether first boot has completed.
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.config.snapshot().initialized
    }

    /// Current configuration snapshot.
    #[must_use]
    pub fn config_snapshot(&self) -> hs_core::config::SystemConfig {
        self.config.snapshot()
    }

    /// Kiosk token used by the loopback kiosk listener.
    #[must_use]
    pub fn kiosk_token(&self) -> String {
        self.config.snapshot().kiosk.token
    }

    /// Device display name.
    #[must_use]
    pub fn device_name(&self) -> String {
        self.config.snapshot().device.name
    }

    /// Publish an audit entry, ignoring storage failures but logging them.
    pub fn audit(
        &self,
        actor: &str,
        action: &str,
        target: Option<&str>,
        outcome: hs_core::models::AuditOutcome,
        ip: Option<&str>,
        details: Option<serde_json::Value>,
    ) {
        if let Err(err) = self.auth.audit(actor, action, target, outcome, ip, details) {
            tracing::error!(error = %err, action, "failed to append audit entry");
        }
    }
}

/// Convenient alias used by handlers.
pub type SharedState = Arc<AppState>;
