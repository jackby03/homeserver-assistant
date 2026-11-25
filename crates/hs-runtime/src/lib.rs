//! Runtime provider port, native Incus adapter and mock adapter.
//!
//! This crate is the only place in the workspace that knows how workloads
//! (system containers, OCI application containers and virtual machines) are
//! created, started, stopped and snapshotted. The rest of the daemon depends on
//! the [`RuntimeProvider`] trait only, which keeps the composition root
//! (`hs-daemon`) free of provider specific details and lets unit tests use
//! [`mock::MockRuntime`] instead of a real Incus daemon.
//!
//! # Layers
//! - [`RuntimeProvider`]: object-safe port used by `hs-apps`, `hs-daemon` and
//!   `hs-homeassistant`.
//! - [`incus`]: production adapter speaking the Incus REST API over its Unix
//!   socket, plus a small transport port so the adapter can be unit tested
//!   without Incus.
//! - [`mock`]: stateful in-memory adapter used by tests and by `hs-daemon`
//!   development mode.
//!
//! # Example
//! ```
//! use std::sync::Arc;
//!
//! use hs_core::models::{InstanceSpec, NetworkSegment, RuntimeKind, RuntimeStatus};
//! use hs_runtime::mock::MockRuntime;
//! use hs_runtime::{OciDeploySpec, RuntimeProvider};
//!
//! # tokio::runtime::Runtime::new().unwrap().block_on(async {
//! let runtime = Arc::new(MockRuntime::new());
//! let spec = InstanceSpec {
//!     name: "debian".to_owned(),
//!     kind: RuntimeKind::Container,
//!     image: "images:debian/13".to_owned(),
//!     segment: Some(NetworkSegment::Lan),
//!     limits: Default::default(),
//!     env: Default::default(),
//!     start: true,
//! };
//! let summary = runtime.create(spec).await.expect("create instance");
//! assert_eq!(summary.status, RuntimeStatus::Running);
//!
//! let app = runtime
//!     .deploy_oci(OciDeploySpec {
//!         name: "home-assistant".to_owned(),
//!         image: "ghcr.io/home-assistant/home-assistant:stable".to_owned(),
//!         segment: NetworkSegment::Lan,
//!         env: Default::default(),
//!         limits: Default::default(),
//!     })
//!     .await
//!     .expect("deploy OCI app");
//! assert_eq!(app.name, "home-assistant");
//! # });
//! ```

#![deny(missing_docs)]

pub mod incus;
pub mod mock;

pub use incus::IncusProvider;
pub use mock::MockRuntime;

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use hs_core::models::{
    ExecOutput, InstanceSpec, InstanceSummary, NetworkSegment, ResourceLimits, RuntimeKind,
    RuntimeStatus, SnapshotSummary,
};

/// Keep at most the last `lines` lines of `text`.
///
/// `lines == 0` returns the whole text unchanged. Providers use this helper to
/// implement the `logs(name, lines)` contract in the same way.
pub(crate) fn tail_lines(text: &str, lines: usize) -> String {
    if lines == 0 {
        return text.to_owned();
    }
    let collected: Vec<&str> = text.lines().collect();
    let start = collected.len().saturating_sub(lines);
    collected[start..].join("\n")
}

/// Convenient result alias for runtime provider operations.
pub type RuntimeResult<T> = Result<T, RuntimeError>;

/// Errors produced by a [`RuntimeProvider`].
///
/// The variants intentionally mirror the errors an HTTP client can distinguish
/// so that `hs-daemon` can map them to the shared API envelope without leaking
/// provider specific payloads.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeError {
    /// The provider or one of its host dependencies is not available.
    #[error("runtime unavailable: {0}")]
    Unavailable(String),

    /// The requested instance, snapshot or operation does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// The request conflicts with the current provider state.
    #[error("conflict: {0}")]
    Conflict(String),

    /// The request is invalid (bad name, image reference, limits, ...).
    #[error("invalid runtime request: {0}")]
    Invalid(String),

    /// The provider backend failed while executing the request.
    #[error("runtime backend error: {0}")]
    Backend(String),

    /// The operation did not complete before the configured deadline.
    #[error("runtime operation timed out: {0}")]
    Timeout(String),
}

