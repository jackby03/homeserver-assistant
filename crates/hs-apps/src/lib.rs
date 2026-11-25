//! Application catalog, deployment orchestration and reverse-proxy routes.
//!
//! `hs-apps` sits between the HTTP API (`hs-daemon`) and the runtime provider
//! port. It owns the curated template catalog, validates deploy requests,
//! translates them into [`OciDeploySpec`]s, persists [`AppRecord`]s in the
//! embedded database and derives the Caddy proxy routes for LAN applications.
//!
//! Secrets are intentionally **not** handled here: `hs-daemon` resolves them
//! from the encrypted vault and passes the values in [`DeployAppRequest::env`].
//! Template variables marked as secret are forwarded to the runtime but never
//! written to the [`AppRecord`].
//!
//! # Example
//! ```
//! use std::sync::Arc;
//!
//! use hs_apps::{AppService, DeployAppRequest};
//! use hs_core::Database;
//! use hs_core::models::NetworkSegment;
//! use hs_runtime::mock::MockRuntime;
//!
//! # tokio::runtime::Runtime::new().unwrap().block_on(async {
//! let runtime = Arc::new(MockRuntime::new());
//! let db = Arc::new(Database::open_in_memory().expect("database"));
//! let service = AppService::new(runtime, db, "home.local".to_owned());
//!
//! let record = service
//!     .deploy(DeployAppRequest {
//!         name: "Home Assistant".to_owned(),
//!         template_id: Some("home-assistant".to_owned()),
//!         image: None,
//!         segment: NetworkSegment::Lan,
//!         internal_port: 8123,
//!         subdomain: Some("home-assistant".to_owned()),
//!         expose_externally: false,
//!         env: Default::default(),
//!     })
//!     .await
//!     .expect("deploy home assistant");
//!
//! assert_eq!(record.hostname.as_deref(), Some("home-assistant.home.local"));
//! assert_eq!(service.list().expect("list apps").len(), 1);
//! # });
//! ```

#![deny(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use hs_core::models::{
    AppCategory, AppRecord, AppStatus, AppTemplate, AppTemplateEnv, DeployAppRequest,
    NetworkSegment, ResourceLimits, RuntimeStatus,
};
use hs_core::{CoreError, CoreResult, Database};
use hs_runtime::{OciDeploySpec, RuntimeError, RuntimeProvider};

/// Health-check path suggestion comment: templates only describe metadata, the
/// reverse proxy and health probing are owned by `hs-daemon`/infra.
const INTERNAL_PORT_ENV: &str = "PORT";

/// A reverse proxy route derived from a deployed LAN application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyRoute {
    /// Public hostname (`home-assistant.home.local`).
    pub host: String,
    /// Upstream URL the proxy forwards to.
    pub upstream: String,
}

