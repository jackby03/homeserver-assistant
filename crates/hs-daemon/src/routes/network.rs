//! Network, encrypted DNS, DMZ and firewall routes.

use std::time::Instant;

use axum::extract::State;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use hs_auth::Permission;
use hs_core::config::{
    DmzConfig, DnsConfig, DnsProvider, NetworkConfig, NetworkMode, StaticIpv4, SystemConfig,
};
use hs_core::models::AuditOutcome;
use hs_network::firewall::{self, FirewallPlan};
use hs_network::interfaces::{self, InterfaceInfo};
use serde::{Deserialize, Serialize};

use crate::error::{HttpError, HttpResult};
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;

/// Network routes.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/network", get(get_network).put(update_network))
        .route("/network/dns", put(update_dns))
        .route("/network/dns/providers", get(dns_providers))
        .route("/network/dns/test", post(test_dns))
        .route("/network/dmz", put(update_dmz))
        .route("/network/firewall", get(get_firewall))
        .route("/network/firewall/apply", post(apply_firewall))
}

/// Addressing update payload used by the network page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateNetworkBody {
    /// Physical uplink interface.
    pub interface: String,
    /// Trusted LAN bridge (defaults to the current value when omitted).
    #[serde(default)]
    pub lan_bridge: Option<String>,
    /// Addressing mode.
    pub mode: NetworkMode,
    /// Static address when `mode == static`.
    #[serde(default)]
    pub static_ipv4: Option<StaticIpv4>,
}

/// `PUT /api/v1/network` — update interface, bridge and addressing mode.
async fn update_network(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(body): Json<UpdateNetworkBody>,
) -> HttpResult<Json<NetworkConfig>> {
    auth.require(Permission::NetworkManage)?;
    let current = state.config_snapshot().network;
    let network = NetworkConfig {
        interface: body.interface,
        lan_bridge: body
            .lan_bridge
            .unwrap_or_else(|| current.lan_bridge.clone()),
        mode: body.mode,
        static_ipv4: body.static_ipv4,
        dns: current.dns,
        dmz: current.dmz,
    };
    network.validate().map_err(HttpError::from)?;
    let updated = state
        .config
        .update(|config| {
            config.network = network.clone();
            Ok(())
        })
        .map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "network.update",
        None,
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({
            "interface": updated.network.interface,
            "lan_bridge": updated.network.lan_bridge,
            "mode": match updated.network.mode { NetworkMode::Dhcp => "dhcp", NetworkMode::Static => "static" },
            "static_ipv4": updated.network.static_ipv4.as_ref().map(|ip| ip.address.clone()),
        })),
    );
    Ok(Json(updated.network))
}

/// `GET /api/v1/network`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkResponse {
    /// Detected host interfaces.
    pub interfaces: Vec<InterfaceInfo>,
    /// Persisted network configuration.
    pub config: NetworkConfig,
}

/// Provider selector entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsProviderInfo {
    /// Stable id.
    pub id: String,
    /// Human readable label.
    pub label: String,
}

/// Firewall view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirewallResponse {
    /// Rendered nftables ruleset.
    pub ruleset: String,
    /// Last applied timestamp (RFC3339), if any.
    pub last_applied_at: Option<String>,
}

/// DNS probe result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsTestResponse {
    /// Whether the encrypted upstream looks reachable.
    pub ok: bool,
    /// Observed TCP handshake latency.
    pub latency_ms: Option<u64>,
    /// Provider id tested.
    pub provider: String,
    /// Error detail when `ok == false`.
    pub error: Option<String>,
}

async fn get_network(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<NetworkResponse>> {
    auth.require(Permission::NetworkRead)?;
    let config = state.config_snapshot();
    Ok(Json(NetworkResponse {
        interfaces: interfaces::detect(),
        config: config.network,
    }))
}

async fn update_dns(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(dns): Json<DnsConfig>,
) -> HttpResult<Json<NetworkConfig>> {
    auth.require(Permission::NetworkManage)?;
    dns.validate().map_err(HttpError::from)?;
    let updated = persist_network(&state, |network| {
        network.dns = dns.clone();
        Ok(())
    })?;
    state.audit(
        &auth.user().username,
        "network.dns_update",
        None,
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({
            "provider": dns.provider.id(),
            "protocol": dns.protocol.id(),
            "enabled": dns.enabled,
            "blocklists": dns.blocklists.len(),
        })),
    );
    Ok(Json(updated))
}

async fn dns_providers(auth: AdminAuth) -> HttpResult<Json<Vec<DnsProviderInfo>>> {
    auth.require(Permission::NetworkRead)?;
    Ok(Json(
        DnsProvider::all()
            .iter()
            .map(|provider| DnsProviderInfo {
                id: provider.id().to_owned(),
                label: provider.label().to_owned(),
            })
            .collect(),
    ))
}

