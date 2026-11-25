//! Authentication service: accounts, sessions, CSRF, RBAC and audit.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use hs_core::models::{AuditEntry, AuditOutcome, Role, SessionRecord, User};
use hs_core::state::Database;
use hs_core::validation;
use serde_json::Value;
use uuid::Uuid;

use crate::error::{AuthError, AuthResult};
use crate::jwt::JwtService;
use crate::password::{hash_password, verify_password};
use crate::rbac::{role_can, Permission};

/// Authentication tuning knobs.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// JWT issuer.
    pub issuer: String,
    /// Session lifetime.
    pub session_ttl: Duration,
    /// Name of the session cookie.
    pub cookie_name: String,
    /// Name of the CSRF cookie (readable by JavaScript).
    pub csrf_cookie_name: String,
    /// Failed logins allowed inside the window.
    pub max_login_attempts: usize,
    /// Sliding window for the login throttle.
    pub login_window: Duration,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            issuer: "homecore-daemon".to_owned(),
            session_ttl: Duration::from_secs(12 * 60 * 60),
            cookie_name: "hs_session".to_owned(),
            csrf_cookie_name: "hs_csrf".to_owned(),
            max_login_attempts: 5,
            login_window: Duration::from_secs(300),
        }
    }
}

impl AuthConfig {
    /// Build from the number of hours configured for sessions.
    #[must_use]
    pub fn with_session_hours(hours: u32) -> Self {
        Self {
            session_ttl: Duration::from_secs(u64::from(hours.max(1)) * 3600),
            ..Self::default()
        }
    }
}

/// Successful login payload handed to the HTTP layer, which sets cookies.
#[derive(Debug, Clone)]
pub struct LoginResult {
    /// Authenticated user.
    pub user: User,
    /// Signed JWT to use as a bearer token or session cookie value.
    pub token: String,
    /// CSRF token bound to the created session.
    pub csrf_token: String,
    /// Session expiry.
    pub expires_at: DateTime<Utc>,
}

/// Authenticated request context.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// Live user record (role changes apply immediately).
    pub user: User,
    /// Session id used for revocation and CSRF.
    pub session_id: Uuid,
    /// CSRF token bound to the session.
    pub csrf_token: String,
}

impl AuthContext {
    /// Whether the current role grants `permission`.
    #[must_use]
    pub fn can(&self, permission: Permission) -> bool {
        role_can(self.user.role, permission)
    }

    /// Enforce a permission, returning [`AuthError::Forbidden`] otherwise.
    ///
    /// # Errors
    /// Returns [`AuthError::Forbidden`] when the role lacks the permission.
    pub fn require(&self, permission: Permission) -> AuthResult<()> {
        if self.can(permission) {
            Ok(())
        } else {
            Err(AuthError::Forbidden(format!(
                "role `{}` lacks permission `{}`",
                self.user.role.id(),
                permission.id()
            )))
        }
    }
}

/// Sliding-window login throttle. Kept per username+IP pair.
#[derive(Debug)]
struct LoginThrottle {
    attempts: HashMap<String, Vec<Instant>>,
    max_attempts: usize,
    window: Duration,
}

impl LoginThrottle {
    fn new(max_attempts: usize, window: Duration) -> Self {
        Self {
            attempts: HashMap::new(),
            max_attempts,
            window,
        }
    }

    fn allow(&mut self, key: &str, now: Instant) -> bool {
        let AttemptsAndLen { len } = self.prune(key, now);
        len < self.max_attempts
    }

    fn record_failure(&mut self, key: &str, now: Instant) {
        let AttemptsAndLen { .. } = self.prune(key, now);
        self.attempts.entry(key.to_owned()).or_default().push(now);
    }

    fn reset(&mut self, key: &str) {
        self.attempts.remove(key);
    }

    fn prune(&mut self, key: &str, now: Instant) -> AttemptsAndLen {
        let window = self.window;
        let entry = self.attempts.entry(key.to_owned()).or_default();
        entry.retain(|attempt| now.saturating_duration_since(*attempt) < window);
        AttemptsAndLen { len: entry.len() }
    }
}

#[derive(Debug, Clone, Copy)]
struct AttemptsAndLen {
    len: usize,
}

