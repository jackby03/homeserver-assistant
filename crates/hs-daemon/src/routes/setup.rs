//! First-boot onboarding routes.
//!
//! Steps 2..=6 require an authenticated administrator (created in step 1), so a
//! device on the LAN cannot be configured by somebody else before the owner
//! finishes onboarding. All endpoints are disabled once `initialized == true`
//! by the [`crate::middleware::auth::setup_only`] middleware.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use hs_agent::UpsertProviderRequest;
use hs_apps::builtin_catalog;
use hs_auth::Permission;
use hs_core::config::{DmzConfig, DnsConfig, NetworkConfig, NetworkMode, StaticIpv4};
use hs_core::models::{AppRecord, AuditOutcome, DeployAppRequest, NetworkSegment, Profile};
use hs_core::validation;
use hs_profiles::CreateProfileRequest;
use hs_system::AudioSink;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::error::{HttpError, HttpResult};
use crate::events::ServerEvent;
use crate::middleware::auth::{setup_only, AdminAuth};
use crate::routes::auth::{build_session_response, client_ip};
use crate::state::SharedState;

/// Setup routes (public step 1, authenticated steps 2-6).
pub fn router(state: SharedState) -> Router<SharedState> {
    Router::new()
        .route("/setup/admin", post(setup_admin))
        .route("/setup/network", post(setup_network))
        .route("/setup/peripherals", axum::routing::get(peripherals))
        .route("/setup/audio/test", post(audio_test))
        .route("/setup/profiles", post(setup_profiles))
        .route("/setup/agent", post(setup_agent))
        .route("/setup/finish", post(finish))
        .layer(axum::middleware::from_fn_with_state(state, setup_only))
}

// ---------------------------------------------------------------------------
// Step 1: device + administrator
// ---------------------------------------------------------------------------

/// Device identity chosen in the wizard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupDevice {
    /// Friendly device name.
    pub name: String,
    /// UI language (`es` or `en`).
    pub language: String,
    /// IANA timezone.
    pub timezone: String,
}

/// Step 1 request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupAdminRequest {
    /// Administrator login name.
    pub username: String,
    /// Administrator password (policy enforced server-side).
    pub password: String,
    /// Device settings.
    pub device: SetupDevice,
}

async fn setup_admin(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<SetupAdminRequest>,
) -> HttpResult<Response> {
    if state.is_initialized() {
        return Err(HttpError::conflict("the appliance is already initialized"));
    }
    if state.auth.has_users().map_err(HttpError::from)? {
        return Err(HttpError::conflict(
            "an administrator account already exists; finish onboarding from the portal",
        ));
    }

    validation::device_name(&request.device.name).map_err(HttpError::from)?;
    validation::language(&request.device.language).map_err(HttpError::from)?;
    validation::timezone(&request.device.timezone).map_err(HttpError::from)?;

    let auth = Arc::clone(&state.auth);
    let username = request.username.clone();
    let password = request.password.clone();
    let display_name = request.username.clone();
    let user = tokio::task::spawn_blocking(move || {
        auth.bootstrap_admin(&username, &password, &display_name)
    })
    .await
    .map_err(|err| HttpError::internal(format!("admin creation task failed: {err}")))??;

    // Prepare the configuration (still uninitialized).
    let hostname = slugify(&request.device.name);
    state
        .config
        .update(|config| {
            config.device.name = request.device.name.clone();
            config.device.language = request.device.language.clone();
            config.device.timezone = request.device.timezone.clone();
            config.device.hostname = hostname.clone();
            if config.kiosk.token.is_empty() {
                config.rotate_kiosk_token();
            }
            Ok(())
        })
        .map_err(HttpError::from)?;

    state.kiosk.set_device_name(request.device.name.clone());
    state.setup.complete_step(1);
    state.audit(
        &user.username,
        "setup.admin",
        Some(&user.id.to_string()),
        AuditOutcome::Success,
        client_ip(&headers).as_deref(),
        Some(serde_json::json!({"device": request.device.name})),
    );

    // Log in immediately so the wizard continues with an authenticated session.
    let auth = Arc::clone(&state.auth);
    let username = request.username.clone();
    let password = request.password.clone();
    let ip = client_ip(&headers);
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let login = tokio::task::spawn_blocking(move || {
        auth.login(&username, &password, ip.as_deref(), ua.as_deref())
    })
    .await
    .map_err(|err| HttpError::internal(format!("login task failed: {err}")))??;

    info!(device = %request.device.name, "first-boot administrator created");
    Ok(build_session_response(
        &headers,
        login.token,
        login.csrf_token,
        &login.user,
    ))
}

