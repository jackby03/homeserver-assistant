//! Home Assistant instance provisioning through the runtime port.
//!
//! The provisioner deploys the official Home Assistant OCI image as a container
//! on the trusted LAN segment, waits for it to become running and reports the
//! URL the daemon should store in `ServicesConfig::home_assistant_url`.
//!
//! It depends only on [`hs_runtime::RuntimeProvider`], so it is tested with
//! `hs_runtime::mock::MockRuntime` and never touches Incus or the network.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hs_core::error::{CoreError, CoreResult};
use hs_core::models::{InstanceSummary, NetworkSegment, ResourceLimits};
use hs_runtime::{OciDeploySpec, RuntimeError, RuntimeProvider};
use serde::{Deserialize, Serialize};

/// OCI image used for the managed Home Assistant instance.
pub const HA_IMAGE: &str = "ghcr.io/home-assistant/home-assistant:stable";
/// Home Assistant listens on this TCP port inside the container.
pub const DEFAULT_HA_PORT: u16 = 8123;
/// Default CPU limit assigned to the Home Assistant container.
const HA_CPU_LIMIT: f32 = 2.0;
/// Default memory limit assigned to the Home Assistant container (MiB).
const HA_MEMORY_MB: u64 = 2048;
/// How long to wait for the container to report `running` after deployment.
const PROVISION_TIMEOUT: Duration = Duration::from_secs(90);
/// Poll interval while waiting for the container to start.
const PROVISION_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Request to provision the managed Home Assistant instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvisionRequest {
    /// Runtime instance name (lower-case, unique in the provider).
    pub name: String,
    /// IANA timezone passed as `TZ` to the container.
    pub timezone: String,
    /// TCP port published for the Home Assistant web interface.
    pub port: u16,
}

impl Default for ProvisionRequest {
    fn default() -> Self {
        Self {
            name: "home-assistant".to_owned(),
            timezone: "UTC".to_owned(),
            port: DEFAULT_HA_PORT,
        }
    }
}

/// Result of a successful provisioning run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvisionReport {
    /// Runtime summary of the deployed instance.
    pub instance: InstanceSummary,
    /// URL the appliance should use to reach the Home Assistant web interface.
    pub url: String,
}

/// Deploys and starts a managed Home Assistant container.
pub struct HaProvisioner {
    runtime: Arc<dyn RuntimeProvider>,
    timeout: Duration,
}

impl HaProvisioner {
    /// Create a provisioner using the configured runtime provider.
    #[must_use]
    pub fn new(runtime: Arc<dyn RuntimeProvider>) -> Self {
        Self {
            runtime,
            timeout: PROVISION_TIMEOUT,
        }
    }

    /// Create a provisioner with a custom startup timeout (tests, slow boards).
    #[must_use]
    pub fn with_timeout(runtime: Arc<dyn RuntimeProvider>, timeout: Duration) -> Self {
        Self { runtime, timeout }
    }

    /// Deploy Home Assistant and return the running instance and its URL.
    ///
    /// # Errors
    /// Returns [`CoreError::Validation`] when the request fields are invalid,
    /// [`CoreError::Conflict`] when an instance with the same name already
    /// exists, and [`CoreError::Internal`] when the runtime fails or the
    /// container does not become running within the timeout.
    pub async fn provision(&self, request: ProvisionRequest) -> CoreResult<ProvisionReport> {
        validate_request(&request)?;

        let mut env = BTreeMap::new();
        env.insert("TZ".to_owned(), request.timezone.clone());
        let spec = OciDeploySpec {
            name: request.name.clone(),
            image: HA_IMAGE.to_owned(),
            segment: NetworkSegment::Lan,
            env,
            limits: ResourceLimits {
                cpu_limit: Some(HA_CPU_LIMIT),
                memory_mb: Some(HA_MEMORY_MB),
            },
        };

        let mut instance = self
            .runtime
            .deploy_oci(spec)
            .await
            .map_err(CoreError::from)?;
        if !instance.status.is_running() {
            instance = self.wait_until_running(&request.name).await?;
        }
        let url = instance_url(&instance, request.port);
        Ok(ProvisionReport { instance, url })
    }

    /// Poll the provider until the instance reports `running`.
    async fn wait_until_running(&self, name: &str) -> CoreResult<InstanceSummary> {
        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            match self.runtime.get(name).await {
                Ok(instance) if instance.status.is_running() => return Ok(instance),
                Ok(_) => {}
                Err(RuntimeError::NotFound(_)) => {}
                Err(error) => return Err(CoreError::from(error)),
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(CoreError::Internal(format!(
                    "Home Assistant instance `{name}` did not become running within {} seconds",
                    self.timeout.as_secs_f64()
                )));
            }
            tokio::time::sleep(PROVISION_POLL_INTERVAL).await;
        }
    }
}

/// Validate the provisioning request using the shared core validators.
fn validate_request(request: &ProvisionRequest) -> CoreResult<()> {
    hs_core::validation::instance_name(&request.name)?;
    hs_core::validation::timezone(&request.timezone)?;
    hs_core::validation::port(request.port)?;
    Ok(())
}