impl RuntimeError {
    /// Build an [`RuntimeError::Unavailable`] error.
    #[must_use]
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable(message.into())
    }

    /// Build a [`RuntimeError::NotFound`] error.
    #[must_use]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    /// Build a [`RuntimeError::Backend`] error.
    #[must_use]
    pub fn backend(message: impl Into<String>) -> Self {
        Self::Backend(message.into())
    }

    /// Stable machine-readable code used by the HTTP API layer.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "runtime_unavailable",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Invalid(_) => "invalid_request",
            Self::Backend(_) => "runtime_backend_error",
            Self::Timeout(_) => "runtime_timeout",
        }
    }

    /// Whether retrying the same operation may succeed.
    #[must_use]
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Timeout(_) | Self::Backend(_))
    }
}

impl From<hs_core::CoreError> for RuntimeError {
    fn from(err: hs_core::CoreError) -> Self {
        match err {
            hs_core::CoreError::Validation { field, message } => {
                Self::Invalid(format!("{field}: {message}"))
            }
            hs_core::CoreError::NotFound(what) => Self::NotFound(what),
            hs_core::CoreError::Conflict(what) => Self::Conflict(what),
            hs_core::CoreError::Unauthorized(what) => Self::Unavailable(what),
            hs_core::CoreError::Forbidden(what) => Self::Unavailable(what),
            other => Self::Backend(other.to_string()),
        }
    }
}

impl From<RuntimeError> for hs_core::CoreError {
    fn from(err: RuntimeError) -> Self {
        match err {
            RuntimeError::NotFound(what) => Self::NotFound(what),
            RuntimeError::Conflict(what) => Self::Conflict(what),
            RuntimeError::Invalid(message) => Self::Validation {
                field: "runtime",
                message,
            },
            RuntimeError::Unavailable(message) => {
                Self::Internal(format!("runtime unavailable: {message}"))
            }
            RuntimeError::Backend(message) => Self::Internal(format!("runtime error: {message}")),
            RuntimeError::Timeout(message) => {
                Self::Internal(format!("runtime operation timed out: {message}"))
            }
        }
    }
}

/// Deployment request for an OCI container image.
///
/// This is the runtime-level shape used by `hs-apps` when deploying an
/// application from the catalog. The provider is responsible for translating
/// the image reference into its native image source (for Incus: an `oci`
/// image source with the registry as `server` and the rest as `alias`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OciDeploySpec {
    /// Instance name in the provider.
    pub name: String,
    /// Full OCI image reference, e.g. `ghcr.io/home-assistant/home-assistant:stable`.
    pub image: String,
    /// Network segment the workload must be attached to.
    pub segment: NetworkSegment,
    /// Environment variables passed to the workload.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Resource limits applied to the workload.
    #[serde(default)]
    pub limits: ResourceLimits,
}

impl OciDeploySpec {
    /// Validate the deployment specification.
    ///
    /// # Errors
    /// Returns [`RuntimeError::Invalid`] when the name or image is malformed,
    /// or when the limits are out of range.
    pub fn validate(&self) -> RuntimeResult<()> {
        hs_core::validation::instance_name(&self.name).map_err(|err| match err {
            hs_core::CoreError::Validation { field, message } => {
                RuntimeError::Invalid(format!("{field}: {message}"))
            }
            other => RuntimeError::Backend(other.to_string()),
        })?;
        if self.image.trim().is_empty() {
            return Err(RuntimeError::Invalid("image: must not be empty".to_owned()));
        }
        if self.image.chars().any(char::is_whitespace) {
            return Err(RuntimeError::Invalid(
                "image: must not contain whitespace".to_owned(),
            ));
        }
        self.limits
            .validate()
            .map_err(|err| RuntimeError::Invalid(err.to_string()))?;
        Ok(())
    }
}