// ---------------------------------------------------------------------------
// Step 2: network + DNS + DMZ
// ---------------------------------------------------------------------------

/// Step 2 request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupNetworkRequest {
    /// Uplink interface name.
    pub interface: String,
    /// Trusted LAN bridge name (defaults to `br-lan`).
    #[serde(default)]
    pub lan_bridge: Option<String>,
    /// `dhcp` or `static`.
    pub mode: NetworkMode,
    /// Static address settings.
    #[serde(default)]
    pub static_ipv4: Option<StaticIpv4>,
    /// DNS configuration.
    pub dns: DnsConfig,
    /// DMZ configuration.
    pub dmz: DmzConfig,
}

async fn setup_network(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(request): Json<SetupNetworkRequest>,
) -> HttpResult<Json<NetworkConfig>> {
    auth.require(Permission::NetworkManage)?;
    let network = NetworkConfig {
        interface: request.interface,
        lan_bridge: request
            .lan_bridge
            .unwrap_or_else(|| hs_core::config::DEFAULT_LAN_BRIDGE.to_owned()),
        mode: request.mode,
        static_ipv4: request.static_ipv4,
        dns: request.dns,
        dmz: request.dmz,
    };
    network.validate().map_err(HttpError::from)?;
    let updated = state
        .config
        .update(|config| {
            config.network = network.clone();
            Ok(())
        })
        .map_err(HttpError::from)?;
    state.setup.complete_step(2);
    state.audit(
        &auth.user().username,
        "setup.network",
        None,
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({
            "interface": updated.network.interface,
            "dns_provider": updated.network.dns.provider.id(),
            "dns_protocol": updated.network.dns.protocol.id(),
            "dmz_enabled": updated.network.dmz.enabled,
        })),
    );
    Ok(Json(updated.network))
}

// ---------------------------------------------------------------------------
// Step 3: audio, camera and touchscreen
// ---------------------------------------------------------------------------

/// Peripherals discovered on the host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeripheralsResponse {
    /// Camera device nodes.
    pub camera_devices: Vec<String>,
    /// Touchscreen/input device nodes.
    pub touch_devices: Vec<String>,
    /// PipeWire sinks.
    pub audio_sinks: Vec<AudioSink>,
}

async fn peripherals(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<PeripheralsResponse>> {
    auth.require(Permission::DashboardView)?;
    let status = state.system.peripherals();
    Ok(Json(PeripheralsResponse {
        camera_devices: status.camera_devices,
        touch_devices: status.touch_devices,
        audio_sinks: state.system.audio_sinks(),
    }))
}

/// Audio test request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioTestRequest {
    /// Sink selected in the wizard.
    #[serde(default)]
    pub sink_id: Option<String>,
}

/// Audio test response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioTestResponse {
    /// Whether the sink was applied.
    pub applied: bool,
    /// Selected sink id.
    pub sink_id: Option<String>,
    /// Selected sink description.
    pub sink_name: Option<String>,
}

