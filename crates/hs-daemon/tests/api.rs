//! Integration tests for the daemon HTTP API.
//!
//! These tests exercise the real axum routers, middleware, RBAC, CSRF and
//! domain services against the in-memory mock runtime and fake hardware. No
//! network, root privileges or Incus are required.

mod common;

use axum::http::{Method, StatusCode};
use serde_json::json;

use common::TestApp;

const PASSWORD: &str = "Sup3rSecret!!";

#[tokio::test]
async fn public_endpoints_are_available_without_auth() {
    let app = TestApp::spawn().await;

    let health = app
        .admin(Method::GET, "/api/v1/health", None, &[], None)
        .await;
    assert_eq!(health.status, StatusCode::OK);
    assert_eq!(health.body["status"], "ok");
    assert_eq!(health.body["initialized"], false);
    assert_eq!(health.body["fake_runtime"], true);

    let status = app
        .admin(Method::GET, "/api/v1/setup/status", None, &[], None)
        .await;
    assert_eq!(status.status, StatusCode::OK);
    assert_eq!(status.body["initialized"], false);
    assert!(status.body["completed_steps"].is_array());
}

#[tokio::test]
async fn protected_endpoints_require_authentication() {
    let app = TestApp::spawn().await;
    for path in [
        "/api/v1/system/overview",
        "/api/v1/instances",
        "/api/v1/profiles",
        "/api/v1/audit",
        "/api/v1/agent/providers",
        "/api/v1/network",
    ] {
        let response = app.admin(Method::GET, path, None, &[], None).await;
        assert_eq!(
            response.status,
            StatusCode::UNAUTHORIZED,
            "{path} must require authentication; body={}",
            response.text
        );
        assert_eq!(response.body["error"]["code"], "unauthorized");
    }
}

#[tokio::test]
async fn setup_admin_creates_session_and_enables_steps() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;

    let me = app
        .admin(
            Method::GET,
            "/api/v1/auth/me",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.body["user"]["username"], "admin");
    assert_eq!(me.body["user"]["role"], "admin");
    assert!(me.body["user"].get("password_hash").is_none());

    // Step 2 requires an authenticated admin once the account exists.
    let network = app
        .admin(
            Method::POST,
            "/api/v1/setup/network",
            Some(json!({
                "interface": "eth0",
                "mode": "dhcp",
                "static_ipv4": null,
                "dns": {
                    "enabled": true,
                    "engine": "adguard_home",
                    "provider": "quad9",
                    "protocol": "doh",
                    "custom_upstream": null,
                    "blocklists": ["https://example.com/blocklist.txt"]
                },
                "dmz": {
                    "enabled": false,
                    "bridge": "br-dmz",
                    "subnet": "10.20.30.0/24",
                    "gateway": "10.20.30.1",
                    "allow_dns": true
                }
            })),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(network.status, StatusCode::OK, "{}", network.text);
    assert_eq!(network.body["dns"]["provider"], "quad9");

    let setup_status = app
        .admin(Method::GET, "/api/v1/setup/status", None, &[], None)
        .await;
    assert_eq!(setup_status.body["completed_steps"], json!([1, 2]));
}

