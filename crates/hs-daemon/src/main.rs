//! `hs-daemon` binary: CLI entry point.
//!
//! Commands:
//! - `serve` (default runtime): start the admin API, the kiosk listener and the
//!   background tasks.
//! - `doctor`: check the installation (paths, database, vault, admin account).
//! - `reset-admin`: local recovery command to reset an administrator password.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand};
use hs_core::models::Role;
use hs_core::state::{ConfigManager, Database, SecretVault};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

/// HomeServer Assistant privileged daemon.
#[derive(Debug, Parser)]
#[command(
    name = "hs-daemon",
    version,
    about = "HomeServer Assistant OS privileged daemon",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the daemon (admin API, kiosk listener and background tasks).
    Serve(ServeArgs),
    /// Check the appliance installation.
    Doctor(DoctorArgs),
    /// Reset an administrator password from the local console.
    ResetAdmin(ResetAdminArgs),
}

#[derive(Debug, Clone, Args)]
struct ServeArgs {
    /// Data directory (config, database, master key).
    #[arg(long, env = "HS_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Admin API bind address (loopback; Caddy terminates TLS).
    #[arg(long, env = "HS_ADMIN_BIND")]
    bind: Option<SocketAddr>,
    /// Kiosk bind address (loopback only).
    #[arg(long, env = "HS_KIOSK_BIND")]
    kiosk_bind: Option<SocketAddr>,
    /// Directory containing the built UI (`ui/dist`).
    #[arg(long, env = "HS_STATIC_DIR")]
    static_dir: Option<PathBuf>,
    /// Vision service Unix socket.
    #[arg(long, env = "HS_VISION_SOCKET")]
    vision_socket: Option<PathBuf>,
    /// Use fake runtime/hardware (no root, no Incus, no sysfs).
    #[arg(long)]
    dev: bool,
    /// Feed synthetic tracking events to the kiosk.
    #[arg(long)]
    fake_vision: bool,
    /// Emit JSON logs (reserved; human readable logs are used today).
    #[arg(long)]
    log_json: bool,
}

impl ServeArgs {
    fn into_config(self) -> Result<hs_daemon::DaemonConfig> {
        hs_daemon::DaemonConfig::resolve(
            self.data_dir,
            self.bind,
            self.kiosk_bind,
            self.static_dir,
            self.vision_socket,
            self.dev,
            self.fake_vision,
            self.log_json,
        )
        .map_err(|err| anyhow!(err))
    }
}

#[derive(Debug, Clone, Args)]
struct DoctorArgs {
    /// Data directory to inspect.
    #[arg(long, env = "HS_DATA_DIR")]
    data_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
struct ResetAdminArgs {
    /// Data directory to inspect.
    #[arg(long, env = "HS_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Administrator username to reset.
    #[arg(long, default_value = "admin")]
    username: String,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Serve(args) => run_serve(args).await,
        Command::Doctor(args) => run_doctor(args),
        Command::ResetAdmin(args) => run_reset_admin(args),
    };
    if let Err(err) = result {
        error!(error = %err, "hs-daemon failed");
        eprintln!("hs-daemon: error: {err:#}");
        std::process::exit(1);
    }
}

async fn run_serve(args: ServeArgs) -> Result<()> {
    init_tracing();
    let daemon = args.into_config()?;
    info!(
        version = env!("CARGO_PKG_VERSION"),
        data_dir = %daemon.data_dir.display(),
        dev_mode = daemon.dev_mode,
        "starting HomeServer Assistant daemon"
    );

    let state = hs_daemon::bootstrap::build_state(daemon.clone()).await?;
    hs_daemon::tasks::spawn_all(Arc::clone(&state));
    if let Err(err) = hs_daemon::caddy::sync_global(&state) {
        warn!(error = %err, "failed to write the Caddy environment file");
    }

    if !state.is_initialized() {
        info!("appliance is not initialized: first-boot onboarding is required");
    }

    let admin_router = hs_daemon::router::admin_router(Arc::clone(&state));
    let kiosk_router = hs_daemon::router::kiosk_router(Arc::clone(&state));

    let admin_listener = tokio::net::TcpListener::bind(daemon.admin_bind)
        .await
        .with_context(|| format!("cannot bind admin API on {}", daemon.admin_bind))?;
    let kiosk_listener = tokio::net::TcpListener::bind(daemon.kiosk_bind)
        .await
        .with_context(|| format!("cannot bind kiosk on {}", daemon.kiosk_bind))?;

    info!(address = %daemon.admin_bind, "admin API listening (behind Caddy TLS)");
    info!(address = %daemon.kiosk_bind, "kiosk listening (loopback only)");
    println!(
        "hs-daemon ready | admin: http://{} | kiosk: http://{}/kiosk | data: {}",
        daemon.admin_bind,
        daemon.kiosk_bind,
        daemon.data_dir.display()
    );

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            info!("shutdown signal received");
        }
        let _ = shutdown_tx.send(true);
    });

    let admin_shutdown = shutdown_future(shutdown_rx.clone());
    let kiosk_shutdown = shutdown_future(shutdown_rx);

    let admin = axum::serve(admin_listener, admin_router).with_graceful_shutdown(admin_shutdown);
    let kiosk = axum::serve(kiosk_listener, kiosk_router).with_graceful_shutdown(kiosk_shutdown);

    tokio::try_join!(admin, kiosk).context("HTTP server stopped unexpectedly")?;
    info!("daemon stopped");
    Ok(())
}