async fn audio_test(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(request): Json<AudioTestRequest>,
) -> HttpResult<Json<AudioTestResponse>> {
    auth.require(Permission::DashboardView)?;
    let sinks = state.system.audio_sinks();
    let selected = match &request.sink_id {
        Some(id) => sinks.iter().find(|sink| &sink.id == id).cloned(),
        None => sinks.iter().find(|sink| sink.is_default).cloned(),
    };

    if let Some(sink) = &selected {
        state
            .system
            .set_audio_sink(&sink.id)
            .map_err(HttpError::from)?;
        let volume = state.config_snapshot().audio.volume;
        state.system.set_volume(volume).map_err(HttpError::from)?;
        state
            .config
            .update(|config| {
                config.audio.sink_id = Some(sink.id.clone());
                config.audio.sink_name = Some(sink.name.clone());
                Ok(())
            })
            .map_err(HttpError::from)?;
    }

    state.setup.complete_step(3);
    state.audit(
        &auth.user().username,
        "setup.peripherals",
        request.sink_id.as_deref(),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({
            "sink": selected.as_ref().map(|sink| sink.id.clone()),
            "camera_devices": state.system.peripherals().camera_devices.len(),
        })),
    );
    Ok(Json(AudioTestResponse {
        applied: selected.is_some(),
        sink_id: selected.as_ref().map(|sink| sink.id.clone()),
        sink_name: selected.map(|sink| sink.name),
    }))
}

// ---------------------------------------------------------------------------
// Step 5: profiles + AI provider
// ---------------------------------------------------------------------------

/// Profiles creation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupProfilesRequest {
    /// Profiles to create.
    pub profiles: Vec<CreateProfileRequest>,
}

async fn setup_profiles(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(request): Json<SetupProfilesRequest>,
) -> HttpResult<Json<Vec<Profile>>> {
    auth.require(Permission::ProfilesManage)?;
    if request.profiles.len() > 16 {
        return Err(HttpError::bad_request("at most 16 profiles can be created"));
    }
    let mut created = Vec::with_capacity(request.profiles.len());
    for profile in request.profiles {
        created.push(state.profiles.create(profile).map_err(HttpError::from)?);
    }
    state.setup.complete_step(5);
    state.audit(
        &auth.user().username,
        "setup.profiles",
        None,
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"count": created.len()})),
    );
    Ok(Json(created))
}

/// AI provider configuration request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupAgentRequest {
    /// Provider to create and activate.
    pub provider: UpsertProviderRequest,
}

async fn setup_agent(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(request): Json<SetupAgentRequest>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::AgentManage)?;
    let record = state
        .agent
        .upsert_provider(request.provider)
        .map_err(HttpError::from)?;
    state
        .agent
        .set_active_provider(record.id)
        .await
        .map_err(HttpError::from)?;
    let kind = record.kind;
    let model = record.model.clone();
    let base_url = record.base_url.clone();
    state
        .config
        .update(|config| {
            config.agent.provider = Some(kind);
            config.agent.model = model.clone();
            config.agent.base_url = base_url.clone();
            Ok(())
        })
        .map_err(HttpError::from)?;
    state.setup.complete_step(5);
    state.audit(
        &auth.user().username,
        "setup.agent",
        Some(&record.id.to_string()),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"kind": kind.id(), "model": record.model})),
    );
    Ok(Json(serde_json::json!({"provider": record})))
}

// ---------------------------------------------------------------------------
// Step 6: deployment
// ---------------------------------------------------------------------------

/// Finish request (no body fields today).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FinishRequest {}

async fn finish(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::NetworkManage)?;
    if state.is_initialized() {
        return Err(HttpError::conflict("the appliance is already initialized"));
    }
    if !state.setup.try_begin_deploy() {
        return Err(HttpError::conflict("the deployment job is already running"));
    }
    let job_id = Uuid::new_v4();
    state.audit(
        &auth.user().username,
        "setup.finish",
        Some(&job_id.to_string()),
        AuditOutcome::Success,
        None,
        None,
    );
    let task_state = Arc::clone(&state);
    tokio::spawn(async move {
        run_deployment(task_state).await;
    });
    Ok(Json(serde_json::json!({
        "job_id": job_id,
        "message": "deployment started",
    })))
}