/// Authentication and authorization facade backed by the embedded database.
#[derive(Debug)]
pub struct AuthService {
    db: Arc<Database>,
    jwt: JwtService,
    config: AuthConfig,
    throttle: Mutex<LoginThrottle>,
}

impl AuthService {
    /// Create the service.
    #[must_use]
    pub fn new(db: Arc<Database>, jwt: JwtService, config: AuthConfig) -> Self {
        let throttle = Mutex::new(LoginThrottle::new(
            config.max_login_attempts,
            config.login_window,
        ));
        Self {
            db,
            jwt,
            config,
            throttle,
        }
    }

    /// Session cookie name.
    #[must_use]
    pub fn cookie_name(&self) -> &str {
        &self.config.cookie_name
    }

    /// CSRF cookie name.
    #[must_use]
    pub fn csrf_cookie_name(&self) -> &str {
        &self.config.csrf_cookie_name
    }

    /// Session TTL.
    #[must_use]
    pub fn session_ttl(&self) -> Duration {
        self.config.session_ttl
    }

    /// Whether at least one administrative user exists.
    ///
    /// # Errors
    /// Returns a storage error when the database cannot be read.
    pub fn has_users(&self) -> AuthResult<bool> {
        Ok(!self.db.list_users()?.is_empty())
    }

    /// Create the first administrator during first boot.
    ///
    /// # Errors
    /// Returns [`AuthError::Conflict`] when users already exist and
    /// [`AuthError::Validation`] for invalid credentials.
    pub fn bootstrap_admin(
        &self,
        username: &str,
        password: &str,
        display_name: &str,
    ) -> AuthResult<User> {
        if self.has_users()? {
            return Err(AuthError::Conflict(
                "an administrator account already exists".to_owned(),
            ));
        }
        let user = self.build_user(username, password, display_name, Role::Admin)?;
        self.db.save_user(&user)?;
        self.audit(
            "system",
            "auth.bootstrap_admin",
            Some(&user.id.to_string()),
            AuditOutcome::Success,
            None,
            None,
        )?;
        Ok(user)
    }

    /// Create an additional user. Requires `users.manage`.
    ///
    /// # Errors
    /// Returns [`AuthError::Forbidden`] without permission, [`AuthError::Conflict`]
    /// for duplicate usernames.
    pub fn create_user(
        &self,
        actor: &AuthContext,
        username: &str,
        password: &str,
        display_name: &str,
        role: Role,
    ) -> AuthResult<User> {
        actor.require(Permission::UsersManage)?;
        let user = self.build_user(username, password, display_name, role)?;
        self.db.save_user(&user)?;
        self.audit(
            &actor.user.username,
            "auth.create_user",
            Some(&user.id.to_string()),
            AuditOutcome::Success,
            None,
            Some(serde_json::json!({"username": user.username, "role": user.role.id()})),
        )?;
        Ok(user)
    }

    /// List users. Requires `users.manage`.
    ///
    /// # Errors
    /// Returns [`AuthError::Forbidden`] without permission.
    pub fn list_users(&self, actor: &AuthContext) -> AuthResult<Vec<User>> {
        actor.require(Permission::UsersManage)?;
        Ok(self.db.list_users()?)
    }

    /// Change a user's role. Requires `users.manage`.
    ///
    /// # Errors
    /// Returns [`AuthError::Conflict`] when demoting the last administrator.
    pub fn update_role(&self, actor: &AuthContext, user_id: Uuid, role: Role) -> AuthResult<User> {
        actor.require(Permission::UsersManage)?;
        let mut user = self
            .db
            .user_by_id(user_id)?
            .ok_or_else(|| AuthError::NotFound(format!("user {user_id}")))?;
        if user.role == Role::Admin && role != Role::Admin && self.admin_count()? <= 1 {
            return Err(AuthError::Conflict(
                "cannot demote the last administrator".to_owned(),
            ));
        }
        user.role = role;
        user.updated_at = Utc::now();
        self.db.save_user(&user)?;
        self.audit(
            &actor.user.username,
            "auth.update_role",
            Some(&user.id.to_string()),
            AuditOutcome::Success,
            None,
            Some(serde_json::json!({"role": role.id()})),
        )?;
        Ok(user)
    }

