//! Household profile routes: CRUD, avatar, permissions and biometric enrollment.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use hs_auth::Permission;
use hs_core::models::{AuditOutcome, Profile};
use hs_profiles::{CreateProfileRequest, UpdateProfileRequest};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{HttpError, HttpResult};
use crate::middleware::auth::AdminAuth;
use crate::state::SharedState;

/// Profile routes.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/profiles", get(list_profiles).post(create_profile))
        .route("/profiles/defaults", get(default_profiles))
        .route(
            "/profiles/{id}",
            get(get_profile).put(update_profile).delete(delete_profile),
        )
        .route("/profiles/{id}/activate", post(activate_profile))
        .route("/profiles/{id}/tools", axum::routing::put(set_tools))
        .route("/profiles/{id}/enroll", post(enroll_face))
        .route("/profiles/{id}/face", axum::routing::delete(remove_face))
}

/// Tools update payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetToolsRequest {
    /// Tool names the profile may use.
    pub allowed_tools: Vec<String>,
}

/// `GET /api/v1/profiles/defaults` — suggested household profiles for the
/// onboarding wizard. Returns request bodies, not persisted profiles.
async fn default_profiles(auth: AdminAuth) -> HttpResult<Json<Vec<CreateProfileRequest>>> {
    auth.require(Permission::ProfilesRead)?;
    Ok(Json(hs_profiles::ProfileService::default_profiles()))
}

/// `PUT /api/v1/profiles/{id}/tools`
async fn set_tools(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
    Json(request): Json<SetToolsRequest>,
) -> HttpResult<Json<Profile>> {
    auth.require(Permission::ProfilesManage)?;
    let profile = state
        .profiles
        .set_allowed_tools(id, request.allowed_tools)
        .map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "profile.set_tools",
        Some(&profile.id.to_string()),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"count": profile.allowed_tools.len()})),
    );
    Ok(Json(profile))
}

/// Biometric enrollment payload (embedding produced locally by the vision helper).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollFaceRequest {
    /// Face embedding vector.
    pub embedding: Vec<f32>,
}

async fn list_profiles(
    State(state): State<SharedState>,
    auth: AdminAuth,
) -> HttpResult<Json<Vec<Profile>>> {
    auth.require(Permission::ProfilesRead)?;
    Ok(Json(state.profiles.list().map_err(HttpError::from)?))
}

async fn get_profile(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
) -> HttpResult<Json<Profile>> {
    auth.require(Permission::ProfilesRead)?;
    Ok(Json(state.profiles.get(id).map_err(HttpError::from)?))
}

async fn create_profile(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Json(request): Json<CreateProfileRequest>,
) -> HttpResult<Json<Profile>> {
    auth.require(Permission::ProfilesManage)?;
    let profile = state.profiles.create(request).map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "profile.create",
        Some(&profile.id.to_string()),
        AuditOutcome::Success,
        None,
        Some(serde_json::json!({"name": profile.name, "role": profile.role.id()})),
    );
    Ok(Json(profile))
}

async fn update_profile(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateProfileRequest>,
) -> HttpResult<Json<Profile>> {
    auth.require(Permission::ProfilesManage)?;
    let profile = state
        .profiles
        .update(id, request)
        .map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "profile.update",
        Some(&profile.id.to_string()),
        AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(profile))
}

async fn delete_profile(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
) -> HttpResult<Json<serde_json::Value>> {
    auth.require(Permission::ProfilesManage)?;
    state.profiles.delete(id).map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "profile.delete",
        Some(&id.to_string()),
        AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn activate_profile(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
) -> HttpResult<Json<Profile>> {
    auth.require(Permission::KioskControl)?;
    let profile = state.profiles.get(id).map_err(HttpError::from)?;
    state.kiosk.activate_profile(profile.clone());
    state.audit(
        &auth.user().username,
        "profile.activate",
        Some(&id.to_string()),
        AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(profile))
}

async fn enroll_face(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
    Json(request): Json<EnrollFaceRequest>,
) -> HttpResult<Json<Profile>> {
    auth.require(Permission::ProfilesManage)?;
    let profile = state
        .profiles
        .enroll_face(id, request.embedding)
        .map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "profile.enroll_face",
        Some(&id.to_string()),
        AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(profile))
}

async fn remove_face(
    State(state): State<SharedState>,
    auth: AdminAuth,
    Path(id): Path<Uuid>,
) -> HttpResult<Json<Profile>> {
    auth.require(Permission::ProfilesManage)?;
    let profile = state.profiles.remove_face(id).map_err(HttpError::from)?;
    state.audit(
        &auth.user().username,
        "profile.remove_face",
        Some(&id.to_string()),
        AuditOutcome::Success,
        None,
        None,
    );
    Ok(Json(profile))
}