/// Curated application catalog shipped with the appliance.
///
/// The catalog is static data: it describes real OCI images, default ports,
/// categories, suggested network segments and the environment variables each
/// application understands. Images are pinned to stable/release tags that the
/// appliance team validates at release time.
#[must_use]
pub fn builtin_catalog() -> Vec<AppTemplate> {
    vec![
        AppTemplate {
            id: "home-assistant".to_owned(),
            name: "Home Assistant".to_owned(),
            description: "Local home automation hub with dashboards and integrations.".to_owned(),
            image: "ghcr.io/home-assistant/home-assistant:stable".to_owned(),
            default_port: 8123,
            category: AppCategory::Automation,
            segment: NetworkSegment::Lan,
            env: vec![optional_env("TZ", "Timezone", "UTC")],
            featured: true,
        },
        AppTemplate {
            id: "adguard-home".to_owned(),
            name: "AdGuard Home".to_owned(),
            description: "Network-wide DNS filtering and tracker blocking.".to_owned(),
            image: "adguard/adguardhome:latest".to_owned(),
            default_port: 3000,
            category: AppCategory::Networking,
            segment: NetworkSegment::Lan,
            env: Vec::new(),
            featured: true,
        },
        AppTemplate {
            id: "music-assistant".to_owned(),
            name: "Music Assistant".to_owned(),
            description: "Multi-room audio library and streaming coordinator.".to_owned(),
            image: "ghcr.io/music-assistant/server:latest".to_owned(),
            default_port: 8095,
            category: AppCategory::Media,
            segment: NetworkSegment::Lan,
            env: vec![optional_env("TZ", "Timezone", "UTC")],
            featured: true,
        },
        AppTemplate {
            id: "immich".to_owned(),
            name: "Immich".to_owned(),
            description: "Self-hosted photo and video backup with face recognition.".to_owned(),
            image: "ghcr.io/immich-app/immich-server:release".to_owned(),
            default_port: 2283,
            category: AppCategory::Media,
            segment: NetworkSegment::Dmz,
            env: vec![
                required_secret_env("DB_PASSWORD", "Database password"),
                optional_env("DB_USERNAME", "Database user", "immich"),
                optional_env("DB_DATABASE_NAME", "Database name", "immich"),
            ],
            featured: true,
        },
        AppTemplate {
            id: "nextcloud".to_owned(),
            name: "Nextcloud".to_owned(),
            description: "Private file sync and collaboration suite.".to_owned(),
            image: "nextcloud:stable".to_owned(),
            default_port: 80,
            category: AppCategory::Productivity,
            segment: NetworkSegment::Dmz,
            env: vec![optional_env("TZ", "Timezone", "UTC")],
            featured: false,
        },
        AppTemplate {
            id: "vaultwarden".to_owned(),
            name: "Vaultwarden".to_owned(),
            description: "Lightweight Bitwarden-compatible password vault.".to_owned(),
            image: "vaultwarden/server:latest".to_owned(),
            default_port: 80,
            category: AppCategory::Productivity,
            segment: NetworkSegment::Dmz,
            env: vec![optional_secret_env("ADMIN_TOKEN", "Admin panel token")],
            featured: false,
        },
        AppTemplate {
            id: "uptime-kuma".to_owned(),
            name: "Uptime Kuma".to_owned(),
            description: "Self-hosted uptime and service monitoring dashboard.".to_owned(),
            image: "louislam/uptime-kuma:1".to_owned(),
            default_port: 3001,
            category: AppCategory::Networking,
            segment: NetworkSegment::Lan,
            env: Vec::new(),
            featured: false,
        },
        AppTemplate {
            id: "n8n".to_owned(),
            name: "n8n".to_owned(),
            description: "Visual workflow automation with hundreds of integrations.".to_owned(),
            image: "n8nio/n8n:latest".to_owned(),
            default_port: 5678,
            category: AppCategory::Automation,
            segment: NetworkSegment::Lan,
            env: vec![
                required_secret_env("N8N_ENCRYPTION_KEY", "Workflow encryption key"),
                optional_env("GENERIC_TIMEZONE", "Workflow timezone", "UTC"),
            ],
            featured: false,
        },
    ]
}

/// Declare an optional non-secret environment variable.
fn optional_env(key: &str, label: &str, default: &str) -> AppTemplateEnv {
    AppTemplateEnv {
        key: key.to_owned(),
        label: label.to_owned(),
        required: false,
        secret: false,
        default: Some(default.to_owned()),
    }
}

/// Declare a required secret environment variable (never persisted).
fn required_secret_env(key: &str, label: &str) -> AppTemplateEnv {
    AppTemplateEnv {
        key: key.to_owned(),
        label: label.to_owned(),
        required: true,
        secret: true,
        default: None,
    }
}

/// Declare an optional secret environment variable (never persisted).
fn optional_secret_env(key: &str, label: &str) -> AppTemplateEnv {
    AppTemplateEnv {
        key: key.to_owned(),
        label: label.to_owned(),
        required: false,
        secret: true,
        default: None,
    }
}

/// Application deployment service.
///
/// The service is stateless apart from its dependencies and cheap to share
/// inside an `Arc`: every deployment record lives in [`Database`].
pub struct AppService {
    runtime: Arc<dyn RuntimeProvider>,
    db: Arc<Database>,
    base_domain: String,
}