/// Build the Home Assistant URL from the instance summary.
///
/// The first reported IPv4 address wins. When the provider has not reported an
/// address yet the instance mDNS name (`<name>.local`) is used, which the
/// appliance publishes for LAN workloads.
fn instance_url(instance: &InstanceSummary, port: u16) -> String {
    match instance.ipv4_addresses.first() {
        Some(address) => format!("http://{address}:{port}"),
        None => format!("http://{}.local:{port}", instance.name),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use hs_core::models::{InstanceSummary, RuntimeStatus};
    use hs_runtime::mock::MockRuntime;
    use hs_runtime::RuntimeError;

    use super::{
        instance_url, validate_request, HaProvisioner, ProvisionRequest, DEFAULT_HA_PORT, HA_IMAGE,
    };
    use hs_core::error::CoreError;

    fn request() -> ProvisionRequest {
        ProvisionRequest {
            name: "home-assistant".to_owned(),
            timezone: "Europe/Madrid".to_owned(),
            port: DEFAULT_HA_PORT,
        }
    }

    #[tokio::test]
    async fn provisions_expected_oci_specification() {
        let runtime = Arc::new(MockRuntime::new());
        let provisioner = HaProvisioner::new(runtime.clone());
        let report = provisioner.provision(request()).await.expect("provisioned");

        assert_eq!(report.instance.name, "home-assistant");
        assert_eq!(report.instance.status, RuntimeStatus::Running);
        assert!(
            report.url.starts_with("http://10.0.0."),
            "the provisioner must prefer the instance IPv4 address: {}",
            report.url
        );
        assert!(report.url.ends_with(":8123"));

        let deploys = runtime.deploy_calls().await;
        assert_eq!(deploys.len(), 1);
        let deploy = &deploys[0];
        assert_eq!(deploy.name, "home-assistant");
        assert_eq!(deploy.image, HA_IMAGE);
        assert_eq!(deploy.segment, hs_core::models::NetworkSegment::Lan);
        assert_eq!(
            deploy.env.get("TZ").map(String::as_str),
            Some("Europe/Madrid")
        );
        assert_eq!(deploy.limits.cpu_limit, Some(2.0));
        assert_eq!(deploy.limits.memory_mb, Some(2048));

        let instances = runtime.instances().await;
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].status, RuntimeStatus::Running);
    }

    #[tokio::test]
    async fn rejects_invalid_requests_before_deploying() {
        let runtime = Arc::new(MockRuntime::new());
        let provisioner = HaProvisioner::new(runtime.clone());

        let mut bad_name = request();
        bad_name.name = "Bad Name".to_owned();
        assert!(matches!(
            provisioner.provision(bad_name).await,
            Err(CoreError::Validation { .. })
        ));

        let mut bad_timezone = request();
        bad_timezone.timezone = "not a timezone".to_owned();
        assert!(matches!(
            provisioner.provision(bad_timezone).await,
            Err(CoreError::Validation { .. })
        ));

        let mut bad_port = request();
        bad_port.port = 0;
        assert!(matches!(
            provisioner.provision(bad_port).await,
            Err(CoreError::Validation { .. })
        ));

        assert!(runtime.deploy_calls().await.is_empty());
    }

    #[tokio::test]
    async fn surfaces_runtime_failures() {
        let runtime = Arc::new(MockRuntime::new());
        runtime
            .set_failure(Some(RuntimeError::Unavailable(
                "incus is not running".to_owned(),
            )))
            .await;
        let provisioner = HaProvisioner::new(runtime.clone());
        let error = provisioner
            .provision(request())
            .await
            .expect_err("runtime failure");
        assert!(matches!(error, CoreError::Internal(_)));
    }

    #[tokio::test]
    async fn reports_conflict_when_instance_already_exists() {
        let runtime = Arc::new(MockRuntime::new());
        let provisioner = HaProvisioner::new(runtime.clone());
        provisioner
            .provision(request())
            .await
            .expect("first deploy");
        let error = provisioner
            .provision(request())
            .await
            .expect_err("second deploy must conflict");
        assert!(matches!(error, CoreError::Conflict(_)));
    }

    #[tokio::test]
    async fn startup_wait_times_out_for_missing_instance() {
        let runtime = Arc::new(MockRuntime::new());
        let provisioner = HaProvisioner::with_timeout(runtime, Duration::from_millis(30));
        let error = provisioner
            .wait_until_running("missing-instance")
            .await
            .expect_err("missing instance must time out");
        assert!(matches!(error, CoreError::Internal(_)));
    }

    #[test]
    fn builds_url_from_ip_or_mdns_fallback() {
        let summary = InstanceSummary {
            name: "home-assistant".to_owned(),
            kind: hs_core::models::RuntimeKind::Container,
            status: RuntimeStatus::Running,
            image: HA_IMAGE.to_owned(),
            ipv4_addresses: vec!["10.20.30.10".to_owned()],
            limits: hs_core::models::ResourceLimits::default(),
            created_at: None,
            snapshot_count: 0,
        };
        assert_eq!(
            instance_url(&summary, DEFAULT_HA_PORT),
            "http://10.20.30.10:8123"
        );

        let no_ip = InstanceSummary {
            ipv4_addresses: Vec::new(),
            ..summary
        };
        assert_eq!(
            instance_url(&no_ip, DEFAULT_HA_PORT),
            "http://home-assistant.local:8123"
        );
    }

    #[test]
    fn request_validation_accepts_the_default_request() {
        validate_request(&request()).expect("default request is valid");
    }
}
