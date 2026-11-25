//! Daemon runtime configuration (paths, bind addresses, development flags).
//!
//! This is deliberately separate from [`hs_core::config::SystemConfig`]: the
//! system configuration is user-facing and stored in the data directory, while
//! `DaemonConfig` is deployment/runtime wiring supplied by systemd or CLI flags.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use hs_core::state::fsutil;

/// Default loopback address for the admin API (TLS is terminated by Caddy).
pub const DEFAULT_ADMIN_BIND: &str = "127.0.0.1:8080";
/// Default loopback address for the wall kiosk UI/API.
pub const DEFAULT_KIOSK_BIND: &str = "127.0.0.1:3000";
/// Default Unix socket where the vision service pushes tracking events.
pub const DEFAULT_VISION_SOCKET: &str = "/run/homecore/vision.sock";
/// Environment variable for the data directory.
pub const DATA_DIR_ENV: &str = "HS_DATA_DIR";
/// Environment variable for the admin bind address.
pub const ADMIN_BIND_ENV: &str = "HS_ADMIN_BIND";
/// Environment variable for the kiosk bind address.
pub const KIOSK_BIND_ENV: &str = "HS_KIOSK_BIND";
/// Environment variable for the built UI directory.
pub const STATIC_DIR_ENV: &str = "HS_STATIC_DIR";
/// Environment variable for the vision socket path.
pub const VISION_SOCKET_ENV: &str = "HS_VISION_SOCKET";
/// Environment variable for the Caddy per-app snippet directory.
pub const CADDY_APPS_DIR_ENV: &str = "HS_CADDY_APPS_DIR";
/// Environment variable for the Caddy environment file.
pub const CADDY_ENV_FILE_ENV: &str = "HS_CADDY_ENV_FILE";
/// Default directory where the daemon writes per-app Caddy sites.
pub const DEFAULT_CADDY_APPS_DIR: &str = "/etc/caddy/apps.d";
/// Default Caddy environment file (hostname/base domain/admin port).
pub const DEFAULT_CADDY_ENV_FILE: &str = "/etc/homecore/caddy.env";

/// Resolved daemon configuration.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Directory holding `config.json`, `state.redb` and `master.key`.
    pub data_dir: PathBuf,
    /// Admin API bind address (loopback in production).
    pub admin_bind: SocketAddr,
    /// Kiosk UI/API bind address (loopback only).
    pub kiosk_bind: SocketAddr,
    /// Directory with the built React UI (`ui/dist`).
    pub static_dir: PathBuf,
    /// Vision service Unix socket.
    pub vision_socket: PathBuf,
    /// Directory where per-app Caddy site snippets are written.
    pub caddy_apps_dir: PathBuf,
    /// Caddy environment file (hostname, base domain, admin port).
    pub caddy_env_file: PathBuf,
    /// Use fakes for the runtime and hardware (no Incus/sysfs required).
    pub dev_mode: bool,
    /// Feed the kiosk with synthetic tracking events (UI development).
    pub fake_vision: bool,
    /// Emit JSON logs instead of human readable logs.
    pub log_json: bool,
}

