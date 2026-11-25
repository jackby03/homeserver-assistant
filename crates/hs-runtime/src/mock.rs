//! Stateful in-memory [`MockRuntime`] used by tests and by `hs-daemon --dev`.
//!
//! The mock keeps the same state machine as the real provider: instances can be
//! created, started, stopped, snapshotted and deleted, and every call is
//! recorded so tests can assert what the caller asked the runtime to do. It is
//! deliberately dependency free (no filesystem, no network) so it can run in
//! any CI environment.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::Mutex;

use crate::{
    tail_lines, ExecOutput, InstanceSpec, InstanceSummary, NetworkSegment, OciDeploySpec,
    ResourceLimits, RuntimeError, RuntimeKind, RuntimeProvider, RuntimeResult, RuntimeStatus,
    SnapshotSummary,
};

/// In-memory runtime provider that records every mutating call.
///
/// Cloning is not supported on purpose: wrap it in an [`Arc`] and pass it as
/// `Arc<dyn RuntimeProvider>`.
#[derive(Debug)]
pub struct MockRuntime {
    state: Mutex<MockState>,
    available: AtomicBool,
}

impl Default for MockRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for MockState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockState")
            .field("instances", &self.instances.len())
            .field("snapshots", &self.snapshots.len())
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct MockState {
    instances: BTreeMap<String, InstanceSummary>,
    snapshots: BTreeMap<String, Vec<SnapshotSummary>>,
    logs: BTreeMap<String, String>,
    exec_output: Option<ExecOutput>,
    failure: Option<RuntimeError>,
    deploy_calls: Vec<OciDeploySpec>,
    create_calls: Vec<InstanceSpec>,
    exec_calls: Vec<(String, Vec<String>)>,
    logs_calls: Vec<(String, usize)>,
    started: Vec<String>,
    stopped: Vec<String>,
    restarted: Vec<String>,
    deleted: Vec<String>,
    restored: Vec<(String, String)>,
}

