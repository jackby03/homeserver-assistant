//! Caddy integration: per-application reverse-proxy sites and global settings.
//!
//! The daemon owns the Caddy "edge" configuration contract:
//!
//! - `/etc/caddy/apps.d/<app-id>.caddy` — one site block per LAN application,
//!   included by the managed Caddyfile (`import apps.d/*.caddy`).
//! - `/etc/homecore/caddy.env` — `HS_HOSTNAME`, `HS_BASE_DOMAIN`,
//!   `HS_ADMIN_PORT` consumed by the admin portal site.
//!
//! Files are written atomically and Caddy is reloaded through systemd on Linux.
//! In development mode both paths live under the data directory, so nothing
//! outside the workspace is touched.

use std::path::Path;

use hs_core::models::{AppRecord, NetworkSegment};
use tracing::{info, warn};
use uuid::Uuid;

use crate::state::SharedState;

/// Sync the Caddy site snippet for a deployed application.
///
/// DMZ applications intentionally get no site: they are only reachable through
/// the outbound tunnel, never through the LAN reverse proxy.
///
/// # Errors
/// Returns a message when the runtime address cannot be resolved or the snippet
/// cannot be written.
pub async fn sync_app(state: &SharedState, app: &AppRecord) -> Result<(), String> {
    if app.segment != NetworkSegment::Lan {
        return Ok(());
    }
    let Some(hostname) = &app.hostname else {
        return Ok(());
    };
    let instance = state
        .runtime
        .get(&app.instance_name)
        .await
        .map_err(|err| format!("cannot resolve application address: {err}"))?;
    let Some(address) = instance.ipv4_addresses.first() else {
        return Err(format!(
            "application `{}` has no IPv4 address yet",
            app.name
        ));
    };

    let body = render_app_site(hostname, address, app.internal_port, &app.name, app.id);
    write_atomic(&app_file(state, app.id), &body, 0o644)?;
    reload(state).await;
    info!(app = %app.name, hostname = %hostname, upstream = %address, "caddy site updated");
    Ok(())
}

/// Remove the Caddy site of an application (idempotent).
///
/// # Errors
/// Returns a message when the file cannot be removed.
pub async fn remove_app(state: &SharedState, app_id: Uuid) -> Result<(), String> {
    let path = app_file(state, app_id);
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|err| format!("cannot remove {}: {err}", path.display()))?;
        reload(state).await;
        info!(%app_id, "caddy site removed");
    }
    Ok(())
}

/// Write the global Caddy environment file (hostname, base domain, admin port).
///
/// Caddy reads environment files at process start; a restart is required after
/// a hostname/base-domain change. The daemon logs that explicitly.
///
/// # Errors
/// Returns a message when the file cannot be written.
pub fn sync_global(state: &SharedState) -> Result<(), String> {
    let config = state.config_snapshot();
    let body = format!(
        "# Managed by HomeServer Assistant OS - do not edit.\n\
         HS_HOSTNAME={}\n\
         HS_BASE_DOMAIN={}\n\
         HS_ADMIN_PORT={}\n",
        config.device.hostname, config.services.base_domain, config.security.admin_web_port
    );
    write_atomic(&state.daemon.caddy_env_file, &body, 0o644)?;
    info!(
        path = %state.daemon.caddy_env_file.display(),
        "caddy environment updated (restart caddy after hostname changes)"
    );
    Ok(())
}

/// Trigger a Caddy reload on Linux (best effort, never fails the request).
async fn reload(state: &SharedState) {
    if state.daemon.dev_mode || !cfg!(target_os = "linux") {
        return;
    }
    let result = tokio::process::Command::new("systemctl")
        .args(["reload", "caddy"])
        .output()
        .await;
    match result {
        Ok(output) if output.status.success() => {
            info!("caddy configuration reloaded");
        }
        Ok(output) => {
            warn!(
                stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                "caddy reload failed; the site will apply on the next reload"
            );
        }
        Err(err) => warn!(error = %err, "failed to run `systemctl reload caddy`"),
    }
}

fn app_file(state: &SharedState, app_id: Uuid) -> std::path::PathBuf {
    state.daemon.caddy_apps_dir.join(format!("{app_id}.caddy"))
}

fn render_app_site(
    hostname: &str,
    upstream_ip: &str,
    internal_port: u16,
    app_name: &str,
    app_id: Uuid,
) -> String {
    format!(
        "# Managed by HomeServer Assistant OS - do not edit.\n\
         # Application: {app_name} ({app_id})\n\
         {hostname} {{\n\
         \ttls internal\n\
         \tencode zstd gzip\n\
         \treverse_proxy http://{upstream_ip}:{internal_port}\n\
         \theader {{\n\
         \t\tX-Content-Type-Options nosniff\n\
         \t\tX-Frame-Options SAMEORIGIN\n\
         \t}}\n\
         }}\n"
    )
}

fn write_atomic(path: &Path, body: &str, mode: u32) -> Result<(), String> {
    #[cfg(not(unix))]
    let _ = mode;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body).map_err(|err| format!("cannot write {}: {err}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    }
    std::fs::rename(&tmp, path).map_err(|err| format!("cannot replace {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_site_block_with_internal_tls() {
        let id = Uuid::nil();
        let site = render_app_site("nextcloud.home.local", "10.20.30.5", 8080, "Nextcloud", id);
        assert!(site.contains("nextcloud.home.local {"));
        assert!(site.contains("tls internal"));
        assert!(site.contains("reverse_proxy http://10.20.30.5:8080"));
        assert!(site.contains("X-Content-Type-Options nosniff"));
        assert!(site.contains("Managed by HomeServer Assistant OS"));
    }

    #[test]
    fn writes_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.d").join("x.caddy");
        write_atomic(&path, "first", 0o644).unwrap();
        write_atomic(&path, "second", 0o644).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    }
}