impl DaemonConfig {
    /// Resolve configuration from CLI overrides, environment variables and
    /// defaults.
    ///
    /// The eight parameters mirror the CLI flags one-to-one; grouping them in a
    /// struct would not make the call sites clearer.
    ///
    /// # Errors
    /// Returns an error when a bind address or path is invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve(
        data_dir: Option<PathBuf>,
        admin_bind: Option<SocketAddr>,
        kiosk_bind: Option<SocketAddr>,
        static_dir: Option<PathBuf>,
        vision_socket: Option<PathBuf>,
        dev_mode: bool,
        fake_vision: bool,
        log_json: bool,
    ) -> Result<Self, String> {
        let data_dir = data_dir
            .or_else(|| std::env::var_os(DATA_DIR_ENV).map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(hs_core::DEFAULT_DATA_DIR));

        let admin_bind = match admin_bind {
            Some(bind) => bind,
            None => parse_bind(
                std::env::var(ADMIN_BIND_ENV).ok().as_deref(),
                DEFAULT_ADMIN_BIND,
            )?,
        };
        let kiosk_bind = match kiosk_bind {
            Some(bind) => bind,
            None => parse_bind(
                std::env::var(KIOSK_BIND_ENV).ok().as_deref(),
                DEFAULT_KIOSK_BIND,
            )?,
        };
        let static_dir = static_dir
            .or_else(|| std::env::var_os(STATIC_DIR_ENV).map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("ui/dist"));
        let vision_socket = vision_socket
            .or_else(|| std::env::var_os(VISION_SOCKET_ENV).map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_VISION_SOCKET));
        let (caddy_apps_dir, caddy_env_file) = if dev_mode {
            (
                std::env::var_os(CADDY_APPS_DIR_ENV)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| data_dir.join("caddy").join("apps.d")),
                std::env::var_os(CADDY_ENV_FILE_ENV)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| data_dir.join("caddy").join("caddy.env")),
            )
        } else {
            (
                std::env::var_os(CADDY_APPS_DIR_ENV)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(DEFAULT_CADDY_APPS_DIR)),
                std::env::var_os(CADDY_ENV_FILE_ENV)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(DEFAULT_CADDY_ENV_FILE)),
            )
        };

        Ok(Self {
            data_dir,
            admin_bind,
            kiosk_bind,
            static_dir,
            vision_socket,
            caddy_apps_dir,
            caddy_env_file,
            dev_mode,
            fake_vision,
            log_json,
        })
    }

    /// Create the data directory layout with restrictive permissions.
    ///
    /// # Errors
    /// Returns an I/O error when the directory cannot be created.
    pub fn ensure_layout(&self) -> std::io::Result<()> {
        fsutil::ensure_dir(&self.data_dir)?;
        fsutil::ensure_dir(&self.data_dir.join("caddy"))?;
        fsutil::ensure_dir(&self.data_dir.join("backups"))?;
        if self.dev_mode {
            fsutil::ensure_dir(&self.caddy_apps_dir)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                std::fs::set_permissions(&self.data_dir, std::fs::Permissions::from_mode(0o700));
        }
        Ok(())
    }

    /// Path of the system configuration file.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.data_dir
            .join(hs_core::state::config_store::CONFIG_FILE)
    }

    /// Path of the embedded database.
    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("state.redb")
    }

    /// Path of the encrypted master key.
    #[must_use]
    pub fn master_key_path(&self) -> PathBuf {
        self.data_dir.join(hs_core::state::vault::MASTER_KEY_FILE)
    }

    /// Directory containing the built admin/kiosk assets.
    #[must_use]
    pub fn static_dir(&self) -> &Path {
        &self.static_dir
    }
}

fn parse_bind(value: Option<&str>, fallback: &str) -> Result<SocketAddr, String> {
    let raw = value.unwrap_or(fallback);
    raw.parse::<SocketAddr>()
        .map_err(|err| format!("invalid bind address `{raw}`: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_uses_explicit_values() {
        let config = DaemonConfig::resolve(
            Some(PathBuf::from("/tmp/hs-data")),
            Some("127.0.0.1:9000".parse().unwrap()),
            Some("127.0.0.1:3001".parse().unwrap()),
            Some(PathBuf::from("/tmp/ui")),
            Some(PathBuf::from("/tmp/vision.sock")),
            true,
            true,
            false,
        )
        .unwrap();
        assert_eq!(config.data_dir, PathBuf::from("/tmp/hs-data"));
        assert_eq!(config.admin_bind.port(), 9000);
        assert_eq!(config.kiosk_bind.port(), 3001);
        assert!(config.dev_mode);
        assert_eq!(
            config.config_path(),
            PathBuf::from("/tmp/hs-data").join("config.json")
        );
        assert_eq!(
            config.database_path(),
            PathBuf::from("/tmp/hs-data").join("state.redb")
        );
    }

    #[test]
    fn resolve_rejects_bad_bind() {
        assert!(parse_bind(Some("not-a-socket"), DEFAULT_ADMIN_BIND).is_err());
        assert!(parse_bind(None, DEFAULT_ADMIN_BIND).is_ok());
    }
}
