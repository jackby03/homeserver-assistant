//! Authentication routes: login, logout, session introspection, password change.

use std::sync::Arc;

use crate::cookies::{cleared_cookie, csrf_cookie, header_value, is_https, session_cookie};
use crate::dto::{AckResponse, SessionResponse, UserView};
use crate::error::{HttpError, HttpResult};
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hs_auth::{AuthContext, AuthService};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Authentication routes.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
        .route("/auth/password", post(change_password))
}

/// Login credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    /// Login name (case-insensitive).
    pub username: String,
    /// Plaintext password.
    pub password: String,
}

/// Password change request for the authenticated user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    /// Current password.
    pub current_password: String,
    /// New password (validated server-side).
    pub new_password: String,
}

/// Extract the best-effort client address for audit/throttling.
#[must_use]
pub fn client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(256).collect())
}

/// `POST /api/v1/auth/login`
async fn login(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> HttpResult<Response> {
    if request.username.trim().is_empty() || request.password.is_empty() {
        return Err(HttpError::bad_request("username and password are required"));
    }

    let auth: Arc<AuthService> = Arc::clone(&state.auth);
    let username = request.username.clone();
    let password = request.password.clone();
    let ip = client_ip(&headers);
    let ua = user_agent(&headers);

    let result = tokio::task::spawn_blocking(move || {
        auth.login(&username, &password, ip.as_deref(), ua.as_deref())
    })
    .await
    .map_err(|err| HttpError::internal(format!("login task failed: {err}")))?;

    let login = match result {
        Ok(login) => login,
        Err(err) => {
            warn!(error = %err, username = %request.username, "login failed");
            return Err(HttpError::from(err));
        }
    };

    info!(
        user = %login.user.username,
        role = login.user.role.id(),
        "administrator logged in"
    );
    Ok(build_session_response(
        &headers,
        login.token,
        login.csrf_token,
        &login.user,
    ))
}

/// `POST /api/v1/auth/logout`
async fn logout(
    State(state): State<SharedState>,
    auth: AdminAuth,
    headers: HeaderMap,
) -> HttpResult<Response> {
    state.auth.logout(&auth.0).map_err(HttpError::from)?;
    let secure = is_https(&headers);
    let mut response = Json(AckResponse::ok()).into_response();
    append_clear_cookies(&mut response, secure);
    Ok(response)
}

/// `GET /api/v1/auth/me`
async fn me(State(state): State<SharedState>, auth: AdminAuth) -> Json<SessionResponse> {
    let _ = state;
    Json(SessionResponse {
        user: UserView::from(&auth.0.user),
        csrf_token: auth.0.csrf_token.clone(),
    })
}

/// `POST /api/v1/auth/password`
async fn change_password(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(request): Json<ChangePasswordRequest>,
) -> HttpResult<Json<AckResponse>> {
    let service = Arc::clone(&state.auth);
    let context: AuthContext = auth.0.clone();
    let current = request.current_password;
    let new_password = request.new_password;
    tokio::task::spawn_blocking(move || {
        service.change_own_password(&context, &current, &new_password)
    })
    .await
    .map_err(|err| HttpError::internal(format!("password task failed: {err}")))??;
    Ok(Json(AckResponse::ok()))
}

/// Build the JSON response and set the session + CSRF cookies.
pub fn build_session_response(
    headers: &HeaderMap,
    token: String,
    csrf_token: String,
    user: &hs_core::models::User,
) -> Response {
    let secure = is_https(headers);
    let session = header_value(session_cookie(&token, secure));
    let csrf = header_value(csrf_cookie(&csrf_token, secure));
    let mut response = Json(SessionResponse {
        user: UserView::from(user),
        csrf_token,
    })
    .into_response();
    response
        .headers_mut()
        .append(axum::http::header::SET_COOKIE, session);
    response
        .headers_mut()
        .append(axum::http::header::SET_COOKIE, csrf);
    response
}

/// Append cleared session cookies to a response.
pub fn append_clear_cookies(response: &mut Response, secure: bool) {
    response.headers_mut().append(
        axum::http::header::SET_COOKIE,
        cleared_cookie(crate::cookies::SESSION_COOKIE, secure),
    );
    response.headers_mut().append(
        axum::http::header::SET_COOKIE,
        cleared_cookie(crate::cookies::CSRF_COOKIE, secure),
    );
    response.headers_mut().append(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
}