/// Port implemented by every workload provider (Incus, mocks, tests).
///
/// Implementations must be cheap to clone through an [`std::sync::Arc`] and
/// safe to call concurrently: the daemon shares one provider between HTTP
/// handlers. Methods follow the provider's synchronous state machine; they
/// return once the action has been accepted and, for long running operations
/// (create/restore/delete), once the provider reports completion.
#[async_trait]
pub trait RuntimeProvider: Send + Sync + 'static {
    /// Stable provider identifier (`incus`, `mock`).
    fn provider_name(&self) -> &'static str;

    /// Cheap synchronous availability probe (no network round trip).
    fn available(&self) -> bool;

    /// List all known instances.
    ///
    /// # Errors
    /// Returns a [`RuntimeError`] when the provider cannot be queried.
    async fn list(&self) -> RuntimeResult<Vec<InstanceSummary>>;

    /// Fetch a single instance by name.
    ///
    /// # Errors
    /// Returns [`RuntimeError::NotFound`] when the instance does not exist.
    async fn get(&self, name: &str) -> RuntimeResult<InstanceSummary>;

    /// Create a new instance.
    ///
    /// # Errors
    /// Returns [`RuntimeError::Conflict`] when the name is already taken and
    /// [`RuntimeError::Invalid`] when the specification is malformed.
    async fn create(&self, spec: InstanceSpec) -> RuntimeResult<InstanceSummary>;

    /// Start an existing instance.
    ///
    /// # Errors
    /// Returns [`RuntimeError::NotFound`] when the instance does not exist.
    async fn start(&self, name: &str) -> RuntimeResult<()>;

    /// Stop an existing instance.
    ///
    /// # Errors
    /// Returns [`RuntimeError::NotFound`] when the instance does not exist.
    async fn stop(&self, name: &str) -> RuntimeResult<()>;

    /// Restart an existing instance.
    ///
    /// # Errors
    /// Returns [`RuntimeError::NotFound`] when the instance does not exist.
    async fn restart(&self, name: &str) -> RuntimeResult<()>;

    /// Delete an instance and its snapshots.
    ///
    /// # Errors
    /// Returns [`RuntimeError::NotFound`] when the instance does not exist.
    async fn delete(&self, name: &str) -> RuntimeResult<()>;

    /// Update CPU and memory limits of an existing instance.
    ///
    /// # Errors
    /// Returns [`RuntimeError::NotFound`] when the instance does not exist.
    async fn update_limits(&self, name: &str, limits: ResourceLimits) -> RuntimeResult<()>;

    /// List snapshots of an instance.
    ///
    /// # Errors
    /// Returns [`RuntimeError::NotFound`] when the instance does not exist.
    async fn list_snapshots(&self, name: &str) -> RuntimeResult<Vec<SnapshotSummary>>;

    /// Create a snapshot of an instance.
    ///
    /// # Errors
    /// Returns [`RuntimeError::NotFound`] when the instance does not exist and
    /// [`RuntimeError::Conflict`] when the snapshot name is already in use.
    async fn snapshot(&self, name: &str, snapshot: &str) -> RuntimeResult<SnapshotSummary>;

    /// Restore an instance from a snapshot.
    ///
    /// # Errors
    /// Returns [`RuntimeError::NotFound`] when either object does not exist.
    async fn restore(&self, name: &str, snapshot: &str) -> RuntimeResult<()>;

    /// Delete a snapshot of an instance.
    ///
    /// # Errors
    /// Returns [`RuntimeError::NotFound`] when either object does not exist.
    async fn delete_snapshot(&self, name: &str, snapshot: &str) -> RuntimeResult<()>;

    /// Return the most recent `lines` log lines of an instance.
    ///
    /// Providers that cannot stream logs return a human readable fallback
    /// message instead of an error.
    ///
    /// # Errors
    /// Returns [`RuntimeError::NotFound`] when the instance does not exist.
    async fn logs(&self, name: &str, lines: usize) -> RuntimeResult<String>;

    /// Execute a one-shot command inside an instance.
    ///
    /// # Errors
    /// Returns [`RuntimeError::Unavailable`] when command execution is not
    /// supported on the current platform and [`RuntimeError::NotFound`] when
    /// the instance does not exist.
    async fn exec(&self, name: &str, command: &[String]) -> RuntimeResult<ExecOutput>;

    /// Create and start an instance from an OCI image.
    ///
    /// # Errors
    /// Returns [`RuntimeError::Invalid`] when the specification is malformed
    /// and [`RuntimeError::Conflict`] when the name is already taken.
    async fn deploy_oci(&self, spec: OciDeploySpec) -> RuntimeResult<InstanceSummary>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oci_deploy_spec_round_trips_as_json() {
        let mut env = BTreeMap::new();
        env.insert("TZ".to_owned(), "Europe/Madrid".to_owned());
        let spec = OciDeploySpec {
            name: "home-assistant".to_owned(),
            image: "ghcr.io/home-assistant/home-assistant:stable".to_owned(),
            segment: NetworkSegment::Lan,
            env,
            limits: ResourceLimits {
                cpu_limit: Some(2.0),
                memory_mb: Some(2048),
            },
        };
        let json = serde_json::to_string(&spec).expect("serialize spec");
        let parsed: OciDeploySpec = serde_json::from_str(&json).expect("deserialize spec");
        assert_eq!(parsed, spec);
    }

    #[test]
    fn oci_deploy_spec_validation() {
        let mut spec = OciDeploySpec {
            name: "ok-name".to_owned(),
            image: "nextcloud:stable".to_owned(),
            segment: NetworkSegment::Dmz,
            env: BTreeMap::new(),
            limits: ResourceLimits::default(),
        };
        spec.validate().expect("valid spec");
        spec.name = "Bad Name".to_owned();
        assert!(matches!(spec.validate(), Err(RuntimeError::Invalid(_))));
        spec.name = "ok-name".to_owned();
        spec.image = "   ".to_owned();
        assert!(matches!(spec.validate(), Err(RuntimeError::Invalid(_))));
        spec.image = "bad image".to_owned();
        assert!(matches!(spec.validate(), Err(RuntimeError::Invalid(_))));
        spec.image = "nextcloud:stable".to_owned();
        spec.limits.cpu_limit = Some(0.0);
        assert!(matches!(spec.validate(), Err(RuntimeError::Invalid(_))));
    }

    #[test]
    fn runtime_error_maps_to_core_error() {
        assert_eq!(
            hs_core::CoreError::from(RuntimeError::NotFound("x".to_owned())).code(),
            "not_found"
        );
        assert_eq!(
            hs_core::CoreError::from(RuntimeError::Conflict("x".to_owned())).code(),
            "conflict"
        );
        assert_eq!(
            hs_core::CoreError::from(RuntimeError::Invalid("x".to_owned())).code(),
            "invalid_request"
        );
        assert_eq!(
            hs_core::CoreError::from(RuntimeError::Unavailable("x".to_owned())).code(),
            "internal_error"
        );
        assert_eq!(
            hs_core::CoreError::from(RuntimeError::Backend("x".to_owned())).code(),
            "internal_error"
        );
        assert_eq!(
            hs_core::CoreError::from(RuntimeError::Timeout("x".to_owned())).code(),
            "internal_error"
        );
    }

    #[test]
    fn error_codes_and_retryability() {
        assert_eq!(
            RuntimeError::Unavailable(String::new()).code(),
            "runtime_unavailable"
        );
        assert_eq!(
            RuntimeError::Backend(String::new()).code(),
            "runtime_backend_error"
        );
        assert_eq!(
            RuntimeError::Timeout(String::new()).code(),
            "runtime_timeout"
        );
        assert!(RuntimeError::Backend(String::new()).retryable());
        assert!(RuntimeError::Timeout(String::new()).retryable());
        assert!(!RuntimeError::NotFound(String::new()).retryable());
        assert!(!RuntimeError::Conflict(String::new()).retryable());
        assert!(!RuntimeError::Invalid(String::new()).retryable());
        assert!(!RuntimeError::Unavailable(String::new()).retryable());
    }
}