#[tokio::test]
async fn setup_admin_is_rejected_when_users_exist() {
    let app = TestApp::spawn().await;
    app.setup_admin("admin", PASSWORD).await;
    let second = app
        .admin(
            Method::POST,
            "/api/v1/setup/admin",
            Some(json!({
                "username": "other",
                "password": PASSWORD,
                "device": {"name": "Second", "language": "en", "timezone": "UTC"}
            })),
            &[],
            None,
        )
        .await;
    assert_eq!(second.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn login_logout_cycle_and_wrong_credentials() {
    let app = TestApp::spawn().await;
    app.setup_admin("admin", PASSWORD).await;

    let bad = app
        .admin(
            Method::POST,
            "/api/v1/auth/login",
            Some(json!({"username": "admin", "password": "WrongPassword1!"})),
            &[],
            None,
        )
        .await;
    assert_eq!(bad.status, StatusCode::UNAUTHORIZED);
    assert_eq!(bad.body["error"]["code"], "invalid_credentials");

    let session = app.login("admin", PASSWORD).await;
    let logout = app
        .admin(
            Method::POST,
            "/api/v1/auth/logout",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(logout.status, StatusCode::OK);

    let me = app
        .admin(
            Method::GET,
            "/api/v1/auth/me",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn cookie_sessions_require_the_csrf_header() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;

    let without = app
        .admin(
            Method::POST,
            "/api/v1/profiles",
            Some(json!({
                "name": "Mamá",
                "role": "adult",
                "avatar": {"shape": "squircle", "color": "#6ee7ff", "eye_color": "#0b1220", "eye_tilt_deg": 0.0},
                "language": "es",
                "tone": "friendly",
                "allowed_tools": ["music.play"]
            })),
            &session.cookies,
            None,
        )
        .await;
    assert_eq!(without.status, StatusCode::FORBIDDEN);

    let with = app
        .admin(
            Method::POST,
            "/api/v1/profiles",
            Some(json!({
                "name": "Mamá",
                "role": "adult",
                "avatar": {"shape": "squircle", "color": "#6ee7ff", "eye_color": "#0b1220", "eye_tilt_deg": 0.0},
                "language": "es",
                "tone": "friendly",
                "allowed_tools": ["music.play"]
            })),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(with.status, StatusCode::OK, "{}", with.text);
}

#[tokio::test]
async fn setup_endpoints_are_disabled_after_initialization() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;
    app.mark_initialized();

    let response = app
        .admin(
            Method::POST,
            "/api/v1/setup/network",
            Some(json!({"interface": "eth0", "mode": "dhcp", "static_ipv4": null,
                "dns": {"enabled": true, "engine": "adguard_home", "provider": "quad9", "protocol": "doh", "custom_upstream": null, "blocklists": []},
                "dmz": {"enabled": false, "bridge": "br-dmz", "subnet": "10.20.30.0/24", "gateway": "10.20.30.1", "allow_dns": true}})),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT);
    assert!(response.body["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("already initialized"));
}

#[tokio::test]
async fn kiosk_api_requires_the_kiosk_token() {
    let app = TestApp::spawn().await;
    let token = app.state.kiosk_token();

    let unauthorized = app
        .kiosk(Method::GET, "/api/v1/kiosk/state", None, None)
        .await;
    assert_eq!(unauthorized.status, StatusCode::UNAUTHORIZED);

    let authorized = app
        .kiosk(Method::GET, "/api/v1/kiosk/state", None, Some(&token))
        .await;
    assert_eq!(authorized.status, StatusCode::OK, "{}", authorized.text);
    assert_eq!(authorized.body["device_name"], "HomeServer Assistant");
    assert!(authorized.body["profile"].is_null());

    let query = app
        .kiosk(
            Method::POST,
            "/api/v1/kiosk/agent/query",
            Some(json!({"text": "hola"})),
            Some(&token),
        )
        .await;
    assert_eq!(query.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn rbac_prevents_viewer_from_managing_instances() {
    let app = TestApp::spawn().await;
    let admin = app.setup_admin("admin", PASSWORD).await;

    let created = app
        .admin(
            Method::POST,
            "/api/v1/users",
            Some(json!({
                "username": "viewer",
                "password": PASSWORD,
                "display_name": "Viewer",
                "role": "viewer"
            })),
            &admin.cookies,
            Some(&admin.csrf),
        )
        .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);

    let viewer = app.login("viewer", PASSWORD).await;
    let denied = app
        .admin(
            Method::POST,
            "/api/v1/instances",
            Some(json!({
                "name": "test-box",
                "kind": "container",
                "image": "images:debian/13",
                "segment": "lan",
                "limits": {"cpu_limit": 1.0, "memory_mb": 512},
                "env": {},
                "start": true
            })),
            &viewer.cookies,
            Some(&viewer.csrf),
        )
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(denied.body["error"]["code"], "forbidden");

    // The viewer can still read.
    let list = app
        .admin(
            Method::GET,
            "/api/v1/instances",
            None,
            &viewer.cookies,
            Some(&viewer.csrf),
        )
        .await;
    assert_eq!(list.status, StatusCode::OK);
}

#[tokio::test]
async fn mock_runtime_exposes_demo_instances_and_lifecycle() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;

    let list = app
        .admin(
            Method::GET,
            "/api/v1/instances",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    assert_eq!(list.body["provider"]["name"], "mock");
    assert_eq!(list.body["provider"]["available"], true);
    let instances = list.body["instances"].as_array().expect("instance list");
    assert!(instances.len() >= 2, "demo instances must be seeded");

    let created = app
        .admin(
            Method::POST,
            "/api/v1/instances",
            Some(json!({
                "name": "test-box",
                "kind": "container",
                "image": "images:debian/13",
                "segment": "lan",
                "limits": {"cpu_limit": 1.0, "memory_mb": 512},
                "env": {"FOO": "bar"},
                "start": true
            })),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    assert_eq!(created.body["name"], "test-box");
    assert_eq!(created.body["status"], "running");

    let stopped = app
        .admin(
            Method::POST,
            "/api/v1/instances/test-box/stop",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(stopped.status, StatusCode::OK);
    assert_eq!(stopped.body["status"], "stopped");

    let snapshot = app
        .admin(
            Method::POST,
            "/api/v1/instances/test-box/snapshots",
            Some(json!({"name": "before-upgrade"})),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(snapshot.status, StatusCode::OK, "{}", snapshot.text);

    let deleted = app
        .admin(
            Method::DELETE,
            "/api/v1/instances/test-box",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(deleted.status, StatusCode::OK);
}

#[tokio::test]
async fn network_firewall_ruleset_enforces_dmz_isolation() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;

    let response = app
        .admin(
            Method::GET,
            "/api/v1/network/firewall",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text);
    let ruleset = response.body["ruleset"].as_str().expect("ruleset string");
    assert!(
        ruleset.contains("br-dmz"),
        "ruleset must reference the DMZ bridge"
    );
    assert!(ruleset.contains("drop"), "ruleset must contain drop rules");

    let providers = app
        .admin(
            Method::GET,
            "/api/v1/network/dns/providers",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(providers.status, StatusCode::OK);
    let providers = providers.body.as_array().expect("providers");
    assert!(
        providers.len() >= 40,
        "the catalog must expose all researched variants"
    );
    assert!(providers
        .iter()
        .any(|entry| entry["id"] == "cloudflare_family"));
    let cloudflare = providers
        .iter()
        .find(|entry| entry["id"] == "cloudflare")
        .expect("cloudflare entry");
    assert!(cloudflare["ipv4"]
        .as_array()
        .is_some_and(|list| !list.is_empty()));
    assert!(cloudflare["ipv6"]
        .as_array()
        .is_some_and(|list| !list.is_empty()));
    assert_eq!(cloudflare["doh"], "https://cloudflare-dns.com/dns-query");
    assert_eq!(cloudflare["vendor"]["id"], "cloudflare");
    assert!(providers
        .iter()
        .any(|entry| entry["id"] == "controld_hagezi_ultimate"));
    assert!(providers.iter().any(|entry| entry["id"] == "comodo_secure"));
}

#[tokio::test]
async fn dns_provider_catalog_validates_transports_and_nextdns_id() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;

    let update = |provider: &'static str, protocol: &'static str, custom: Option<&'static str>| {
        json!({
            "enabled": true,
            "engine": "adguard_home",
            "provider": provider,
            "protocol": protocol,
            "custom_upstream": custom,
            "blocklists": []
        })
    };

    // New installations default to Cloudflare Family over DoH.
    let network = app
        .admin(
            Method::GET,
            "/api/v1/network",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(network.status, StatusCode::OK);
    assert_eq!(
        network.body["config"]["dns"]["provider"],
        "cloudflare_family"
    );
    assert_eq!(network.body["config"]["dns"]["protocol"], "doh");

    // Plain DNS is refused when the provider offers encryption.
    let plain_encrypted = app
        .admin(
            Method::PUT,
            "/api/v1/network/dns",
            Some(update("cloudflare_family", "plain", None)),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(
        plain_encrypted.status,
        StatusCode::BAD_REQUEST,
        "{}",
        plain_encrypted.text
    );

    // Unsupported transport for the provider.
    let rejected = app
        .admin(
            Method::PUT,
            "/api/v1/network/dns",
            Some(update("dnswatch", "dot", None)),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(
        rejected.status,
        StatusCode::BAD_REQUEST,
        "{}",
        rejected.text
    );

    // NextDNS needs the configuration id.
    let missing_id = app
        .admin(
            Method::PUT,
            "/api/v1/network/dns",
            Some(update("nextdns", "doh", None)),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(
        missing_id.status,
        StatusCode::BAD_REQUEST,
        "{}",
        missing_id.text
    );

    let nextdns = app
        .admin(
            Method::PUT,
            "/api/v1/network/dns",
            Some(update("nextdns", "doh", Some("abc123"))),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(nextdns.status, StatusCode::OK, "{}", nextdns.text);
    assert_eq!(nextdns.body["dns"]["provider"], "nextdns");

    // Legacy plain-only provider is allowed only with the plain transport.
    let plain = app
        .admin(
            Method::PUT,
            "/api/v1/network/dns",
            Some(update("comodo_secure", "plain", None)),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(plain.status, StatusCode::OK, "{}", plain.text);
    assert_eq!(plain.body["dns"]["protocol"], "plain");

    // A community Control D filter is accepted over DoH.
    let community = app
        .admin(
            Method::PUT,
            "/api/v1/network/dns",
            Some(update("controld_hagezi_ultimate", "doh", None)),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(community.status, StatusCode::OK, "{}", community.text);
}

#[tokio::test]
async fn profiles_can_be_created_and_activated_for_the_kiosk() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;

    let created = app
        .admin(
            Method::POST,
            "/api/v1/profiles",
            Some(json!({
                "name": "Mamá",
                "role": "adult",
                "avatar": {"shape": "droplet", "color": "#fbbf24", "eye_color": "#0b1220", "eye_tilt_deg": 8.0},
                "language": "es",
                "tone": "friendly",
                "allowed_tools": ["music.play", "homeassistant.call_service"]
            })),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    let profile_id = created.body["id"].as_str().expect("profile id").to_owned();

    let activated = app
        .admin(
            Method::POST,
            &format!("/api/v1/profiles/{profile_id}/activate"),
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(activated.status, StatusCode::OK);

    let kiosk_state = app
        .kiosk(
            Method::GET,
            "/api/v1/kiosk/state",
            None,
            Some(&app.state.kiosk_token()),
        )
        .await;
    assert_eq!(kiosk_state.status, StatusCode::OK);
    assert_eq!(kiosk_state.body["profile"]["name"], "Mamá");
    assert_eq!(kiosk_state.body["profile"]["avatar"]["shape"], "droplet");
}

#[tokio::test]
async fn audit_log_records_authentication_events() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;

    let audit = app
        .admin(
            Method::GET,
            "/api/v1/audit?limit=50",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(audit.status, StatusCode::OK, "{}", audit.text);
    let entries = audit.body.as_array().expect("audit entries");
    assert!(entries
        .iter()
        .any(|entry| entry["action"] == "auth.bootstrap_admin"));
    assert!(entries.iter().any(|entry| entry["action"] == "auth.login"));
}

#[tokio::test]
async fn firewall_allows_dmz_egress_with_nat_when_enabled() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;

    let configured = app
        .admin(
            Method::POST,
            "/api/v1/setup/network",
            Some(json!({
                "interface": "eth0",
                "lan_bridge": "br-lan",
                "mode": "dhcp",
                "static_ipv4": null,
                "dns": {
                    "enabled": true,
                    "engine": "adguard_home",
                    "provider": "cloudflare_family",
                    "protocol": "doh",
                    "custom_upstream": null,
                    "blocklists": []
                },
                "dmz": {
                    "enabled": true,
                    "bridge": "br-dmz",
                    "subnet": "10.20.30.0/24",
                    "gateway": "10.20.30.1",
                    "allow_dns": true
                }
            })),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(configured.status, StatusCode::OK, "{}", configured.text);

    let response = app
        .admin(
            Method::GET,
            "/api/v1/network/firewall",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text);
    let ruleset = response.body["ruleset"].as_str().expect("ruleset string");
    assert!(
        ruleset.contains("homeserver_nat"),
        "DMZ egress requires a NAT table"
    );
    assert!(ruleset.contains("masquerade"), "DMZ egress must be NATed");
    assert!(
        ruleset.contains("DMZ outbound internet"),
        "egress accept rule must be present"
    );
    let dmz_lan_drop = ruleset
        .find("DMZ must never reach LAN")
        .expect("DMZ->LAN drop rule");
    let egress_accept = ruleset
        .find("DMZ outbound internet (updates, tunnel)")
        .expect("egress accept rule");
    assert!(
        dmz_lan_drop < egress_accept,
        "the DMZ->LAN drop must be evaluated before egress"
    );
}

#[tokio::test]
async fn network_addressing_can_be_updated_and_apps_get_caddy_routes() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;

    let updated = app
        .admin(
            Method::PUT,
            "/api/v1/network",
            Some(json!({
                "interface": "eth0",
                "lan_bridge": "br-lan",
                "mode": "static",
                "static_ipv4": {
                    "address": "192.168.1.50",
                    "prefix_len": 24,
                    "gateway": "192.168.1.1"
                }
            })),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text);
    assert_eq!(updated.body["mode"], "static");
    assert_eq!(updated.body["static_ipv4"]["address"], "192.168.1.50");

    let deployed = app
        .admin(
            Method::POST,
            "/api/v1/apps",
            Some(json!({
                "name": "Nextcloud",
                "template_id": "nextcloud",
                "image": null,
                "segment": "lan",
                "internal_port": 80,
                "subdomain": "cloud",
                "expose_externally": false,
                "env": {}
            })),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(deployed.status, StatusCode::OK, "{}", deployed.text);
    let app_id = deployed.body["id"].as_str().expect("app id").to_owned();
    assert_eq!(deployed.body["hostname"], "cloud.home.local");

    let snippet = app
        .state
        .daemon
        .caddy_apps_dir
        .join(format!("{app_id}.caddy"));
    let content = std::fs::read_to_string(&snippet).expect("caddy snippet written");
    assert!(content.contains("cloud.home.local {"));
    assert!(content.contains("tls internal"));
    assert!(
        content.contains("reverse_proxy http://10.0.0."),
        "unexpected upstream: {content}"
    );
    assert!(content.contains(":80"));

    let removed = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/apps/{app_id}"),
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(removed.status, StatusCode::OK);
    assert!(
        !snippet.exists(),
        "removing the app must remove its Caddy site"
    );
}

#[tokio::test]
async fn voice_assistant_name_is_configurable_and_wake_word_activates_the_kiosk() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;

    // Defaults: always-on wake word enabled, customizable name.
    let config = app
        .admin(
            Method::GET,
            "/api/v1/voice/config",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(config.status, StatusCode::OK, "{}", config.text);
    assert_eq!(config.body["config"]["assistant_name"], "Aura");
    assert_eq!(config.body["config"]["wake_word_enabled"], true);
    assert_eq!(config.body["config"]["activation_mode"], "both");

    // Customize the name.
    let updated = app
        .admin(
            Method::PUT,
            "/api/v1/voice/config",
            Some(json!({
                "assistant_name": "Jarvis",
                "wake_word_enabled": true,
                "activation_mode": "both",
                "sensitivity": 0.7,
                "listening_window_secs": 10,
                "language": "en"
            })),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text);
    assert_eq!(updated.body["config"]["assistant_name"], "Jarvis");

    // The kiosk mirrors the configured name.
    let kiosk_state = app
        .kiosk(
            Method::GET,
            "/api/v1/kiosk/state",
            None,
            Some(&app.state.kiosk_token()),
        )
        .await;
    assert_eq!(kiosk_state.status, StatusCode::OK);
    assert_eq!(kiosk_state.body["assistant_name"], "Jarvis");
    assert_eq!(kiosk_state.body["voice"]["wake_word_enabled"], true);

    // A local transcript calling the name activates the listening window.
    let wake = app
        .kiosk(
            Method::POST,
            "/api/v1/kiosk/wake",
            Some(json!({"transcript": "oye Jarvis enciende la luz", "confidence": 0.92})),
            Some(&app.state.kiosk_token()),
        )
        .await;
    assert_eq!(wake.status, StatusCode::OK, "{}", wake.text);
    assert_eq!(wake.body["activated"], true);
    assert_eq!(wake.body["reason"], "activated");
    assert_eq!(wake.body["trailing_query"], "enciende la luz");

    // Non-matching speech does not activate.
    let ignored = app
        .kiosk(
            Method::POST,
            "/api/v1/kiosk/wake",
            Some(json!({"transcript": "hola, ¿qué tiempo hace?"})),
            Some(&app.state.kiosk_token()),
        )
        .await;
    assert_eq!(ignored.status, StatusCode::OK);
    assert_eq!(ignored.body["activated"], false);
    assert_eq!(ignored.body["reason"], "not_matched");

    // Invalid names are rejected.
    let invalid = app
        .admin(
            Method::PUT,
            "/api/v1/voice/config",
            Some(json!({
                "assistant_name": "x",
                "wake_word_enabled": true,
                "activation_mode": "both",
                "sensitivity": 0.5,
                "listening_window_secs": 8,
                "language": "es"
            })),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);

    // Wake activations are audited.
    let audit = app
        .admin(
            Method::GET,
            "/api/v1/audit?limit=50",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert!(audit
        .body
        .as_array()
        .is_some_and(|entries| entries.iter().any(|entry| entry["action"] == "voice.wake")));
}

#[tokio::test]
async fn disabled_wake_word_ignores_transcripts() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;
    let updated = app
        .admin(
            Method::PUT,
            "/api/v1/voice/config",
            Some(json!({
                "assistant_name": "Aura",
                "wake_word_enabled": false,
                "activation_mode": "push_to_talk",
                "sensitivity": 0.6,
                "listening_window_secs": 8,
                "language": "es"
            })),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK);

    let wake = app
        .kiosk(
            Method::POST,
            "/api/v1/kiosk/wake",
            Some(json!({"transcript": "oye aura"})),
            Some(&app.state.kiosk_token()),
        )
        .await;
    assert_eq!(wake.status, StatusCode::OK);
    assert_eq!(wake.body["activated"], false);
    assert_eq!(wake.body["reason"], "wake_disabled");
}

#[tokio::test]
async fn system_endpoints_report_host_metrics() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;

    let overview = app
        .admin(
            Method::GET,
            "/api/v1/system/overview",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(overview.status, StatusCode::OK, "{}", overview.text);
    assert!(overview.body["cpu"]["usage_percent"].is_number());
    assert!(overview.body["memory"]["total_mb"].is_number());
    assert!(overview.body["services"].is_array());

    let peripherals = app
        .admin(
            Method::GET,
            "/api/v1/system/peripherals",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(peripherals.status, StatusCode::OK);
    assert!(peripherals.body["camera_devices"].is_array());
    assert!(peripherals.body["touch_devices"].is_array());
}

#[tokio::test]
async fn home_assistant_status_is_reported_when_unconfigured() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;

    let status = app
        .admin(
            Method::GET,
            "/api/v1/homeassistant/status",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(status.status, StatusCode::OK, "{}", status.text);
    assert_eq!(status.body["configured"], false);
    assert_eq!(status.body["running"], false);

    let entities = app
        .admin(
            Method::GET,
            "/api/v1/homeassistant/entities",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(entities.status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn agent_endpoints_expose_tools_and_provider_validation() {
    let app = TestApp::spawn().await;
    let session = app.setup_admin("admin", PASSWORD).await;

    let tools = app
        .admin(
            Method::GET,
            "/api/v1/agent/tools",
            None,
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(tools.status, StatusCode::OK, "{}", tools.text);
    let tools = tools.body.as_array().expect("tools");
    assert_eq!(tools.len(), 10);
    assert!(tools.iter().any(|tool| tool["name"] == "music.play"));

    let invalid = app
        .admin(
            Method::POST,
            "/api/v1/agent/providers",
            Some(json!({
                "kind": "custom",
                "label": "No URL",
                "model": "some-model",
                "enabled": true
            })),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);

    let created = app
        .admin(
            Method::POST,
            "/api/v1/agent/providers",
            Some(json!({
                "kind": "openai",
                "label": "OpenAI",
                "model": "gpt-4.1-mini",
                "api_key": "sk-test-not-real",
                "enabled": true
            })),
            &session.cookies,
            Some(&session.csrf),
        )
        .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    assert!(created.body.get("api_key").is_none());
    let secret_name = format!(
        "llm.provider.{}.api_key",
        created.body["id"].as_str().unwrap_or_default()
    );
    assert!(app
        .state
        .secrets
        .contains(&secret_name)
        .expect("secret lookup"));
}
