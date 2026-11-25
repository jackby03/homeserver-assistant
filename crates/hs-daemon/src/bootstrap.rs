//! Application bootstrap: load configuration and secrets, build adapters and
//! wire the shared state (composition root).

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use hs_agent::{AgentService, DefaultFactory};
use hs_apps::AppService;
use hs_auth::{AuthConfig, AuthService, JwtService};
use hs_core::state::{ConfigManager, Database, SecretStore, SecretVault};
use hs_discovery::DiscoveryService;
use hs_homeassistant::{HaClient, HaService, ReqwestTransport};
use hs_profiles::ProfileService;
use hs_runtime::incus::IncusProvider;
use hs_runtime::mock::MockRuntime;
use hs_runtime::RuntimeProvider;
use hs_system::{FakeHardware, Hardware, LinuxHardware, SystemService};
use hs_tools::ToolRegistry;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::adapters;
use crate::config::DaemonConfig;
use crate::events::EventBus;
use crate::kiosk::KioskManager;
use crate::setup::SetupCoordinator;
use crate::state::{AppState, SharedState};

/// Secret name used to persist the JWT signing secret in the vault.
const JWT_SECRET_NAME: &str = "auth.jwt_secret";
/// Home Assistant long-lived token secret name.
const HA_TOKEN_NAME: &str = "homeassistant.token";

/// Build the fully wired application state.
///
/// # Errors
/// Returns an error when the data directory, database, vault or services cannot
/// be initialized. The caller (binary) is expected to log and exit.
pub async fn build_state(daemon: DaemonConfig) -> Result<SharedState> {
    daemon
        .ensure_layout()
        .with_context(|| format!("cannot create data directory {}", daemon.data_dir.display()))?;

    let config_manager = Arc::new(
        ConfigManager::load(&daemon.data_dir).context("failed to load system configuration")?,
    );
    let config = config_manager.snapshot();

    let db = Arc::new(
        Database::open(&daemon.database_path()).context("failed to open the embedded database")?,
    );
    let vault = SecretVault::load_or_create(&daemon.master_key_path())
        .context("failed to load or create the master key")?;
    let secrets = Arc::new(SecretStore::new(Arc::clone(&db), vault));

    let jwt_secret = load_or_create_jwt_secret(&secrets)?;
    let auth = Arc::new(AuthService::new(
        Arc::clone(&db),
        JwtService::new(jwt_secret, "homecore-daemon"),
        AuthConfig::with_session_hours(config.security.session_ttl_hours),
    ));

    let (runtime, fake_runtime): (Arc<dyn RuntimeProvider>, bool) =
        if daemon.dev_mode || !cfg!(unix) {
            info!("using in-memory mock runtime with demo instances (development mode)");
            let mock = MockRuntime::with_demo_instances();
            (Arc::new(mock), true)
        } else {
            let provider = IncusProvider::new("/var/lib/incus/unix.socket").with_bridges(
                config.network.lan_bridge.clone(),
                config.network.dmz.bridge.clone(),
            );
            let provider: Arc<dyn RuntimeProvider> = Arc::new(provider);
            if provider.available() {
                info!("Incus runtime provider is available");
            } else {
                warn!("Incus socket is not available; instance endpoints will report 503");
            }
            (provider, false)
        };

    let hardware: Arc<dyn Hardware> = if daemon.dev_mode {
        info!("using fake hardware adapter (development mode)");
        Arc::new(FakeHardware::linux())
    } else {
        Arc::new(LinuxHardware::new())
    };
    let system = Arc::new(SystemService::with_hardware(hardware));

    let apps = Arc::new(AppService::new(
        Arc::clone(&runtime),
        Arc::clone(&db),
        config.services.base_domain.clone(),
    ));

    let discovery = Arc::new(DiscoveryService::default_sources());

    let ha = Arc::new(HaService::new());
    if let Some(url) = &config.services.home_assistant_url {
        if let Some(token) = secrets
            .get(HA_TOKEN_NAME)
            .context("failed to read HA token")?
        {
            let transport = Arc::new(ReqwestTransport::new(url, token.as_str()));
            ha.configure(Arc::new(HaClient::new(transport))).await;
            info!(url = %url, "Home Assistant client configured");
        }
    }

    let profiles = Arc::new(ProfileService::new(Arc::clone(&db), Arc::clone(&secrets)));

    let events = EventBus::default();
    let timers = crate::timers::TimerService::new(&daemon.data_dir, events.clone());
    timers.start();

    let ports = adapters::build_ports(
        Arc::clone(&runtime),
        Arc::clone(&system),
        Arc::clone(&ha),
        &daemon.data_dir,
        timers,
    );
    let tools = ToolRegistry::builtin(ports);
    let factory = Arc::new(DefaultFactory::default());
    let agent = Arc::new(AgentService::new(
        Arc::clone(&db),
        Arc::clone(&secrets),
        Arc::clone(&profiles),
        Arc::clone(&tools),
        factory,
        config.agent.temperature,
        config.agent.max_tool_steps,
    ));

    let kiosk = Arc::new(KioskManager::new(
        events.clone(),
        config.device.name.clone(),
    ));
    let voice = Arc::new(crate::voice::VoiceManager::new(
        Arc::clone(&config_manager),
        Arc::clone(&kiosk),
    ));
    let setup = Arc::new(SetupCoordinator::new(events.clone()));

    let state = Arc::new(AppState {
        daemon: daemon.clone(),
        config: config_manager,
        db,
        secrets,
        auth,
        system,
        runtime,
        apps,
        discovery,
        ha,
        profiles,
        tools,
        agent,
        kiosk,
        voice,
        events,
        discovery_devices: RwLock::new(Vec::new()),
        setup,
        started_at: chrono::Utc::now(),
        fake_runtime,
    });

    info!(
        data_dir = %daemon.data_dir.display(),
        admin = %daemon.admin_bind,
        kiosk = %daemon.kiosk_bind,
        initialized = state.is_initialized(),
        "application state initialized"
    );

    // The active profile is runtime state; without this the kiosk would show
    // "no active profile" after every daemon restart until a human switches it.
    if let Ok(profiles) = state.profiles.list() {
        if let Some(profile) = profiles
            .iter()
            .find(|profile| profile.role == hs_core::models::ProfileRole::Owner)
            .or_else(|| profiles.first())
        {
            state.kiosk.activate_profile(profile.clone());
            info!(profile = %profile.name, "default profile activated");
        }
    }

    Ok(state)
}

fn load_or_create_jwt_secret(secrets: &SecretStore) -> Result<[u8; 32]> {
    if let Some(existing) = secrets
        .get(JWT_SECRET_NAME)
        .context("failed to read the JWT secret")?
    {
        let bytes = hex::decode(existing.as_str())
            .map_err(|_| anyhow!("stored JWT secret is not valid hex"))?;
        if bytes.len() != 32 {
            return Err(anyhow!(
                "stored JWT secret has invalid length {} (expected 32)",
                bytes.len()
            ));
        }
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&bytes);
        return Ok(secret);
    }

    let secret = JwtService::generate_secret();
    secrets
        .set(JWT_SECRET_NAME, &hex::encode(secret))
        .context("failed to persist a new JWT secret")?;
    info!("generated a new JWT signing secret");
    Ok(secret)
}