impl MockRuntime {
    /// Create an empty, available mock runtime.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MockState::default()),
            available: AtomicBool::new(true),
        }
    }

    /// Create a mock runtime pre-populated with existing instances.
    #[must_use]
    pub fn with_instances(instances: Vec<InstanceSummary>) -> Self {
        let mut state = MockState::default();
        for instance in instances {
            state.instances.insert(instance.name.clone(), instance);
        }
        Self {
            state: Mutex::new(state),
            available: AtomicBool::new(true),
        }
    }

    /// Create a mock runtime pre-populated with the demo instances used by
    /// `hs-daemon --dev` (Home Assistant and AdGuard Home, both running).
    #[must_use]
    pub fn with_demo_instances() -> Self {
        let demo = |name: &str| InstanceSummary {
            name: name.to_owned(),
            kind: RuntimeKind::Container,
            status: RuntimeStatus::Running,
            image: "ghcr.io/home-assistant/home-assistant:stable".to_owned(),
            ipv4_addresses: vec!["10.0.0.10".to_owned()],
            limits: ResourceLimits::default(),
            created_at: Some(Utc::now()),
            snapshot_count: 0,
        };
        Self::with_instances(vec![demo("home-assistant"), demo("adguard-home")])
    }

    /// Add one instance to a freshly constructed mock runtime.
    #[must_use]
    pub fn with_instance(self, instance: InstanceSummary) -> Self {
        let Self { state, available } = self;
        let mut state = state.into_inner();
        state.instances.insert(instance.name.clone(), instance);
        Self {
            state: Mutex::new(state),
            available,
        }
    }

    /// Current instance table (sorted by name).
    pub async fn instances(&self) -> Vec<InstanceSummary> {
        self.state
            .lock()
            .await
            .instances
            .values()
            .cloned()
            .collect()
    }

    /// Every [`RuntimeProvider::deploy_oci`] call, in order.
    pub async fn deploy_calls(&self) -> Vec<OciDeploySpec> {
        self.state.lock().await.deploy_calls.clone()
    }

    /// Every [`RuntimeProvider::create`] call, in order.
    pub async fn create_calls(&self) -> Vec<InstanceSpec> {
        self.state.lock().await.create_calls.clone()
    }

    /// Every [`RuntimeProvider::exec`] call, in order.
    pub async fn exec_calls(&self) -> Vec<(String, Vec<String>)> {
        self.state.lock().await.exec_calls.clone()
    }

    /// Every [`RuntimeProvider::logs`] call, in order.
    pub async fn logs_calls(&self) -> Vec<(String, usize)> {
        self.state.lock().await.logs_calls.clone()
    }

    /// Instance names passed to [`RuntimeProvider::start`].
    pub async fn started(&self) -> Vec<String> {
        self.state.lock().await.started.clone()
    }

    /// Instance names passed to [`RuntimeProvider::stop`].
    pub async fn stopped(&self) -> Vec<String> {
        self.state.lock().await.stopped.clone()
    }

    /// Instance names passed to [`RuntimeProvider::restart`].
    pub async fn restarted(&self) -> Vec<String> {
        self.state.lock().await.restarted.clone()
    }

    /// Instance names passed to [`RuntimeProvider::delete`].
    pub async fn deleted(&self) -> Vec<String> {
        self.state.lock().await.deleted.clone()
    }

    /// Snapshot restore calls as `(instance, snapshot)` pairs.
    pub async fn restored(&self) -> Vec<(String, String)> {
        self.state.lock().await.restored.clone()
    }

    /// Make every subsequent operation fail with `failure`.
    ///
    /// Pass `None` to restore normal behavior.
    pub async fn set_failure(&self, failure: Option<RuntimeError>) {
        self.state.lock().await.failure = failure;
    }

    /// Toggle the result of [`RuntimeProvider::available`].
    pub fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::SeqCst);
    }

    /// Set the output returned by [`RuntimeProvider::exec`].
    pub async fn set_exec_output(&self, output: ExecOutput) {
        self.state.lock().await.exec_output = Some(output);
    }

    /// Set the log text returned by [`RuntimeProvider::logs`] for an instance.
    pub async fn set_logs(&self, instance: &str, text: impl Into<String>) {
        self.state
            .lock()
            .await
            .logs
            .insert(instance.to_owned(), text.into());
    }

    /// Insert or replace an instance, bypassing validation (test seam).
    pub async fn push_instance(&self, instance: InstanceSummary) {
        self.state
            .lock()
            .await
            .instances
            .insert(instance.name.clone(), instance);
    }

    /// Validate an instance name the same way the real provider does.
    fn validate_name(name: &str) -> RuntimeResult<()> {
        hs_core::validation::instance_name(name)
            .map_err(|err| RuntimeError::Invalid(err.to_string()))
    }

    /// Ensure the mock has not been configured to fail.
    fn fail_if_configured(state: &MockState) -> RuntimeResult<()> {
        match &state.failure {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    /// Store an instance, returning the summary the provider would report.
    fn insert_summary(state: &mut MockState, spec: InstanceSpec) -> InstanceSummary {
        // Assign a deterministic documentation-range address per segment so
        // reverse-proxy and SSH tests can resolve an upstream.
        let host_octet = u8::try_from(state.instances.len() + 10).unwrap_or(250);
        let address = match spec.segment {
            Some(NetworkSegment::Dmz) => format!("10.20.30.{host_octet}"),
            _ => format!("10.0.0.{host_octet}"),
        };
        let summary = InstanceSummary {
            name: spec.name.clone(),
            kind: spec.kind,
            status: if spec.start {
                RuntimeStatus::Running
            } else {
                RuntimeStatus::Stopped
            },
            image: spec.image,
            ipv4_addresses: vec![address],
            limits: spec.limits,
            created_at: Some(Utc::now()),
            snapshot_count: 0,
        };
        state.instances.insert(spec.name, summary.clone());
        summary
    }
}

#[async_trait]
impl RuntimeProvider for MockRuntime {
    fn provider_name(&self) -> &'static str {
        "mock"
    }

    fn available(&self) -> bool {
        self.available.load(Ordering::SeqCst)
    }

    async fn list(&self) -> RuntimeResult<Vec<InstanceSummary>> {
        let state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        Ok(state.instances.values().cloned().collect())
    }

    async fn get(&self, name: &str) -> RuntimeResult<InstanceSummary> {
        Self::validate_name(name)?;
        let state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        state
            .instances
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::not_found(format!("instance `{name}` not found")))
    }

    async fn create(&self, spec: InstanceSpec) -> RuntimeResult<InstanceSummary> {
        spec.validate()
            .map_err(|err| RuntimeError::Invalid(err.to_string()))?;
        let mut state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        if state.instances.contains_key(&spec.name) {
            return Err(RuntimeError::Conflict(format!(
                "instance `{}` already exists",
                spec.name
            )));
        }
        state.create_calls.push(spec.clone());
        Ok(Self::insert_summary(&mut state, spec))
    }

    async fn start(&self, name: &str) -> RuntimeResult<()> {
        Self::validate_name(name)?;
        let mut state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        let instance = state
            .instances
            .get_mut(name)
            .ok_or_else(|| RuntimeError::not_found(format!("instance `{name}` not found")))?;
        instance.status = RuntimeStatus::Running;
        state.started.push(name.to_owned());
        Ok(())
    }

    async fn stop(&self, name: &str) -> RuntimeResult<()> {
        Self::validate_name(name)?;
        let mut state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        let instance = state
            .instances
            .get_mut(name)
            .ok_or_else(|| RuntimeError::not_found(format!("instance `{name}` not found")))?;
        instance.status = RuntimeStatus::Stopped;
        state.stopped.push(name.to_owned());
        Ok(())
    }

    async fn restart(&self, name: &str) -> RuntimeResult<()> {
        Self::validate_name(name)?;
        let mut state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        let instance = state
            .instances
            .get_mut(name)
            .ok_or_else(|| RuntimeError::not_found(format!("instance `{name}` not found")))?;
        instance.status = RuntimeStatus::Running;
        state.restarted.push(name.to_owned());
        Ok(())
    }

    async fn delete(&self, name: &str) -> RuntimeResult<()> {
        Self::validate_name(name)?;
        let mut state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        if state.instances.remove(name).is_none() {
            return Err(RuntimeError::not_found(format!(
                "instance `{name}` not found"
            )));
        }
        state.snapshots.remove(name);
        state.logs.remove(name);
        state.deleted.push(name.to_owned());
        Ok(())
    }

    async fn update_limits(&self, name: &str, limits: ResourceLimits) -> RuntimeResult<()> {
        Self::validate_name(name)?;
        limits
            .validate()
            .map_err(|err| RuntimeError::Invalid(err.to_string()))?;
        let mut state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        let instance = state
            .instances
            .get_mut(name)
            .ok_or_else(|| RuntimeError::not_found(format!("instance `{name}` not found")))?;
        instance.limits = limits;
        Ok(())
    }

    async fn list_snapshots(&self, name: &str) -> RuntimeResult<Vec<SnapshotSummary>> {
        Self::validate_name(name)?;
        let state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        if !state.instances.contains_key(name) {
            return Err(RuntimeError::not_found(format!(
                "instance `{name}` not found"
            )));
        }
        Ok(state.snapshots.get(name).cloned().unwrap_or_default())
    }

    async fn snapshot(&self, name: &str, snapshot: &str) -> RuntimeResult<SnapshotSummary> {
        Self::validate_name(name)?;
        validate_snapshot_name(snapshot)?;
        let mut state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        if !state.instances.contains_key(name) {
            return Err(RuntimeError::not_found(format!(
                "instance `{name}` not found"
            )));
        }
        let entries = state.snapshots.entry(name.to_owned()).or_default();
        if entries.iter().any(|entry| entry.name == snapshot) {
            return Err(RuntimeError::Conflict(format!(
                "snapshot `{snapshot}` already exists for instance `{name}`"
            )));
        }
        let summary = SnapshotSummary {
            name: snapshot.to_owned(),
            created_at: Some(Utc::now()),
            size_bytes: 0,
        };
        entries.push(summary.clone());
        let count = state
            .snapshots
            .get(name)
            .map_or(0, |entries| entries.len() as u32);
        if let Some(instance) = state.instances.get_mut(name) {
            instance.snapshot_count = count;
        }
        Ok(summary)
    }

    async fn restore(&self, name: &str, snapshot: &str) -> RuntimeResult<()> {
        Self::validate_name(name)?;
        validate_snapshot_name(snapshot)?;
        let mut state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        let found = state
            .snapshots
            .get(name)
            .is_some_and(|entries| entries.iter().any(|entry| entry.name == snapshot));
        if !found {
            return Err(RuntimeError::not_found(format!(
                "snapshot `{snapshot}` not found for instance `{name}`"
            )));
        }
        state.restored.push((name.to_owned(), snapshot.to_owned()));
        Ok(())
    }

    async fn delete_snapshot(&self, name: &str, snapshot: &str) -> RuntimeResult<()> {
        Self::validate_name(name)?;
        validate_snapshot_name(snapshot)?;
        let mut state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        let Some(entries) = state.snapshots.get_mut(name) else {
            return Err(RuntimeError::not_found(format!(
                "instance `{name}` not found"
            )));
        };
        let before = entries.len();
        entries.retain(|entry| entry.name != snapshot);
        if entries.len() == before {
            return Err(RuntimeError::not_found(format!(
                "snapshot `{snapshot}` not found for instance `{name}`"
            )));
        }
        let count = entries.len() as u32;
        if let Some(instance) = state.instances.get_mut(name) {
            instance.snapshot_count = count;
        }
        Ok(())
    }

    async fn logs(&self, name: &str, lines: usize) -> RuntimeResult<String> {
        Self::validate_name(name)?;
        let mut state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        if !state.instances.contains_key(name) {
            return Err(RuntimeError::not_found(format!(
                "instance `{name}` not found"
            )));
        }
        state.logs_calls.push((name.to_owned(), lines));
        let text = state
            .logs
            .get(name)
            .cloned()
            .unwrap_or_else(|| format!("no logs available for instance `{name}`"));
        Ok(tail_lines(&text, lines))
    }

    async fn exec(&self, name: &str, command: &[String]) -> RuntimeResult<ExecOutput> {
        Self::validate_name(name)?;
        let mut state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        if !state.instances.contains_key(name) {
            return Err(RuntimeError::not_found(format!(
                "instance `{name}` not found"
            )));
        }
        state.exec_calls.push((name.to_owned(), command.to_vec()));
        Ok(state.exec_output.clone().unwrap_or_else(|| ExecOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
        }))
    }

    async fn deploy_oci(&self, spec: OciDeploySpec) -> RuntimeResult<InstanceSummary> {
        spec.validate()?;
        {
            let mut state = self.state.lock().await;
            Self::fail_if_configured(&state)?;
            if state.instances.contains_key(&spec.name) {
                return Err(RuntimeError::Conflict(format!(
                    "instance `{}` already exists",
                    spec.name
                )));
            }
            state.deploy_calls.push(spec.clone());
            state.create_calls.push(InstanceSpec {
                name: spec.name.clone(),
                kind: RuntimeKind::Container,
                image: spec.image.clone(),
                segment: Some(spec.segment),
                limits: spec.limits,
                env: spec.env.clone(),
                start: true,
            });
        }
        let mut state = self.state.lock().await;
        Self::fail_if_configured(&state)?;
        Ok(Self::insert_summary(
            &mut state,
            InstanceSpec {
                name: spec.name,
                kind: RuntimeKind::Container,
                image: spec.image,
                segment: Some(spec.segment),
                limits: spec.limits,
                env: spec.env,
                start: true,
            },
        ))
    }
}

