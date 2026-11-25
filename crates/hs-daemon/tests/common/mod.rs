//! Shared test harness for `hs-daemon` integration tests.
//!
//! Builds a fully wired daemon in development mode (mock runtime + fake
//! hardware) inside a temporary data directory and exposes helpers to call the
//! axum routers with `tower::ServiceExt::oneshot` — no sockets, no root, no
//! network.

#![allow(dead_code)]

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use hs_daemon::{DaemonConfig, SharedState};

/// Test application with both routers.
pub struct TestApp {
    /// Temporary directory (data + static assets).
    pub dir: tempfile::TempDir,
    /// Shared daemon state, for direct assertions/manipulation.
    pub state: SharedState,
    admin: axum::Router,
    kiosk: axum::Router,
}

/// HTTP response captured by the helpers.
pub struct TestResponse {
    /// Status code.
    pub status: StatusCode,
    /// Parsed JSON body (or `Null` when the body is empty/not JSON).
    pub body: Value,
    /// Raw body text.
    pub text: String,
    /// `Set-Cookie` pairs in `name=value` form.
    pub cookies: Vec<String>,
}

impl TestApp {
    /// Boot the daemon in development mode.
    pub async fn spawn() -> Self {
        let dir = tempfile::tempdir().expect("temporary directory");
        let data_dir = dir.path().join("data");
        let config = DaemonConfig::resolve(
            Some(data_dir),
            Some("127.0.0.1:0".parse().expect("valid admin bind")),
            Some("127.0.0.1:0".parse().expect("valid kiosk bind")),
            Some(dir.path().join("ui")),
            Some(dir.path().join("vision.sock")),
            true,
            true,
            false,
        )
        .expect("daemon config");
        let state = hs_daemon::bootstrap::build_state(config)
            .await
            .expect("app state");
        let admin = hs_daemon::router::admin_router(state.clone());
        let kiosk = hs_daemon::router::kiosk_router(state.clone());
        Self {
            dir,
            state,
            admin,
            kiosk,
        }
    }

    /// Send a request to the admin router.
    pub async fn admin(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        cookies: &[String],
        csrf: Option<&str>,
    ) -> TestResponse {
        send(&self.admin, method, path, body, cookies, csrf).await
    }

    /// Send a request to the kiosk router with an optional kiosk token.
    pub async fn kiosk(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        kiosk_token: Option<&str>,
    ) -> TestResponse {
        let mut headers = Vec::new();
        if let Some(token) = kiosk_token {
            headers.push(("x-kiosk-token", token.to_owned()));
        }
        send_with_headers(&self.kiosk, method, path, body, headers).await
    }

    /// Complete step 1 of onboarding and return the session.
    pub async fn setup_admin(&self, username: &str, password: &str) -> Session {
        let response = self
            .admin(
                Method::POST,
                "/api/v1/setup/admin",
                Some(json!({
                    "username": username,
                    "password": password,
                    "device": {
                        "name": "Test Home",
                        "language": "en",
                        "timezone": "UTC"
                    }
                })),
                &[],
                None,
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "setup/admin failed: {}",
            response.text
        );
        Session {
            cookies: response.cookies,
            csrf: response.body["csrf_token"]
                .as_str()
                .expect("csrf token")
                .to_owned(),
        }
    }

    /// Log in and return the session cookies + CSRF token.
    pub async fn login(&self, username: &str, password: &str) -> Session {
        let response = self
            .admin(
                Method::POST,
                "/api/v1/auth/login",
                Some(json!({"username": username, "password": password})),
                &[],
                None,
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "login failed: {}",
            response.text
        );
        Session {
            cookies: response.cookies,
            csrf: response.body["csrf_token"]
                .as_str()
                .expect("csrf token")
                .to_owned(),
        }
    }

    /// Mark onboarding as finished (bypasses the deployment job).
    pub fn mark_initialized(&self) {
        self.state
            .config
            .update(|config| {
                config.initialized = true;
                Ok(())
            })
            .expect("mark initialized");
    }
}

/// Authenticated session used by tests.
pub struct Session {
    /// Cookie pairs (`name=value`).
    pub cookies: Vec<String>,
    /// CSRF token bound to the session.
    pub csrf: String,
}

async fn send(
    router: &axum::Router,
    method: Method,
    path: &str,
    body: Option<Value>,
    cookies: &[String],
    csrf: Option<&str>,
) -> TestResponse {
    let mut headers = Vec::new();
    if !cookies.is_empty() {
        headers.push(("cookie", cookies.join("; ")));
    }
    if let Some(csrf) = csrf {
        headers.push(("x-hs-csrf", csrf.to_owned()));
    }
    send_with_headers(router, method, path, body, headers).await
}

async fn send_with_headers(
    router: &axum::Router,
    method: Method,
    path: &str,
    body: Option<Value>,
    headers: Vec<(&str, String)>,
) -> TestResponse {
    let mut builder = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let payload = body
        .map(|value| Body::from(serde_json::to_vec(&value).expect("json body")))
        .unwrap_or_else(Body::empty);
    let request = builder.body(payload).expect("request");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("router response");
    let status = response.status();
    let cookies = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .map(|pair| pair.trim().to_owned())
        .filter(|pair| !pair.ends_with('='))
        .collect::<Vec<_>>();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    TestResponse {
        status,
        body,
        text,
        cookies,
    }
}