    /// Enable or disable an account. Requires `users.manage`.
    ///
    /// # Errors
    /// Returns [`AuthError::Conflict`] when disabling the last administrator or self.
    pub fn set_disabled(
        &self,
        actor: &AuthContext,
        user_id: Uuid,
        disabled: bool,
    ) -> AuthResult<User> {
        actor.require(Permission::UsersManage)?;
        if disabled && actor.user.id == user_id {
            return Err(AuthError::Conflict(
                "you cannot disable your own account".to_owned(),
            ));
        }
        let mut user = self
            .db
            .user_by_id(user_id)?
            .ok_or_else(|| AuthError::NotFound(format!("user {user_id}")))?;
        if disabled && user.role == Role::Admin && self.admin_count()? <= 1 {
            return Err(AuthError::Conflict(
                "cannot disable the last administrator".to_owned(),
            ));
        }
        user.disabled = disabled;
        user.updated_at = Utc::now();
        self.db.save_user(&user)?;
        if disabled {
            self.revoke_user_sessions(user_id)?;
        }
        self.audit(
            &actor.user.username,
            "auth.set_disabled",
            Some(&user.id.to_string()),
            AuditOutcome::Success,
            None,
            Some(serde_json::json!({"disabled": disabled})),
        )?;
        Ok(user)
    }

    /// Delete a user. Requires `users.manage`.
    ///
    /// # Errors
    /// Returns [`AuthError::Conflict`] for self-deletion or the last administrator.
    pub fn delete_user(&self, actor: &AuthContext, user_id: Uuid) -> AuthResult<()> {
        actor.require(Permission::UsersManage)?;
        if actor.user.id == user_id {
            return Err(AuthError::Conflict(
                "you cannot delete your own account".to_owned(),
            ));
        }
        let user = self
            .db
            .user_by_id(user_id)?
            .ok_or_else(|| AuthError::NotFound(format!("user {user_id}")))?;
        if user.role == Role::Admin && self.admin_count()? <= 1 {
            return Err(AuthError::Conflict(
                "cannot delete the last administrator".to_owned(),
            ));
        }
        self.revoke_user_sessions(user_id)?;
        self.db.delete_user(user_id)?;
        self.audit(
            &actor.user.username,
            "auth.delete_user",
            Some(&user.id.to_string()),
            AuditOutcome::Success,
            None,
            None,
        )?;
        Ok(())
    }

    /// Reset another user's password. Requires `users.manage`.
    ///
    /// # Errors
    /// Returns [`AuthError::Validation`] for a weak password.
    pub fn reset_password(
        &self,
        actor: &AuthContext,
        user_id: Uuid,
        new_password: &str,
    ) -> AuthResult<()> {
        actor.require(Permission::UsersManage)?;
        validation::password(new_password)?;
        let mut user = self
            .db
            .user_by_id(user_id)?
            .ok_or_else(|| AuthError::NotFound(format!("user {user_id}")))?;
        user.password_hash = hash_password(new_password)?;
        user.updated_at = Utc::now();
        self.db.save_user(&user)?;
        self.revoke_user_sessions(user_id)?;
        self.audit(
            &actor.user.username,
            "auth.reset_password",
            Some(&user.id.to_string()),
            AuditOutcome::Success,
            None,
            None,
        )?;
        Ok(())
    }

    /// Change the authenticated user's password, verifying the current one.
    ///
    /// # Errors
    /// Returns [`AuthError::InvalidCredentials`] when the current password is
    /// wrong.
    pub fn change_own_password(
        &self,
        actor: &AuthContext,
        current_password: &str,
        new_password: &str,
    ) -> AuthResult<()> {
        let mut user = self
            .db
            .user_by_id(actor.user.id)?
            .ok_or_else(|| AuthError::NotFound("current user".to_owned()))?;
        if !verify_password(current_password, &user.password_hash)? {
            self.audit(
                &actor.user.username,
                "auth.change_password",
                Some(&user.id.to_string()),
                AuditOutcome::Denied,
                None,
                None,
            )?;
            return Err(AuthError::InvalidCredentials);
        }
        validation::password(new_password)?;
        user.password_hash = hash_password(new_password)?;
        user.updated_at = Utc::now();
        self.db.save_user(&user)?;
        self.revoke_user_sessions_except(user.id, actor.session_id)?;
        self.audit(
            &actor.user.username,
            "auth.change_password",
            Some(&user.id.to_string()),
            AuditOutcome::Success,
            None,
            None,
        )?;
        Ok(())
    }

