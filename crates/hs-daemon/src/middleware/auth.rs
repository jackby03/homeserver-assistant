//! Authentication extractors and CSRF enforcement.
//!
//! [`AdminAuth`] is an axum extractor that authenticates a request from either
//! the `hs_session` HttpOnly cookie or an `Authorization: Bearer` header. For
//! cookie-authenticated *mutating* requests it also enforces the double-submit
//! CSRF header. Handlers then call `auth.require(Permission::...)`.

use axum::extract::{FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use hs_auth::{AuthContext, Permission};

use crate::error::HttpError;
use crate::state::SharedState;

/// Extractor yielding the authenticated user context.
#[derive(Debug, Clone)]
pub struct AdminAuth(pub AuthContext);

impl FromRequestParts<SharedState> for AdminAuth {
    type Rejection = HttpError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, Self::Rejection> {
        let (token, from_cookie) = extract_token(parts)?;

        let context = state
            .auth
            .authenticate(&token)
            .map_err(|_| HttpError::unauthorized("session is missing, expired or revoked"))?;

        if from_cookie && is_mutating(parts.method.clone()) {
            let provided = parts
                .headers
                .get("x-hs-csrf")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    HttpError::forbidden("missing X-HS-CSRF header for cookie session")
                })?;
            if !state.auth.verify_csrf(&context, provided) {
                state.audit(
                    &context.user.username,
                    "auth.csrf",
                    None,
                    hs_core::models::AuditOutcome::Denied,
                    None,
                    Some(serde_json::json!({"path": parts.uri.path()})),
                );
                return Err(HttpError::forbidden("invalid CSRF token"));
            }
        }

        Ok(Self(context))
    }
}

impl AdminAuth {
    /// Check a permission, returning an HTTP 403 when missing.
    ///
    /// # Errors
    /// Returns [`HttpError`] with 403 when the role lacks the permission.
    pub fn require(&self, permission: Permission) -> Result<(), HttpError> {
        self.0.require(permission).map_err(HttpError::from)
    }

    /// Authenticated user.
    #[must_use]
    pub fn user(&self) -> &hs_core::models::User {
        &self.0.user
    }
}

fn is_mutating(method: Method) -> bool {
    matches!(
        method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

fn extract_token(parts: &Parts) -> Result<(String, bool), HttpError> {
    if let Some(header) = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        if let Some(token) = header.strip_prefix("Bearer ") {
            if !token.trim().is_empty() {
                return Ok((token.trim().to_owned(), false));
            }
        }
    }

    if let Some(cookie_header) = parts
        .headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
    {
        for cookie in cookie_header.split(';') {
            if let Some((name, value)) = cookie.trim().split_once('=') {
                if name == "hs_session" && !value.is_empty() {
                    return Ok((value.to_owned(), true));
                }
            }
        }
    }

    Err(HttpError::unauthorized(
        "this endpoint requires an authenticated session",
    ))
}

/// Middleware that rejects setup endpoints once the appliance is initialized.
pub async fn setup_only(
    state: axum::extract::State<SharedState>,
    request: Request,
    next: Next,
) -> Result<Response, HttpError> {
    if state.is_initialized() {
        return Err(HttpError::conflict(
            "the appliance is already initialized; setup endpoints are disabled",
        ));
    }
    Ok(next.run(request).await)
}

/// Middleware that rejects admin endpoints that are not reachable from loopback.
pub async fn loopback_only(request: Request, next: Next) -> Result<Response, HttpError> {
    if let Some(connect_info) = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
    {
        if !connect_info.0.ip().is_loopback() {
            return Err(HttpError::forbidden(
                "this endpoint is only available from the local machine",
            ));
        }
    }
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutating_methods_require_csrf() {
        assert!(is_mutating(Method::POST));
        assert!(is_mutating(Method::PUT));
        assert!(is_mutating(Method::PATCH));
        assert!(is_mutating(Method::DELETE));
        assert!(!is_mutating(Method::GET));
        assert!(!is_mutating(Method::HEAD));
    }
}