/// Run the first-boot deployment, streaming progress over `/ws/setup`.
///
/// Every infrastructure step is best-effort: failures are reported as warnings
/// (the appliance can still be managed from the portal) except for the final
/// configuration write, which decides whether onboarding completed.
async fn run_deployment(state: SharedState) {
    let config = state.config_snapshot();
    let mut warnings: Vec<String> = Vec::new();
    let progress = |step: u8, label: &str, percent: u8, done: bool| {
        state.setup.publish_progress(step, label, percent, done);
    };

    progress(6, "Applying power management policy", 5, false);
    if let Err(err) = state.system.apply_power_policy(80) {
        warnings.push(format!("power policy: {err}"));
        warn!(error = %err, "power policy could not be applied");
    }

    progress(6, "Applying network configuration", 15, false);
    #[cfg(target_os = "linux")]
    if !state.daemon.dev_mode {
        let netplan_path = std::path::Path::new("/etc/netplan/90-homeserver.yaml");
        if let Err(err) = hs_network::netplan::apply_netplan(&config.network, netplan_path).await {
            warnings.push(format!("netplan: {err}"));
            warn!(error = %err, "netplan configuration could not be applied");
        }
    }

    progress(6, "Applying firewall rules", 25, false);
    #[cfg(target_os = "linux")]
    if config.security.enforce_dmz_isolation {
        if let Ok(plan) = firewall_plan(&config) {
            let path = state.daemon.data_dir.join("firewall").join("nftables.conf");
            if let Err(err) = hs_network::firewall::apply(&plan, &path).await {
                warnings.push(format!("firewall: {err}"));
                warn!(error = %err, "firewall ruleset could not be applied");
            }
        }
    }

    progress(6, "Deploying DNS filtering", 40, false);
    if config.network.dns.enabled {
        match deploy_template(&state, "adguard-home", "dns", NetworkSegment::Lan).await {
            Ok(app) => {
                info!(app = %app.name, "DNS filter deployed");
                if let Err(err) = crate::caddy::sync_app(&state, &app).await {
                    warnings.push(format!("dns route: {err}"));
                    warn!(error = %err, "DNS filter Caddy route failed");
                }
                // Point LAN clients at the deployed resolver once its address
                // is known, replacing the 127.0.0.1 placeholder.
                if let Ok(instance) = state.runtime.get(&app.instance_name).await {
                    if let Some(ip) = instance.ipv4_addresses.first() {
                        #[cfg(target_os = "linux")]
                        if !state.daemon.dev_mode {
                            let netplan_path =
                                std::path::Path::new("/etc/netplan/90-homeserver.yaml");
                            let dns_address = ip.clone();
                            if let Err(err) = hs_network::netplan::apply_netplan_with_dns(
                                &config.network,
                                netplan_path,
                                &dns_address,
                            )
                            .await
                            {
                                warnings.push(format!("netplan dns: {err}"));
                                warn!(error = %err, "netplan DNS resolver update failed");
                            }
                        }
                        info!(resolver = %ip, "LAN resolver advertised");
                    }
                }
            }
            Err(err) => {
                warnings.push(format!("dns: {err}"));
                warn!(error = %err, "DNS filter deployment failed");
            }
        }
    }

    progress(6, "Deploying Home Assistant", 55, false);
    if config.services.enable_home_assistant {
        match deploy_template(&state, "home-assistant", "home", NetworkSegment::Lan).await {
            Ok(app) => {
                if let Err(err) = crate::caddy::sync_app(&state, &app).await {
                    warnings.push(format!("home assistant route: {err}"));
                    warn!(error = %err, "Home Assistant Caddy route failed");
                }
                if let Ok(instance) = state.runtime.get(&app.instance_name).await {
                    if let Some(ip) = instance.ipv4_addresses.first() {
                        let url = format!("http://{ip}:8123");
                        let _ = state.config.update(|config| {
                            config.services.home_assistant_url = Some(url.clone());
                            Ok(())
                        });
                    }
                }
            }
            Err(err) => {
                warnings.push(format!("home assistant: {err}"));
                warn!(error = %err, "Home Assistant deployment failed");
            }
        }
    }

    progress(6, "Deploying music assistant", 70, false);
    if config.services.enable_music {
        match deploy_template(&state, "music-assistant", "music", NetworkSegment::Lan).await {
            Ok(app) => {
                if let Err(err) = crate::caddy::sync_app(&state, &app).await {
                    warnings.push(format!("music assistant route: {err}"));
                    warn!(error = %err, "Music Assistant Caddy route failed");
                }
            }
            Err(err) => {
                warnings.push(format!("music assistant: {err}"));
                warn!(error = %err, "Music Assistant deployment failed");
            }
        }
    }

    progress(6, "Saving configuration", 88, false);
    let initialized = state.config.update(|config| {
        config.initialized = true;
        if config.kiosk.token.is_empty() {
            config.rotate_kiosk_token();
        }
        Ok(())
    });

    match initialized {
        Ok(config) => {
            state.kiosk.set_device_name(config.device.name.clone());
            if let Err(err) = crate::caddy::sync_global(&state) {
                warnings.push(format!("caddy environment: {err}"));
                warn!(error = %err, "Caddy environment file could not be written");
            }
            state.setup.complete_step(6);
            progress(6, "Home server ready", 100, true);
            state.events.publish_server(ServerEvent::Notification {
                level: "success".to_owned(),
                message: "first boot completed".to_owned(),
            });
            state.setup.finish_deploy(None);
            for warning in &warnings {
                state.events.publish_server(ServerEvent::Notification {
                    level: "warning".to_owned(),
                    message: warning.clone(),
                });
            }
            info!(warnings = warnings.len(), "first boot completed");
        }
        Err(err) => {
            let message = format!("failed to persist the final configuration: {err}");
            state.setup.fail(message.clone());
            warn!(error = %err, "first boot failed");
        }
    }
}