    /// Authenticate with username and password, creating a session.
    ///
    /// # Errors
    /// Returns [`AuthError::InvalidCredentials`] for wrong credentials (without
    /// distinguishing the cause), [`AuthError::TooManyAttempts`] when throttled.
    pub fn login(
        &self,
        username: &str,
        password: &str,
        ip: Option<&str>,
        user_agent: Option<&str>,
    ) -> AuthResult<LoginResult> {
        let normalized = username.trim().to_lowercase();
        let throttle_key = format!("{}|{}", normalized, ip.unwrap_or("-").trim());

        {
            let mut throttle = self
                .throttle
                .lock()
                .map_err(|_| AuthError::Internal("login throttle lock poisoned".to_owned()))?;
            if !throttle.allow(&throttle_key, Instant::now()) {
                self.audit(
                    &normalized,
                    "auth.login",
                    None,
                    AuditOutcome::Denied,
                    ip,
                    Some(serde_json::json!({"reason": "rate_limited"})),
                )?;
                return Err(AuthError::TooManyAttempts);
            }
        }

        let user = self.db.user_by_username(&normalized)?;
        let Some(user) = user else {
            self.record_failed_login(&throttle_key);
            self.audit(
                &normalized,
                "auth.login",
                None,
                AuditOutcome::Denied,
                ip,
                Some(serde_json::json!({"reason": "unknown_user"})),
            )?;
            return Err(AuthError::InvalidCredentials);
        };

        if user.disabled {
            self.record_failed_login(&throttle_key);
            self.audit(
                &normalized,
                "auth.login",
                Some(&user.id.to_string()),
                AuditOutcome::Denied,
                ip,
                Some(serde_json::json!({"reason": "disabled"})),
            )?;
            return Err(AuthError::InvalidCredentials);
        }

        if !verify_password(password, &user.password_hash)? {
            self.record_failed_login(&throttle_key);
            self.audit(
                &normalized,
                "auth.login",
                Some(&user.id.to_string()),
                AuditOutcome::Denied,
                ip,
                Some(serde_json::json!({"reason": "bad_password"})),
            )?;
            return Err(AuthError::InvalidCredentials);
        }

        if let Ok(mut throttle) = self.throttle.lock() {
            throttle.reset(&throttle_key);
        }

        let now = Utc::now();
        let session_id = Uuid::new_v4();
        let expires_at = now
            + chrono::Duration::from_std(self.config.session_ttl)
                .unwrap_or_else(|_| chrono::Duration::hours(12));
        let session = SessionRecord {
            id: session_id,
            user_id: user.id,
            created_at: now,
            expires_at,
            revoked: false,
            ip: ip.map(ToOwned::to_owned),
            user_agent: user_agent.map(ToOwned::to_owned),
        };
        self.db.save_session(&session)?;

        let mut updated = user.clone();
        updated.last_login_at = Some(now);
        updated.updated_at = now;
        self.db.save_user(&updated)?;

        let claims = self.jwt.claims_for(
            user.id,
            session_id,
            user.role.id(),
            chrono::Duration::from_std(self.config.session_ttl)
                .unwrap_or_else(|_| chrono::Duration::hours(12)),
        );
        let token = self.jwt.encode(&claims)?;
        let csrf_token = self.jwt.csrf_token(session_id);
        self.audit(
            &user.username,
            "auth.login",
            Some(&user.id.to_string()),
            AuditOutcome::Success,
            ip,
            None,
        )?;

        Ok(LoginResult {
            user: updated,
            token,
            csrf_token,
            expires_at,
        })
    }