impl std::fmt::Debug for AppService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppService")
            .field("provider", &self.runtime.provider_name())
            .field("base_domain", &self.base_domain)
            .finish_non_exhaustive()
    }
}

impl AppService {
    /// Create a service.
    ///
    /// `base_domain` is the LAN domain used to build application hostnames
    /// (`home.local` produces `nextcloud.home.local`).
    #[must_use]
    pub fn new(runtime: Arc<dyn RuntimeProvider>, db: Arc<Database>, base_domain: String) -> Self {
        Self {
            runtime,
            db,
            base_domain,
        }
    }

    /// Runtime provider used by the service.
    #[must_use]
    pub fn runtime(&self) -> &Arc<dyn RuntimeProvider> {
        &self.runtime
    }

    /// Base LAN domain used to build application hostnames.
    #[must_use]
    pub fn base_domain(&self) -> &str {
        &self.base_domain
    }

    /// Curated application catalog.
    #[must_use]
    pub fn catalog(&self) -> Vec<AppTemplate> {
        builtin_catalog()
    }

    /// List deployed applications, sorted by display name.
    ///
    /// # Errors
    /// Returns a storage error when the database cannot be read.
    pub fn list(&self) -> CoreResult<Vec<AppRecord>> {
        let mut apps = self.db.list_apps()?;
        apps.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(apps)
    }

    /// Load one deployed application.
    ///
    /// # Errors
    /// Returns [`CoreError::NotFound`] when the id is unknown.
    pub fn get(&self, id: Uuid) -> CoreResult<AppRecord> {
        self.db
            .app(id)?
            .ok_or_else(|| CoreError::not_found(format!("application `{id}`")))
    }

    /// Deploy an application.
    ///
    /// The record is persisted as `deploying` before the runtime call so a
    /// failed deployment always leaves a `failed` record behind (useful for the
    /// UI and the audit trail).
    ///
    /// # Errors
    /// - [`CoreError::Validation`] for invalid names, images, ports,
    ///   hostnames or missing required environment variables.
    /// - [`CoreError::NotFound`] for unknown template ids.
    /// - Any error reported by the runtime provider (mapped through
    ///   [`hs_runtime::RuntimeError`]).
    pub async fn deploy(&self, req: DeployAppRequest) -> CoreResult<AppRecord> {
        let template = match &req.template_id {
            Some(template_id) => Some(self.template(template_id)?),
            None => None,
        };
        validate_name(&req.name)?;
        hs_core::validation::port(req.internal_port)?;
        let image = resolve_image(&req, template.as_ref())?;
        let (runtime_env, record_env) = prepare_env(&req, template.as_ref())?;
        let hostname = self.resolve_hostname(&req)?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let mut record = AppRecord {
            id,
            name: req.name.trim().to_owned(),
            instance_name: instance_name_for(&req.name, id),
            template_id: template.as_ref().map(|template| template.id.clone()),
            image,
            segment: req.segment,
            internal_port: req.internal_port,
            hostname,
            expose_externally: req.expose_externally,
            env: record_env,
            status: AppStatus::Deploying,
            created_at: now,
            updated_at: now,
        };
        self.db.save_app(&record)?;

        let mut deploy_env = runtime_env;
        deploy_env
            .entry(INTERNAL_PORT_ENV.to_owned())
            .or_insert_with(|| req.internal_port.to_string());
        let spec = OciDeploySpec {
            name: record.instance_name.clone(),
            image: record.image.clone(),
            segment: record.segment,
            env: deploy_env,
            limits: ResourceLimits::default(),
        };

        let summary = match self.runtime.deploy_oci(spec).await {
            Ok(summary) => summary,
            Err(error) => {
                record.status = AppStatus::Failed;
                record.updated_at = Utc::now();
                if let Err(storage_error) = self.db.save_app(&record) {
                    tracing::error!(
                        app = %record.id,
                        error = %storage_error,
                        "could not persist failed deployment record"
                    );
                }
                tracing::warn!(
                    app = %record.name,
                    error = %error,
                    "application deployment failed"
                );
                return Err(error.into());
            }
        };

        record.status = app_status_from_runtime(summary.status);
        record.updated_at = Utc::now();
        self.db.save_app(&record)?;
        tracing::info!(
            app = %record.name,
            instance = %record.instance_name,
            status = ?record.status,
            "application deployed"
        );
        Ok(record)
    }