/// Snapshot names use the same conservative character set as instance names.
fn validate_snapshot_name(value: &str) -> RuntimeResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 63
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(RuntimeError::Invalid(format!(
            "invalid snapshot name `{value}`: use lowercase letters, digits, `-`, `_` or `.`"
        )))
    }
}

/// Convenience alias used by daemon dev mode.
pub type SharedMockRuntime = Arc<MockRuntime>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NetworkSegment, RuntimeKind};
    use std::collections::BTreeMap;

    fn summary(name: &str, status: RuntimeStatus) -> InstanceSummary {
        InstanceSummary {
            name: name.to_owned(),
            kind: RuntimeKind::Container,
            status,
            image: "images:debian/13".to_owned(),
            ipv4_addresses: vec!["10.0.0.5".to_owned()],
            limits: ResourceLimits::default(),
            created_at: None,
            snapshot_count: 0,
        }
    }

    fn spec(name: &str, start: bool) -> InstanceSpec {
        InstanceSpec {
            name: name.to_owned(),
            kind: RuntimeKind::Container,
            image: "images:debian/13".to_owned(),
            segment: Some(NetworkSegment::Lan),
            limits: ResourceLimits::default(),
            env: BTreeMap::new(),
            start,
        }
    }

    #[tokio::test]
    async fn create_start_stop_delete_state_machine() {
        let runtime = MockRuntime::new();
        let created = runtime.create(spec("demo", false)).await.unwrap();
        assert_eq!(created.status, RuntimeStatus::Stopped);
        assert_eq!(runtime.list().await.unwrap().len(), 1);

        runtime.start("demo").await.unwrap();
        assert_eq!(
            runtime.get("demo").await.unwrap().status,
            RuntimeStatus::Running
        );
        runtime.stop("demo").await.unwrap();
        assert_eq!(
            runtime.get("demo").await.unwrap().status,
            RuntimeStatus::Stopped
        );
        runtime.restart("demo").await.unwrap();
        assert_eq!(
            runtime.get("demo").await.unwrap().status,
            RuntimeStatus::Running
        );

        runtime.delete("demo").await.unwrap();
        assert!(runtime.list().await.unwrap().is_empty());
        assert_eq!(runtime.started().await, vec!["demo".to_owned()]);
        assert_eq!(runtime.stopped().await, vec!["demo".to_owned()]);
        assert_eq!(runtime.restarted().await, vec!["demo".to_owned()]);
        assert_eq!(runtime.deleted().await, vec!["demo".to_owned()]);
    }

    #[tokio::test]
    async fn create_conflict_and_missing_instance() {
        let runtime = MockRuntime::new();
        runtime.create(spec("demo", true)).await.unwrap();
        assert!(matches!(
            runtime.create(spec("demo", true)).await,
            Err(RuntimeError::Conflict(_))
        ));
        assert!(matches!(
            runtime.get("missing").await,
            Err(RuntimeError::NotFound(_))
        ));
        assert!(matches!(
            runtime.start("missing").await,
            Err(RuntimeError::NotFound(_))
        ));
        assert!(matches!(
            runtime.get("Bad Name").await,
            Err(RuntimeError::Invalid(_))
        ));
        assert!(matches!(
            runtime.create(spec("Bad Name", true)).await,
            Err(RuntimeError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn snapshots_are_stateful() {
        let runtime = MockRuntime::new();
        runtime.create(spec("demo", true)).await.unwrap();
        let snap = runtime.snapshot("demo", "snap0").await.unwrap();
        assert_eq!(snap.name, "snap0");
        assert_eq!(runtime.list_snapshots("demo").await.unwrap().len(), 1);
        assert_eq!(runtime.get("demo").await.unwrap().snapshot_count, 1);
        assert!(matches!(
            runtime.snapshot("demo", "snap0").await,
            Err(RuntimeError::Conflict(_))
        ));
        runtime.restore("demo", "snap0").await.unwrap();
        assert_eq!(
            runtime.restored().await,
            vec![("demo".to_owned(), "snap0".to_owned())]
        );
        runtime.delete_snapshot("demo", "snap0").await.unwrap();
        assert!(runtime.list_snapshots("demo").await.unwrap().is_empty());
        assert!(matches!(
            runtime.restore("demo", "snap0").await,
            Err(RuntimeError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn deploy_oci_records_spec_and_instance() {
        let runtime = MockRuntime::new();
        let deploy = OciDeploySpec {
            name: "home-assistant".to_owned(),
            image: "ghcr.io/home-assistant/home-assistant:stable".to_owned(),
            segment: NetworkSegment::Dmz,
            env: BTreeMap::from([("TZ".to_owned(), "Europe/Madrid".to_owned())]),
            limits: ResourceLimits {
                cpu_limit: Some(2.0),
                memory_mb: Some(2048),
            },
        };
        let summary = runtime.deploy_oci(deploy.clone()).await.unwrap();
        assert_eq!(summary.status, RuntimeStatus::Running);
        assert_eq!(runtime.deploy_calls().await, vec![deploy]);
        assert_eq!(runtime.create_calls().await.len(), 1);
        assert_eq!(
            runtime.get("home-assistant").await.unwrap().status,
            RuntimeStatus::Running
        );
    }

    #[tokio::test]
    async fn failure_injection_marks_operations_failed() {
        let runtime = MockRuntime::new();
        runtime
            .set_failure(Some(RuntimeError::Backend("incus down".to_owned())))
            .await;
        assert!(matches!(
            runtime.create(spec("demo", true)).await,
            Err(RuntimeError::Backend(_))
        ));
        runtime.set_failure(None).await;
        runtime.create(spec("demo", true)).await.unwrap();
        assert!(runtime.get("demo").await.is_ok());
    }

    #[tokio::test]
    async fn available_flag_is_controllable() {
        let runtime = MockRuntime::new();
        assert!(runtime.available());
        runtime.set_available(false);
        assert!(!runtime.available());
    }

    #[tokio::test]
    async fn with_instances_seeds_state_and_returns_sorted_list() {
        let runtime = MockRuntime::with_instances(vec![
            summary("b", RuntimeStatus::Stopped),
            summary("a", RuntimeStatus::Running),
        ]);
        let names: Vec<_> = runtime
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|instance| instance.name)
            .collect();
        assert_eq!(names, vec!["a".to_owned(), "b".to_owned()]);

        let runtime = runtime.with_instance(summary("c", RuntimeStatus::Running));
        assert_eq!(runtime.list().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn exec_and_logs_are_recorded() {
        let runtime = MockRuntime::new();
        runtime.create(spec("demo", true)).await.unwrap();
        runtime
            .set_exec_output(crate::ExecOutput {
                stdout: "hello\n".to_owned(),
                stderr: String::new(),
                exit_code: Some(0),
            })
            .await;
        let output = runtime
            .exec("demo", &["echo".to_owned(), "hello".to_owned()])
            .await
            .unwrap();
        assert_eq!(output.stdout, "hello\n");
        assert_eq!(
            runtime.exec_calls().await,
            vec![(
                "demo".to_owned(),
                vec!["echo".to_owned(), "hello".to_owned()]
            )]
        );

        runtime.set_logs("demo", "line1\nline2\nline3").await;
        assert_eq!(runtime.logs("demo", 2).await.unwrap(), "line2\nline3");
        assert_eq!(runtime.logs_calls().await, vec![("demo".to_owned(), 2)]);
        let fallback = runtime.logs("demo", 10).await.unwrap();
        assert!(fallback.contains("line1"));
    }

    #[tokio::test]
    async fn update_limits_is_applied() {
        let runtime = MockRuntime::new();
        runtime.create(spec("demo", true)).await.unwrap();
        let limits = ResourceLimits {
            cpu_limit: Some(1.5),
            memory_mb: Some(1024),
        };
        runtime.update_limits("demo", limits).await.unwrap();
        assert_eq!(runtime.get("demo").await.unwrap().limits, limits);
        assert!(matches!(
            runtime
                .update_limits(
                    "demo",
                    ResourceLimits {
                        cpu_limit: Some(-1.0),
                        memory_mb: None,
                    },
                )
                .await,
            Err(RuntimeError::Invalid(_))
        ));
    }
}
