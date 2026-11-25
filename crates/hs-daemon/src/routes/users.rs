//! Administrator account management (admin-only).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use hs_auth::{AuthContext, AuthService, Permission};
use hs_core::models::{AuditOutcome, Role, User};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::dto::UserView;
use crate::error::{HttpError, HttpResult};
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;

/// User management routes.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/users", get(list_users).post(create_user))
        .route(
            "/users/{id}",
            get(get_user).patch(update_user).delete(delete_user),
        )
        .route("/users/{id}/password", post(reset_password))
}

/// Request body to create an administrator account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserRequest {
    /// Login name.
    pub username: String,
    /// Initial password (policy enforced server-side).
    pub password: String,
    /// Display name; defaults to the username.
    #[serde(default)]
    pub display_name: String,
    /// RBAC role.
    pub role: Role,
}

/// Request body to update an account.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateUserRequest {
    /// New role.
    #[serde(default)]
    pub role: Option<Role>,
    /// Enable/disable the account.
    #[serde(default)]
    pub disabled: Option<bool>,
}

/// Password reset body (administrator resets another account).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetPasswordRequest {
    /// New password.
    pub new_password: String,
}

async fn list_users(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<Vec<UserView>>> {
    auth.require(Permission::UsersManage)?;
    let users = state.auth.list_users(&auth.0).map_err(HttpError::from)?;
    Ok(Json(users.iter().map(UserView::from).collect()))
}

async fn get_user(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
) -> HttpResult<Json<UserView>> {
    auth.require(Permission::UsersManage)?;
    let users = state.auth.list_users(&auth.0).map_err(HttpError::from)?;
    let user = users
        .into_iter()
        .find(|user| user.id == id)
        .ok_or_else(|| HttpError::not_found(format!("user {id}")))?;
    Ok(Json(UserView::from(&user)))
}

async fn create_user(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(request): Json<CreateUserRequest>,
) -> HttpResult<Json<UserView>> {
    auth.require(Permission::UsersManage)?;
    // Argon2 hashing is CPU intensive: keep it off the async worker.
    let service: Arc<AuthService> = Arc::clone(&state.auth);
    let context: AuthContext = auth.0.clone();
    let user: User = tokio::task::spawn_blocking(move || {
        service.create_user(
            &context,
            &request.username,
            &request.password,
            &request.display_name,
            request.role,
        )
    })
    .await
    .map_err(|err| HttpError::internal(format!("user creation task failed: {err}")))??;
    state.audit(
        &auth.user().username,
        "users.create",
        Some(&user.id.to_string()),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"username": user.username, "role": user.role.id()})),
    );
    Ok(Json(UserView::from(&user)))
}

async fn update_user(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateUserRequest>,
) -> HttpResult<Json<UserView>> {
    auth.require(Permission::UsersManage)?;
    let mut user = if let Some(role) = request.role {
        state
            .auth
            .update_role(&auth.0, id, role)
            .map_err(HttpError::from)?
    } else {
        let users = state.auth.list_users(&auth.0).map_err(HttpError::from)?;
        users
            .into_iter()
            .find(|user| user.id == id)
            .ok_or_else(|| HttpError::not_found(format!("user {id}")))?
    };
    if let Some(disabled) = request.disabled {
        user = state
            .auth
            .set_disabled(&auth.0, id, disabled)
            .map_err(HttpError::from)?;
    }
    state.audit(
        &auth.user().username,
        "users.update",
        Some(&user.id.to_string()),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"role": user.role.id(), "disabled": user.disabled})),
    );
    Ok(Json(UserView::from(&user)))
}

async fn delete_user(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::UsersManage)?;
    state
        .auth
        .delete_user(&auth.0, id)
        .map_err(HttpError::from)?;
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn reset_password(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
    Json(request): Json<ResetPasswordRequest>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::UsersManage)?;
    let service: Arc<AuthService> = Arc::clone(&state.auth);
    let context: AuthContext = auth.0.clone();
    tokio::task::spawn_blocking(move || {
        service.reset_password(&context, id, &request.new_password)
    })
    .await
    .map_err(|err| HttpError::internal(format!("password reset task failed: {err}")))??;
    Ok(Json(serde_json::json!({"ok": true})))
}