    /// Validate a session token and load the live user.
    ///
    /// # Errors
    /// Returns [`AuthError::Unauthorized`] for invalid, expired or revoked tokens.
    pub fn authenticate(&self, token: &str) -> AuthResult<AuthContext> {
        let claims = self.jwt.decode(token)?;
        let session_id = Uuid::parse_str(&claims.sid)
            .map_err(|_| AuthError::Unauthorized("invalid session id".to_owned()))?;
        let session = self
            .db
            .session(session_id)?
            .ok_or_else(|| AuthError::Unauthorized("session not found".to_owned()))?;
        if !session.is_active(Utc::now()) {
            return Err(AuthError::Unauthorized("session expired".to_owned()));
        }
        let user = self
            .db
            .user_by_id(session.user_id)?
            .ok_or_else(|| AuthError::Unauthorized("user not found".to_owned()))?;
        if user.disabled {
            return Err(AuthError::Unauthorized("account disabled".to_owned()));
        }
        if claims.sub != user.id.to_string() {
            return Err(AuthError::Unauthorized("token subject mismatch".to_owned()));
        }
        Ok(AuthContext {
            csrf_token: self.jwt.csrf_token(session_id),
            user,
            session_id,
        })
    }

    /// Verify the CSRF token sent for a cookie-authenticated request.
    #[must_use]
    pub fn verify_csrf(&self, context: &AuthContext, provided: &str) -> bool {
        self.jwt.verify_csrf(context.session_id, provided)
    }

    /// Revoke the current session.
    ///
    /// # Errors
    /// Returns a storage error when the session cannot be updated.
    pub fn logout(&self, context: &AuthContext) -> AuthResult<()> {
        if let Some(mut session) = self.db.session(context.session_id)? {
            session.revoked = true;
            self.db.save_session(&session)?;
        }
        self.audit(
            &context.user.username,
            "auth.logout",
            Some(&context.user.id.to_string()),
            AuditOutcome::Success,
            None,
            None,
        )?;
        Ok(())
    }

    /// Revoke every session of a user (used on password change/disable).
    ///
    /// # Errors
    /// Returns a storage error when sessions cannot be updated.
    pub fn revoke_user_sessions(&self, user_id: Uuid) -> AuthResult<usize> {
        self.revoke_user_sessions_except(user_id, Uuid::nil())
    }

    /// Number of administrators.
    ///
    /// # Errors
    /// Returns a storage error when users cannot be read.
    pub fn admin_count(&self) -> AuthResult<usize> {
        Ok(self
            .db
            .list_users()?
            .into_iter()
            .filter(|user| user.role == Role::Admin && !user.disabled)
            .count())
    }

    /// Append an audit entry.
    ///
    /// # Errors
    /// Returns a storage error when the entry cannot be persisted.
    pub fn audit(
        &self,
        actor: &str,
        action: &str,
        target: Option<&str>,
        outcome: AuditOutcome,
        ip: Option<&str>,
        details: Option<Value>,
    ) -> AuthResult<AuditEntry> {
        let entry = AuditEntry {
            seq: 0,
            timestamp: Utc::now(),
            actor: actor.to_owned(),
            action: action.to_owned(),
            target: target.map(ToOwned::to_owned),
            outcome,
            ip: ip.map(ToOwned::to_owned),
            details,
        };
        Ok(self.db.append_audit(entry)?)
    }

    /// Read the newest audit entries.
    ///
    /// # Errors
    /// Returns a storage error when entries cannot be read.
    pub fn list_audit(&self, limit: usize) -> AuthResult<Vec<AuditEntry>> {
        Ok(self.db.list_audit(limit)?)
    }

    /// Remove expired sessions.
    ///
    /// # Errors
    /// Returns a storage error when sessions cannot be purged.
    pub fn purge_expired_sessions(&self) -> AuthResult<usize> {
        Ok(self.db.purge_sessions(Utc::now())?)
    }

    fn build_user(
        &self,
        username: &str,
        password: &str,
        display_name: &str,
        role: Role,
    ) -> AuthResult<User> {
        let username = username.trim().to_lowercase();
        validation::username(&username)?;
        validation::password(password)?;
        if self.db.user_by_username(&username)?.is_some() {
            return Err(AuthError::Conflict(format!(
                "user `{username}` already exists"
            )));
        }
        let now = Utc::now();
        Ok(User {
            id: Uuid::new_v4(),
            username: username.clone(),
            display_name: if display_name.trim().is_empty() {
                username
            } else {
                display_name.trim().to_owned()
            },
            password_hash: hash_password(password)?,
            role,
            disabled: false,
            created_at: now,
            updated_at: now,
            last_login_at: None,
        })
    }