async fn deploy_template(
    state: &SharedState,
    template_id: &str,
    subdomain: &str,
    segment: NetworkSegment,
) -> Result<AppRecord, String> {
    let template = builtin_catalog()
        .into_iter()
        .find(|template| template.id == template_id)
        .ok_or_else(|| format!("template `{template_id}` not found"))?;
    let request = DeployAppRequest {
        name: template.name.clone(),
        template_id: Some(template.id.clone()),
        image: None,
        segment,
        internal_port: template.default_port,
        subdomain: Some(subdomain.to_owned()),
        expose_externally: false,
        env: std::collections::BTreeMap::new(),
    };
    state
        .apps
        .deploy(request)
        .await
        .map_err(|err| err.to_string())
}

#[cfg(target_os = "linux")]
fn firewall_plan(
    config: &hs_core::config::SystemConfig,
) -> Result<hs_network::firewall::FirewallPlan, String> {
    let dmz_subnet = config
        .network
        .dmz
        .subnet
        .parse::<ipnet::Ipv4Net>()
        .map_err(|err| format!("invalid DMZ subnet: {err}"))?;
    Ok(hs_network::firewall::FirewallPlan {
        lan_bridge: config.network.lan_bridge.clone(),
        dmz_bridge: config.network.dmz.bridge.clone(),
        admin_port: config.security.admin_web_port,
        dns_port: 53,
        dmz_allow_dns: config.network.dmz.allow_dns,
        dmz_subnet,
        lan_subnet: None,
        wan_interface: Some(config.network.interface.clone()),
        allow_dmz_egress: true,
    })
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        let lower = ch.to_lowercase().next().unwrap_or(ch);
        let mapped = match lower {
            'á' | 'à' | 'ä' | 'â' | 'ã' => Some('a'),
            'é' | 'è' | 'ë' | 'ê' => Some('e'),
            'í' | 'ì' | 'ï' | 'î' => Some('i'),
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => Some('o'),
            'ú' | 'ù' | 'ü' | 'û' => Some('u'),
            'ñ' => Some('n'),
            'ç' => Some('c'),
            other if other.is_ascii() => Some(other),
            _ => None,
        };
        let Some(mapped) = mapped else {
            continue;
        };
        if mapped.is_ascii_alphanumeric() {
            out.push(mapped);
        } else if matches!(mapped, ' ' | '-' | '_' | '.') && !out.is_empty() && !out.ends_with('-')
        {
            out.push('-');
        }
        if out.len() >= 63 {
            break;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "homeserver".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_produces_valid_hostnames() {
        assert_eq!(slugify("Salón Principal"), "salon-principal");
        assert_eq!(slugify("  "), "homeserver");
        assert_eq!(slugify("HomeServer_01!"), "homeserver-01");
        assert_eq!(slugify("Mamá y Papá"), "mama-y-papa");
    }
}