    /// Remove an application: stop and delete its runtime instance, then drop
    /// the record.
    ///
    /// # Errors
    /// Returns [`CoreError::NotFound`] for an unknown id and a runtime error
    /// when the instance cannot be deleted (already missing instances are
    /// tolerated).
    pub async fn remove(&self, id: Uuid) -> CoreResult<()> {
        let record = self.get(id)?;
        if let Err(error) = self.runtime.stop(&record.instance_name).await {
            if !matches!(error, RuntimeError::NotFound(_)) {
                tracing::warn!(
                    app = %record.name,
                    error = %error,
                    "could not stop application instance before removal"
                );
            }
        }
        match self.runtime.delete(&record.instance_name).await {
            Ok(()) => {}
            Err(RuntimeError::NotFound(_)) => {}
            Err(error) => return Err(error.into()),
        }
        self.db.delete_app(id)?;
        tracing::info!(app = %record.name, "application removed");
        Ok(())
    }

    /// Start a deployed application.
    ///
    /// # Errors
    /// Returns [`CoreError::NotFound`] for an unknown id or a runtime error when
    /// the instance cannot be started.
    pub async fn start(&self, id: Uuid) -> CoreResult<AppRecord> {
        let mut record = self.get(id)?;
        self.runtime.start(&record.instance_name).await?;
        record.status = AppStatus::Running;
        record.updated_at = Utc::now();
        self.db.save_app(&record)?;
        Ok(record)
    }

    /// Stop a deployed application.
    ///
    /// # Errors
    /// Returns [`CoreError::NotFound`] for an unknown id or a runtime error when
    /// the instance cannot be stopped.
    pub async fn stop(&self, id: Uuid) -> CoreResult<AppRecord> {
        let mut record = self.get(id)?;
        self.runtime.stop(&record.instance_name).await?;
        record.status = AppStatus::Stopped;
        record.updated_at = Utc::now();
        self.db.save_app(&record)?;
        Ok(record)
    }

    /// Find a catalog template by id.
    fn template(&self, template_id: &str) -> CoreResult<AppTemplate> {
        builtin_catalog()
            .into_iter()
            .find(|template| template.id == template_id)
            .ok_or_else(|| CoreError::not_found(format!("application template `{template_id}`")))
    }

    /// Resolve the reverse-proxy hostname for a deployment.
    ///
    /// DMZ applications never receive a local hostname: external publishing is
    /// handled by the outbound tunnel (infra), and `expose_externally` remains
    /// metadata on the record.
    fn resolve_hostname(&self, req: &DeployAppRequest) -> CoreResult<Option<String>> {
        if req.segment == NetworkSegment::Dmz {
            return Ok(None);
        }
        hs_core::validation::domain(&self.base_domain)?;
        let label = match &req.subdomain {
            Some(subdomain) => {
                let subdomain = subdomain.trim();
                hs_core::validation::subdomain(subdomain)?;
                subdomain.to_owned()
            }
            None => slugify(&req.name),
        };
        if label.is_empty() {
            return Err(CoreError::validation("subdomain", "must not be empty"));
        }
        hs_core::validation::hostname(&label)?;
        Ok(Some(format!("{label}.{}", self.base_domain)))
    }
}

/// Derive the Caddy proxy route for an application.
///
/// Only LAN applications with a hostname are published through the local
/// reverse proxy. DMZ applications are exposed through the tunnel instead, so
/// they never produce a local route (defense in depth: a DMZ record with a
/// hostname still produces no route).
#[must_use]
pub fn proxy_route(app: &AppRecord) -> Option<ProxyRoute> {
    if app.segment != NetworkSegment::Lan {
        return None;
    }
    let host = app.hostname.clone()?;
    Some(ProxyRoute {
        host,
        upstream: format!("http://{}:{}", app.instance_name, app.internal_port),
    })
}