    fn record_failed_login(&self, throttle_key: &str) {
        if let Ok(mut throttle) = self.throttle.lock() {
            throttle.record_failure(throttle_key, Instant::now());
        }
    }

    fn revoke_user_sessions_except(&self, user_id: Uuid, keep: Uuid) -> AuthResult<usize> {
        let mut revoked = 0;
        for mut session in self
            .db
            .list::<SessionRecord>(hs_core::state::Table::Sessions)?
        {
            if session.user_id == user_id && session.id != keep && !session.revoked {
                session.revoked = true;
                self.db.save_session(&session)?;
                revoked += 1;
            }
        }
        Ok(revoked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jwt::JwtService;

    fn service() -> Arc<AuthService> {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let jwt = JwtService::new(JwtService::generate_secret(), "homecore-daemon");
        Arc::new(AuthService::new(db, jwt, AuthConfig::default()))
    }

    fn service_with_admin() -> Arc<AuthService> {
        let service = service();
        service
            .bootstrap_admin("admin", "Sup3rSecret!!", "Administrator")
            .unwrap();
        service
    }

    fn login_admin(service: &AuthService) -> AuthContext {
        let login = service
            .login("admin", "Sup3rSecret!!", Some("127.0.0.1"), None)
            .unwrap();
        service.authenticate(&login.token).unwrap()
    }

    #[test]
    fn bootstrap_creates_admin_and_is_unique() {
        let service = service();
        assert!(!service.has_users().unwrap());
        let user = service
            .bootstrap_admin("admin", "Sup3rSecret!!", "Admin")
            .unwrap();
        assert_eq!(user.role, Role::Admin);
        assert!(service.has_users().unwrap());
        assert!(matches!(
            service.bootstrap_admin("other", "Sup3rSecret!!", "Other"),
            Err(AuthError::Conflict(_))
        ));
    }

    #[test]
    fn bootstrap_validates_weak_password() {
        let service = service();
        assert!(matches!(
            service.bootstrap_admin("admin", "short", "Admin"),
            Err(AuthError::Validation(_))
        ));
    }

    #[test]
    fn login_and_authenticate_roundtrip() {
        let service = service_with_admin();
        let login = service
            .login("Admin", "Sup3rSecret!!", Some("192.168.1.2"), Some("test"))
            .unwrap();
        assert_eq!(login.user.username, "admin");
        assert!(!login.token.is_empty());
        let context = service.authenticate(&login.token).unwrap();
        assert_eq!(context.user.id, login.user.id);
        assert_eq!(context.csrf_token, login.csrf_token);
        assert!(service.verify_csrf(&context, &login.csrf_token));
        assert!(!service.verify_csrf(&context, "forged"));
    }

    #[test]
    fn wrong_password_is_rejected_and_audited() {
        let service = service_with_admin();
        assert!(matches!(
            service.login("admin", "WrongPassword1!", None, None),
            Err(AuthError::InvalidCredentials)
        ));
        let audit = service.list_audit(10).unwrap();
        assert!(audit
            .iter()
            .any(|entry| entry.action == "auth.login" && entry.outcome == AuditOutcome::Denied));
    }

    #[test]
    fn unknown_user_is_rejected() {
        let service = service_with_admin();
        assert!(matches!(
            service.login("nobody", "Sup3rSecret!!", None, None),
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[test]
    fn logout_revokes_session() {
        let service = service_with_admin();
        let context = login_admin(&service);
        service.logout(&context).unwrap();
        assert!(service.authenticate(&context.csrf_token).is_err());
        // The real token is no longer valid either.
        let login = service.login("admin", "Sup3rSecret!!", None, None).unwrap();
        service
            .logout(&service.authenticate(&login.token).unwrap())
            .unwrap();
        assert!(service.authenticate(&login.token).is_err());
    }

    #[test]
    fn disabled_user_cannot_login() {
        let service = service_with_admin();
        let admin = login_admin(&service);
        let operator = service
            .create_user(&admin, "operator", "Sup3rSecret!!", "Op", Role::Operator)
            .unwrap();
        service.set_disabled(&admin, operator.id, true).unwrap();
        assert!(matches!(
            service.login("operator", "Sup3rSecret!!", None, None),
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[test]
    fn rbac_blocks_operator_from_managing_users() {
        let service = service_with_admin();
        let admin = login_admin(&service);
        service
            .create_user(&admin, "operator", "Sup3rSecret!!", "Op", Role::Operator)
            .unwrap();
        let login = service
            .login("operator", "Sup3rSecret!!", None, None)
            .unwrap();
        let operator = service.authenticate(&login.token).unwrap();
        assert!(matches!(
            service.create_user(&operator, "x", "Sup3rSecret!!", "X", Role::Viewer),
            Err(AuthError::Forbidden(_))
        ));
    }

    #[test]
    fn last_admin_cannot_be_demoted_or_deleted() {
        let service = service_with_admin();
        let admin = login_admin(&service);
        assert!(matches!(
            service.update_role(&admin, admin.user.id, Role::Viewer),
            Err(AuthError::Conflict(_))
        ));
        assert!(matches!(
            service.delete_user(&admin, admin.user.id),
            Err(AuthError::Conflict(_))
        ));
    }

    #[test]
    fn login_is_throttled() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let jwt = JwtService::new(JwtService::generate_secret(), "homecore-daemon");
        let config = AuthConfig {
            max_login_attempts: 2,
            ..AuthConfig::default()
        };
        let service = AuthService::new(db, jwt, config);
        service
            .bootstrap_admin("admin", "Sup3rSecret!!", "Admin")
            .unwrap();
        assert!(service.login("admin", "bad", None, None).is_err());
        assert!(service.login("admin", "bad", None, None).is_err());
        assert!(matches!(
            service.login("admin", "Sup3rSecret!!", None, None),
            Err(AuthError::TooManyAttempts)
        ));
    }

    #[test]
    fn password_change_revokes_other_sessions() {
        let service = service_with_admin();
        let first = service.login("admin", "Sup3rSecret!!", None, None).unwrap();
        let second = service.login("admin", "Sup3rSecret!!", None, None).unwrap();
        let context = service.authenticate(&first.token).unwrap();
        service
            .change_own_password(&context, "Sup3rSecret!!", "N3wSecret!!!")
            .unwrap();
        assert!(service.authenticate(&second.token).is_err());
        assert!(service.authenticate(&first.token).is_ok());
        assert!(matches!(
            service.login("admin", "Sup3rSecret!!", None, None),
            Err(AuthError::InvalidCredentials)
        ));
        assert!(service.login("admin", "N3wSecret!!!", None, None).is_ok());
    }

    #[test]
    fn audit_is_append_only_with_increasing_sequence() {
        let service = service_with_admin();
        let entries = service.list_audit(50).unwrap();
        assert!(!entries.is_empty());
        let mut last = 0;
        for entry in entries.into_iter().rev() {
            assert!(entry.seq > last);
            last = entry.seq;
        }
    }

    #[test]
    fn tokens_survive_service_restart_but_not_secret_rotation() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let secret = JwtService::generate_secret();
        let jwt = JwtService::new(secret, "homecore-daemon");
        let service = AuthService::new(db.clone(), jwt, AuthConfig::default());
        service
            .bootstrap_admin("admin", "Sup3rSecret!!", "Admin")
            .unwrap();
        let login = service.login("admin", "Sup3rSecret!!", None, None).unwrap();

        let service2 = AuthService::new(
            db,
            JwtService::new(secret, "homecore-daemon"),
            AuthConfig::default(),
        );
        assert!(service2.authenticate(&login.token).is_ok());

        let service3 = AuthService::new(
            Arc::new(Database::open_in_memory().unwrap()),
            JwtService::new(JwtService::generate_secret(), "homecore-daemon"),
            AuthConfig::default(),
        );
        assert!(service3.authenticate(&login.token).is_err());
    }
}