async fn shutdown_future(mut receiver: tokio::sync::watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }
    while receiver.changed().await.is_ok() {
        if *receiver.borrow() {
            return;
        }
    }
}

fn run_doctor(args: DoctorArgs) -> Result<()> {
    let data_dir = args
        .data_dir
        .or_else(|| std::env::var_os(hs_core::DATA_DIR_ENV).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(hs_core::DEFAULT_DATA_DIR));

    println!("hs-daemon doctor");
    println!("  data directory : {}", data_dir.display());

    let mut failures = 0;
    let mut checks = 0;

    checks += 1;
    if data_dir.is_dir() {
        println!("  [ OK ] data directory exists");
    } else {
        println!("  [FAIL] data directory is missing");
        failures += 1;
    }

    checks += 1;
    match ConfigManager::load(&data_dir) {
        Ok(manager) => {
            let config = manager.snapshot();
            println!(
                "  [ OK ] configuration loaded (initialized={}, device=\"{}\")",
                config.initialized, config.device.name
            );
        }
        Err(err) => {
            println!("  [FAIL] configuration cannot be loaded: {err}");
            failures += 1;
        }
    }

    checks += 1;
    match Database::open(&data_dir.join("state.redb")) {
        Ok(db) => {
            println!("  [ OK ] embedded database opens");
            checks += 1;
            match db.list_users() {
                Ok(users) => {
                    let admins = users
                        .iter()
                        .filter(|user| user.role == Role::Admin && !user.disabled)
                        .count();
                    println!(
                        "  [ OK ] {} user(s), {} active administrator(s)",
                        users.len(),
                        admins
                    );
                }
                Err(err) => {
                    println!("  [FAIL] cannot list users: {err}");
                    failures += 1;
                }
            }
        }
        Err(err) => {
            println!("  [FAIL] embedded database cannot be opened: {err}");
            failures += 1;
        }
    }

    checks += 1;
    let key_path = data_dir.join(hs_core::state::vault::MASTER_KEY_FILE);
    match SecretVault::load_or_create(&key_path) {
        Ok(_) => println!("  [ OK ] secret vault master key is readable"),
        Err(err) => {
            println!("  [FAIL] secret vault master key problem: {err}");
            failures += 1;
        }
    }

    checks += 1;
    if cfg!(unix) {
        if std::path::Path::new("/var/lib/incus/unix.socket").exists() {
            println!("  [ OK ] Incus socket found");
        } else {
            println!("  [WARN] Incus socket not found (instance endpoints will be unavailable)");
        }
    } else {
        println!("  [INFO] not running on Linux; Incus checks skipped");
    }

    println!(
        "doctor finished: {checks} checks, {failures} failure(s){}",
        if failures == 0 {
            ""
        } else {
            " — review the failures above"
        }
    );
    if failures > 0 {
        return Err(anyhow!("{failures} check(s) failed"));
    }
    Ok(())
}

fn run_reset_admin(args: ResetAdminArgs) -> Result<()> {
    use rand::Rng;
    use uuid::Uuid;

    let data_dir = args
        .data_dir
        .or_else(|| std::env::var_os(hs_core::DATA_DIR_ENV).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(hs_core::DEFAULT_DATA_DIR));
    let db = Database::open(&data_dir.join("state.redb"))
        .context("failed to open the embedded database")?;

    let mut user = db
        .user_by_username(&args.username)
        .with_context(|| format!("failed to look up user `{}`", args.username))?
        .ok_or_else(|| anyhow!("user `{}` does not exist", args.username))?;

    if user.role != Role::Admin {
        return Err(anyhow!(
            "user `{}` is not an administrator; refusing to reset",
            user.username
        ));
    }

    // Generate a one-time password: 20 characters, no ambiguous glyphs.
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%&*";
    let mut rng = rand::thread_rng();
    let password: String = (0..20)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect();

    user.password_hash = hs_auth::password::hash_password(&password)
        .map_err(|err| anyhow!("failed to hash password: {err}"))?;
    user.updated_at = chrono::Utc::now();
    db.save_user(&user).context("failed to save the user")?;

    // Revoke every session for the user.
    let sessions = db
        .list::<hs_core::models::SessionRecord>(hs_core::state::Table::Sessions)
        .context("failed to list sessions")?;
    let mut revoked = 0;
    for session in sessions {
        if session.user_id == user.id {
            db.delete_session(session.id).ok();
            revoked += 1;
        }
    }

    println!("hs-daemon reset-admin");
    println!("  user            : {}", user.username);
    println!("  sessions revoked: {revoked}");
    println!("  one-time password: {password}");
    println!("  change it immediately after logging in to the portal.");
    info!(user = %user.username, "administrator password reset from console");
    let _ = Uuid::nil();
    Ok(())
}

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,hs_daemon=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_names(false)
        .try_init();
    if std::env::var("HS_LOG_JSON").is_ok() {
        warn!("HS_LOG_JSON is not implemented; human readable logs are used");
    }
}