/// Build a valid Incus/LXC instance name from an application name and id.
///
/// The result starts with a lowercase ASCII letter, only contains lowercase
/// letters, digits and hyphens, is at most 63 characters long and is unique per
/// deployment thanks to the id suffix (`home-assistant-1a2b3c4d`).
#[must_use]
pub fn instance_name_for(name: &str, id: Uuid) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_hyphen = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            last_hyphen = false;
        } else if !last_hyphen && !slug.is_empty() {
            slug.push('-');
            last_hyphen = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug.truncate(40);
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("app");
    }
    if !slug
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_lowercase())
    {
        slug.insert_str(0, "app-");
    }
    let suffix = id.simple().to_string();
    let short = &suffix[..8.min(suffix.len())];
    format!("{slug}-{short}")
}

/// Convert a display name into a DNS label.
fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_hyphen = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            last_hyphen = false;
        } else if !last_hyphen && !slug.is_empty() {
            slug.push('-');
            last_hyphen = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug.truncate(63);
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

/// Validate an application display name.
fn validate_name(name: &str) -> CoreResult<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 48 {
        return Err(CoreError::validation(
            "name",
            "must be between 1 and 48 characters",
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(CoreError::validation(
            "name",
            "must not contain control characters",
        ));
    }
    Ok(())
}

/// Resolve the OCI image for a deployment (request override or template).
fn resolve_image(req: &DeployAppRequest, template: Option<&AppTemplate>) -> CoreResult<String> {
    let image = match (&req.image, template) {
        (Some(image), _) => image.trim().to_owned(),
        (None, Some(template)) => template.image.clone(),
        (None, None) => {
            return Err(CoreError::validation(
                "image",
                "is required when deploying without a template",
            ));
        }
    };
    if image.is_empty() {
        return Err(CoreError::validation("image", "must not be empty"));
    }
    if image.chars().any(char::is_whitespace) {
        return Err(CoreError::validation(
            "image",
            "must not contain whitespace",
        ));
    }
    Ok(image)
}

/// Merge request env with template declarations.
///
/// Returns `(runtime_env, record_env)`: the runtime receives every value
/// (including secrets) while the record only keeps non-secret declarations so
/// the database never stores credentials.
fn prepare_env(
    req: &DeployAppRequest,
    template: Option<&AppTemplate>,
) -> CoreResult<(BTreeMap<String, String>, BTreeMap<String, String>)> {
    let Some(template) = template else {
        return Ok((req.env.clone(), req.env.clone()));
    };
    let mut runtime_env = BTreeMap::new();
    let mut record_env = BTreeMap::new();
    for declaration in &template.env {
        let value = req
            .env
            .get(&declaration.key)
            .cloned()
            .or_else(|| declaration.default.clone());
        let Some(value) = value else {
            if declaration.required {
                return Err(CoreError::validation(
                    "env",
                    format!(
                        "required environment variable `{}` is missing",
                        declaration.key
                    ),
                ));
            }
            continue;
        };
        runtime_env.insert(declaration.key.clone(), value.clone());
        if !declaration.secret {
            record_env.insert(declaration.key.clone(), value);
        }
    }
    for (key, value) in &req.env {
        if template
            .env
            .iter()
            .any(|declaration| declaration.key == *key)
        {
            continue;
        }
        runtime_env
            .entry(key.clone())
            .or_insert_with(|| value.clone());
        record_env
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    Ok((runtime_env, record_env))
}

/// Map a runtime status to the persisted application status.
fn app_status_from_runtime(status: RuntimeStatus) -> AppStatus {
    match status {
        RuntimeStatus::Running => AppStatus::Running,
        RuntimeStatus::Stopped => AppStatus::Stopped,
        RuntimeStatus::Error => AppStatus::Failed,
        RuntimeStatus::Starting | RuntimeStatus::Stopping | RuntimeStatus::Frozen => {
            AppStatus::Deploying
        }
        RuntimeStatus::Unknown => AppStatus::Pending,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use hs_core::models::DeployAppRequest;
    use hs_runtime::mock::MockRuntime;
    use hs_runtime::RuntimeError;

    fn service_with_mock(runtime: Arc<MockRuntime>) -> AppService {
        let db = Arc::new(Database::open_in_memory().expect("in-memory database"));
        AppService::new(runtime, db, "home.local".to_owned())
    }

    fn deploy_request(template: &str, name: &str, segment: NetworkSegment) -> DeployAppRequest {
        DeployAppRequest {
            name: name.to_owned(),
            template_id: Some(template.to_owned()),
            image: None,
            segment,
            internal_port: 8123,
            subdomain: None,
            expose_externally: false,
            env: BTreeMap::new(),
        }
    }

    fn record(segment: NetworkSegment, hostname: Option<&str>) -> AppRecord {
        AppRecord {
            id: Uuid::new_v4(),
            name: "Demo".to_owned(),
            instance_name: "demo-1234abcd".to_owned(),
            template_id: None,
            image: "nextcloud:stable".to_owned(),
            segment,
            internal_port: 8080,
            hostname: hostname.map(str::to_owned),
            expose_externally: false,
            env: BTreeMap::new(),
            status: AppStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn catalog_is_unique_and_uses_valid_images() {
        let catalog = builtin_catalog();
        assert!(catalog.len() >= 8, "catalog must ship at least 8 templates");
        let mut ids = std::collections::BTreeSet::new();
        for template in &catalog {
            assert!(
                ids.insert(template.id.clone()),
                "duplicate id {}",
                template.id
            );
            assert!(!template.name.is_empty());
            assert!(!template.description.is_empty());
            assert!(template.default_port > 0);
            assert!(!template.image.contains(char::is_whitespace));
            assert!(
                template.image.contains(':'),
                "image `{}` must be tagged",
                template.image
            );
            for declaration in &template.env {
                assert!(!declaration.key.is_empty());
                assert!(!declaration.label.is_empty());
            }
        }
    }

    #[test]
    fn catalog_contains_expected_applications() {
        let catalog = builtin_catalog();
        let home_assistant = catalog
            .iter()
            .find(|template| template.id == "home-assistant")
            .expect("home assistant template");
        assert_eq!(
            home_assistant.image,
            "ghcr.io/home-assistant/home-assistant:stable"
        );
        assert_eq!(home_assistant.default_port, 8123);
        assert_eq!(home_assistant.category, AppCategory::Automation);
        assert_eq!(home_assistant.segment, NetworkSegment::Lan);
        assert!(home_assistant.featured);

        let immich = catalog
            .iter()
            .find(|template| template.id == "immich")
            .expect("immich template");
        assert!(immich
            .env
            .iter()
            .any(|declaration| declaration.key == "DB_PASSWORD"
                && declaration.required
                && declaration.secret));
    }

    #[tokio::test]
    async fn deploy_from_template_persists_and_deploys() {
        let runtime = Arc::new(MockRuntime::new());
        let service = service_with_mock(runtime.clone());
        let mut req = deploy_request("home-assistant", "Home Assistant", NetworkSegment::Lan);
        req.subdomain = Some("home-assistant".to_owned());

        let record = service.deploy(req).await.expect("deploy app");
        assert_eq!(record.status, AppStatus::Running);
        assert_eq!(record.image, "ghcr.io/home-assistant/home-assistant:stable");
        assert_eq!(
            record.hostname.as_deref(),
            Some("home-assistant.home.local")
        );
        assert!(hs_core::validation::instance_name(&record.instance_name).is_ok());
        assert_eq!(service.list().unwrap().len(), 1);
        assert_eq!(runtime.deploy_calls().await.len(), 1);
        let call = &runtime.deploy_calls().await[0];
        assert_eq!(call.image, record.image);
        assert_eq!(call.segment, NetworkSegment::Lan);
        assert_eq!(
            call.env.get(INTERNAL_PORT_ENV).map(String::as_str),
            Some("8123")
        );
    }

    #[tokio::test]
    async fn deploy_custom_image_without_template() {
        let runtime = Arc::new(MockRuntime::new());
        let service = service_with_mock(runtime.clone());
        let req = DeployAppRequest {
            name: "My App".to_owned(),
            template_id: None,
            image: Some("ghcr.io/example/my-app:1.2.3".to_owned()),
            segment: NetworkSegment::Lan,
            internal_port: 8080,
            subdomain: Some("my-app".to_owned()),
            expose_externally: false,
            env: BTreeMap::from([("FOO".to_owned(), "bar".to_owned())]),
        };
        let record = service.deploy(req).await.expect("deploy custom app");
        assert_eq!(record.image, "ghcr.io/example/my-app:1.2.3");
        assert_eq!(record.template_id, None);
        assert_eq!(record.env.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(record.status, AppStatus::Running);
    }

    #[tokio::test]
    async fn missing_required_env_is_rejected() {
        let runtime = Arc::new(MockRuntime::new());
        let service = service_with_mock(runtime.clone());
        let req = deploy_request("immich", "Immich", NetworkSegment::Dmz);
        match service.deploy(req).await {
            Err(CoreError::Validation { field, message }) => {
                assert_eq!(field, "env");
                assert!(message.contains("DB_PASSWORD"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
        assert!(service.list().unwrap().is_empty());
        assert!(runtime.deploy_calls().await.is_empty());
    }

    #[tokio::test]
    async fn required_secret_env_is_forwarded_but_not_persisted() {
        let runtime = Arc::new(MockRuntime::new());
        let service = service_with_mock(runtime.clone());
        let mut req = deploy_request("immich", "Immich", NetworkSegment::Dmz);
        req.env
            .insert("DB_PASSWORD".to_owned(), "super-secret".to_owned());
        let record = service.deploy(req).await.expect("deploy immich");
        assert!(!record.env.contains_key("DB_PASSWORD"));
        assert_eq!(
            record.env.get("DB_USERNAME").map(String::as_str),
            Some("immich")
        );
        let call = &runtime.deploy_calls().await[0];
        assert_eq!(
            call.env.get("DB_PASSWORD").map(String::as_str),
            Some("super-secret")
        );
    }

    #[tokio::test]
    async fn failed_deploy_marks_record_failed() {
        let runtime = Arc::new(MockRuntime::new());
        runtime
            .set_failure(Some(RuntimeError::Backend("incus down".to_owned())))
            .await;
        let service = service_with_mock(runtime);
        let req = deploy_request("home-assistant", "Home Assistant", NetworkSegment::Lan);
        assert!(service.deploy(req).await.is_err());
        let apps = service.list().unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].status, AppStatus::Failed);
    }

    #[tokio::test]
    async fn remove_deletes_record_and_runtime_instance() {
        let runtime = Arc::new(MockRuntime::new());
        let service = service_with_mock(runtime.clone());
        let record = service
            .deploy(deploy_request(
                "home-assistant",
                "Home Assistant",
                NetworkSegment::Lan,
            ))
            .await
            .expect("deploy app");
        assert_eq!(runtime.instances().await.len(), 1);
        service.remove(record.id).await.expect("remove app");
        assert!(service.list().unwrap().is_empty());
        assert!(runtime.instances().await.is_empty());
    }

    #[tokio::test]
    async fn start_and_stop_update_status() {
        let runtime = Arc::new(MockRuntime::new());
        let service = service_with_mock(runtime.clone());
        let record = service
            .deploy(deploy_request(
                "uptime-kuma",
                "Uptime Kuma",
                NetworkSegment::Lan,
            ))
            .await
            .expect("deploy uptime kuma");

        let stopped = service.stop(record.id).await.expect("stop app");
        assert_eq!(stopped.status, AppStatus::Stopped);
        let started = service.start(record.id).await.expect("start app");
        assert_eq!(started.status, AppStatus::Running);
        assert_eq!(runtime.stopped().await, vec![record.instance_name.clone()]);
        assert_eq!(runtime.started().await, vec![record.instance_name]);
    }

    #[tokio::test]
    async fn dmz_deploy_has_no_hostname() {
        let runtime = Arc::new(MockRuntime::new());
        let service = service_with_mock(runtime);
        let mut req = deploy_request("nextcloud", "Nextcloud", NetworkSegment::Dmz);
        req.subdomain = Some("cloud".to_owned());
        req.expose_externally = true;
        let record = service.deploy(req).await.expect("deploy nextcloud");
        assert_eq!(record.hostname, None);
        assert!(record.expose_externally);
        assert_eq!(proxy_route(&record), None);
    }

    #[test]
    fn proxy_route_lan_vs_dmz() {
        let lan = record(NetworkSegment::Lan, Some("demo.home.local"));
        let route = proxy_route(&lan).expect("LAN route");
        assert_eq!(route.host, "demo.home.local");
        assert_eq!(route.upstream, "http://demo-1234abcd:8080");

        let dmz = record(NetworkSegment::Dmz, None);
        assert_eq!(proxy_route(&dmz), None);

        let lan_without_hostname = record(NetworkSegment::Lan, None);
        assert_eq!(proxy_route(&lan_without_hostname), None);

        // Defensive: even a DMZ record that somehow has a hostname is not routed
        // through the LAN proxy.
        let dmz_with_hostname = record(NetworkSegment::Dmz, Some("demo.home.local"));
        assert_eq!(proxy_route(&dmz_with_hostname), None);
    }

    #[test]
    fn instance_name_generation_is_valid_and_unique() {
        let id = Uuid::parse_str("1a2b3c4d-0000-0000-0000-000000000000").unwrap();
        let names = [
            ("Home Assistant", "home-assistant-1a2b3c4d"),
            ("My Great App!!", "my-great-app-1a2b3c4d"),
            ("123", "app-123-1a2b3c4d"),
            ("---", "app-1a2b3c4d"),
            ("", "app-1a2b3c4d"),
        ];
        for (input, expected) in names {
            let generated = instance_name_for(input, id);
            assert_eq!(generated, expected, "input: {input:?}");
            assert!(generated.len() < 63);
            hs_core::validation::instance_name(&generated).unwrap();
        }
        let long = "A".repeat(120);
        let generated = instance_name_for(&long, id);
        assert!(generated.len() < 63);
        hs_core::validation::instance_name(&generated).unwrap();
    }

    #[tokio::test]
    async fn invalid_requests_are_rejected() {
        let runtime = Arc::new(MockRuntime::new());
        let service = service_with_mock(runtime.clone());

        let mut req = deploy_request("home-assistant", "Home Assistant", NetworkSegment::Lan);
        req.internal_port = 0;
        assert!(matches!(
            service.deploy(req).await,
            Err(CoreError::Validation { field, .. }) if field == "port"
        ));

        let req = DeployAppRequest {
            name: "   ".to_owned(),
            template_id: Some("home-assistant".to_owned()),
            image: None,
            segment: NetworkSegment::Lan,
            internal_port: 8123,
            subdomain: None,
            expose_externally: false,
            env: BTreeMap::new(),
        };
        assert!(matches!(
            service.deploy(req).await,
            Err(CoreError::Validation { field, .. }) if field == "name"
        ));

        let req = deploy_request("does-not-exist", "Unknown", NetworkSegment::Lan);
        assert!(matches!(
            service.deploy(req).await,
            Err(CoreError::NotFound(_))
        ));

        let req = DeployAppRequest {
            name: "No Image".to_owned(),
            template_id: None,
            image: None,
            segment: NetworkSegment::Lan,
            internal_port: 8080,
            subdomain: None,
            expose_externally: false,
            env: BTreeMap::new(),
        };
        assert!(matches!(
            service.deploy(req).await,
            Err(CoreError::Validation { field, .. }) if field == "image"
        ));
    }

    #[tokio::test]
    async fn start_stop_remove_unknown_ids_are_not_found() {
        let runtime = Arc::new(MockRuntime::new());
        let service = service_with_mock(runtime);
        let missing = Uuid::new_v4();
        assert!(matches!(
            service.start(missing).await,
            Err(CoreError::NotFound(_))
        ));
        assert!(matches!(
            service.stop(missing).await,
            Err(CoreError::NotFound(_))
        ));
        assert!(matches!(
            service.remove(missing).await,
            Err(CoreError::NotFound(_))
        ));
        assert!(matches!(service.get(missing), Err(CoreError::NotFound(_))));
    }
}