async fn update_dmz(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(dmz): Json<DmzConfig>,
) -> HttpResult<Json<NetworkConfig>> {
    auth.require(Permission::NetworkManage)?;
    dmz.validate().map_err(HttpError::from)?;
    let updated = persist_network(&state, |network| {
        network.dmz = dmz.clone();
        Ok(())
    })?;
    state.audit(
        &auth.user().username,
        "network.dmz_update",
        None,
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({
            "enabled": dmz.enabled,
            "subnet": dmz.subnet,
            "bridge": dmz.bridge,
        })),
    );
    Ok(Json(updated))
}

async fn get_firewall(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<FirewallResponse>> {
    auth.require(Permission::NetworkRead)?;
    let config = state.config_snapshot();
    let plan = build_plan(&config)?;
    let ruleset = firewall::render_nftables(&plan);
    firewall::validate_isolation(&ruleset).map_err(HttpError::from)?;
    let last_applied_at: Option<String> = state
        .db
        .meta("network.firewall_last_applied")
        .map_err(HttpError::from)?;
    Ok(Json(FirewallResponse {
        ruleset,
        last_applied_at,
    }))
}

async fn apply_firewall(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::NetworkManage)?;
    let config = state.config_snapshot();
    let plan = build_plan(&config)?;
    let target = state.daemon.data_dir.join("firewall").join("nftables.conf");
    let ruleset = firewall::render_nftables(&plan);
    firewall::validate_isolation(&ruleset).map_err(HttpError::from)?;
    firewall::apply(&plan, &target)
        .await
        .map_err(HttpError::from)?;
    let now = chrono::Utc::now().to_rfc3339();
    state
        .db
        .set_meta("network.firewall_last_applied", &now)
        .map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "network.firewall_apply",
        None,
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"path": target.display().to_string()})),
    );
    Ok(Json(serde_json::json!({
        "applied": true,
        "rules_count": ruleset.lines().filter(|line| line.contains("counter")).count(),
        "applied_at": now,
    })))
}

async fn test_dns(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<DnsTestResponse>> {
    auth.require(Permission::NetworkRead)?;
    let config = state.config_snapshot();
    let dns = config.network.dns;
    let (host, port) = probe_target(&dns);
    let provider = dns.provider.id().to_owned();
    let started = Instant::now();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await;

    match result {
        Ok(Ok(_stream)) => Ok(Json(DnsTestResponse {
            ok: true,
            latency_ms: Some(started.elapsed().as_millis() as u64),
            provider,
            error: None,
        })),
        Ok(Err(err)) => Ok(Json(DnsTestResponse {
            ok: false,
            latency_ms: None,
            provider,
            error: Some(err.to_string()),
        })),
        Err(_) => Ok(Json(DnsTestResponse {
            ok: false,
            latency_ms: None,
            provider,
            error: Some("connection timed out".to_owned()),
        })),
    }
}

fn persist_network<F>(state: &SharedState, mutate: F) -> Result<NetworkConfig, HttpError>
where
    F: FnOnce(&mut NetworkConfig) -> hs_core::CoreResult<()>,
{
    let updated = state
        .config
        .update(|config: &mut SystemConfig| mutate(&mut config.network))
        .map_err(HttpError::from)?;
    Ok(updated.network)
}

fn build_plan(config: &SystemConfig) -> Result<FirewallPlan, HttpError> {
    let dmz_subnet = config
        .network
        .dmz
        .subnet
        .parse::<ipnet::Ipv4Net>()
        .map_err(|err| HttpError::bad_request(format!("invalid DMZ subnet: {err}")))?;
    Ok(FirewallPlan {
        lan_bridge: config.network.lan_bridge.clone(),
        dmz_bridge: config.network.dmz.bridge.clone(),
        admin_port: config.security.admin_web_port,
        dns_port: 53,
        dmz_allow_dns: config.network.dmz.allow_dns,
        dmz_subnet,
        lan_subnet: None,
        wan_interface: Some(config.network.interface.clone()),
        allow_dmz_egress: config.network.dmz.enabled,
    })
}

fn probe_target(dns: &DnsConfig) -> (String, u16) {
    if dns.provider == DnsProvider::Custom {
        if let Some(upstream) = &dns.custom_upstream {
            let rest = upstream
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            let host = rest
                .split('/')
                .next()
                .unwrap_or(rest)
                .split(':')
                .next()
                .unwrap_or(rest);
            return (host.to_owned(), 443);
        }
    }
    let host = match dns.provider {
        DnsProvider::Cloudflare
        | DnsProvider::CloudflareMalware
        | DnsProvider::CloudflareFamily => "1.1.1.1",
        DnsProvider::Quad9 => "9.9.9.9",
        DnsProvider::ControlD => "76.76.2.0",
        DnsProvider::Custom => "1.1.1.1",
    };
    (host.to_owned(), 443)
}
