//! Native Incus adapter speaking the Incus REST API over its Unix socket.
//!
//! The adapter is split into two parts:
//! - [`IncusProvider`] implements the workspace [`RuntimeProvider`] port.
//! - [`IncusTransport`] is a tiny HTTP request port. Production uses a hyper
//!   client bound to `/var/lib/incus/unix.socket` (Unix only); unit tests use an
//!   in-memory fake so they never require Incus, root or a real socket.
//!
//! Only a deliberately small API surface is used: `/1.0/instances`,
//! `/1.0/instances/{name}/state`, `/1.0/instances/{name}/snapshots`,
//! `/1.0/instances/{name}/logs` and `/1.0/operations/{id}/wait`. Background
//! operations returned by mutating calls are awaited until they finish or the
//! configured deadline expires.
//!
//! # Example
//! ```no_run
//! use hs_runtime::incus::{DEFAULT_INCUS_SOCKET, IncusProvider};
//!
//! # tokio::runtime::Runtime::new().unwrap().block_on(async {
//! let provider = IncusProvider::new(DEFAULT_INCUS_SOCKET);
//! provider.check().await.expect("Incus must be reachable from hs-daemon");
//! # });
//! ```

pub mod api;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::{
    tail_lines, ExecOutput, InstanceSpec, InstanceSummary, NetworkSegment, OciDeploySpec,
    ResourceLimits, RuntimeError, RuntimeKind, RuntimeProvider, RuntimeResult, SnapshotSummary,
};

use api::{Envelope, IncusInstance, IncusSnapshot, Operation};

/// Default path of the Incus daemon Unix socket on Debian/Ubuntu appliances.
pub const DEFAULT_INCUS_SOCKET: &str = "/var/lib/incus/unix.socket";

/// Default Incus managed bridge used for trusted LAN workloads.
pub const DEFAULT_LAN_BRIDGE: &str = "incusbr0";

/// Default bridge used for DMZ workloads (matches [`hs_core::config::DmzConfig`]).
pub const DEFAULT_DMZ_BRIDGE: &str = hs_core::config::DEFAULT_DMZ_BRIDGE;

/// Default timeout for Incus background operations (image pulls can be slow).
pub const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(300);

/// HTTP method used by the [`IncusTransport`] port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    /// `GET`
    Get,
    /// `POST`
    Post,
    /// `PATCH`
    Patch,
    /// `DELETE`
    Delete,
}

impl HttpMethod {
    /// Wire representation of the method.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

/// Minimal HTTP response returned by an [`IncusTransport`].
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Raw response body.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Whether the status code is in the 2xx range.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Response body decoded as lossy UTF-8.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Small HTTP request port used by [`IncusProvider`].
///
/// Implementations must be cheap to share and safe to call concurrently. The
/// production implementation speaks HTTP/1.1 over the Incus Unix socket; tests
/// provide a scripted fake.
#[async_trait]
pub trait IncusTransport: Send + Sync {
    /// Cheap synchronous availability probe (for example, does the socket exist).
    fn available(&self) -> bool;

    /// Perform one request against `path` on the Incus API root.
    ///
    /// # Errors
    /// Returns a [`RuntimeError`] when the request cannot be performed.
    async fn request(
        &self,
        method: HttpMethod,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> RuntimeResult<HttpResponse>;
}

/// Production transport: hyper HTTP/1.1 client over a Unix domain socket.
#[cfg(unix)]
mod unix_transport {
    use std::path::PathBuf;

    use async_trait::async_trait;
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper_util::client::legacy::Client;
    use hyperlocal::{UnixClientExt, Uri};

    use super::{HttpMethod, HttpResponse, IncusTransport, RuntimeError, RuntimeResult};

    /// Hyper client bound to a Unix socket.
    pub(crate) struct UnixTransport {
        socket: PathBuf,
        client: Client<hyperlocal::UnixConnector, Full<Bytes>>,
    }

    impl UnixTransport {
        pub(crate) fn new(socket: impl Into<PathBuf>) -> Self {
            Self {
                socket: socket.into(),
                client: Client::unix(),
            }
        }
    }

    #[async_trait]
    impl IncusTransport for UnixTransport {
        fn available(&self) -> bool {
            self.socket.exists()
        }

        async fn request(
            &self,
            method: HttpMethod,
            path: &str,
            body: Option<Vec<u8>>,
        ) -> RuntimeResult<HttpResponse> {
            let uri: hyper::Uri = Uri::new(&self.socket, path).into();
            let request = hyper::Request::builder()
                .method(method.as_str())
                .uri(uri)
                .header(hyper::header::CONTENT_TYPE, "application/json")
                .header(
                    hyper::header::USER_AGENT,
                    concat!("hs-runtime/", env!("CARGO_PKG_VERSION")),
                )
                .body(Full::new(Bytes::from(body.unwrap_or_default())))
                .map_err(|err| {
                    RuntimeError::Backend(format!("failed to build Incus request: {err}"))
                })?;
            let response = self.client.request(request).await.map_err(|err| {
                RuntimeError::Unavailable(format!(
                    "cannot reach the Incus socket at {}: {err}",
                    self.socket.display()
                ))
            })?;
            let status = response.status().as_u16();
            let collected = response.into_body().collect().await.map_err(|err| {
                RuntimeError::Backend(format!("failed to read the Incus response: {err}"))
            })?;
            Ok(HttpResponse {
                status,
                body: collected.to_bytes().to_vec(),
            })
        }
    }
}

/// Fallback transport used on non-Unix build hosts.
#[cfg(not(unix))]
struct UnsupportedTransport;

#[cfg(not(unix))]
#[async_trait]
impl IncusTransport for UnsupportedTransport {
    fn available(&self) -> bool {
        false
    }

    async fn request(
        &self,
        _method: HttpMethod,
        _path: &str,
        _body: Option<Vec<u8>>,
    ) -> RuntimeResult<HttpResponse> {
        Err(RuntimeError::Unavailable(
            "the Incus adapter requires a Unix host with the Incus daemon".to_owned(),
        ))
    }
}

#[cfg(unix)]
fn default_transport(socket: &Path) -> Arc<dyn IncusTransport> {
    Arc::new(unix_transport::UnixTransport::new(socket.to_path_buf()))
}

#[cfg(not(unix))]
fn default_transport(_socket: &Path) -> Arc<dyn IncusTransport> {
    Arc::new(UnsupportedTransport)
}

/// Incus runtime provider.
///
/// The provider owns no background tasks: every call opens (through the
/// transport) an HTTP request, awaits the Incus operation when needed and
/// returns the normalized domain summary.
pub struct IncusProvider {
    socket_path: PathBuf,
    transport: Arc<dyn IncusTransport>,
    operation_timeout: Duration,
    lan_bridge: String,
    dmz_bridge: String,
}

impl std::fmt::Debug for IncusProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncusProvider")
            .field("socket_path", &self.socket_path)
            .field("operation_timeout", &self.operation_timeout)
            .field("lan_bridge", &self.lan_bridge)
            .field("dmz_bridge", &self.dmz_bridge)
            .finish_non_exhaustive()
    }
}

impl IncusProvider {
    /// Create a provider for the Incus socket at `socket_path`.
    #[must_use]
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        let socket_path = socket_path.into();
        let transport = default_transport(&socket_path);
        Self {
            socket_path,
            transport,
            operation_timeout: DEFAULT_OPERATION_TIMEOUT,
            lan_bridge: DEFAULT_LAN_BRIDGE.to_owned(),
            dmz_bridge: DEFAULT_DMZ_BRIDGE.to_owned(),
        }
    }

    /// Create a provider with an explicit transport (used by tests and by hosts
    /// that already manage their own HTTP stack).
    #[must_use]
    pub fn with_transport(
        socket_path: impl Into<PathBuf>,
        transport: Arc<dyn IncusTransport>,
    ) -> Self {
        Self {
            socket_path: socket_path.into(),
            transport,
            operation_timeout: DEFAULT_OPERATION_TIMEOUT,
            lan_bridge: DEFAULT_LAN_BRIDGE.to_owned(),
            dmz_bridge: DEFAULT_DMZ_BRIDGE.to_owned(),
        }
    }

    /// Override the deadline applied to Incus background operations.
    #[must_use]
    pub fn with_operation_timeout(mut self, timeout: Duration) -> Self {
        self.operation_timeout = timeout;
        self
    }

    /// Override the bridges used for LAN and DMZ workloads.
    #[must_use]
    pub fn with_bridges(
        mut self,
        lan_bridge: impl Into<String>,
        dmz_bridge: impl Into<String>,
    ) -> Self {
        self.lan_bridge = lan_bridge.into();
        self.dmz_bridge = dmz_bridge.into();
        self
    }

    /// Path of the Incus Unix socket used by this provider.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Verify that the Incus API is reachable and healthy.
    ///
    /// # Errors
    /// Returns [`RuntimeError::Unavailable`] when the socket is missing and a
    /// backend error when Incus answers with a failure envelope.
    pub async fn check(&self) -> RuntimeResult<()> {
        if !self.transport.available() {
            return Err(RuntimeError::Unavailable(format!(
                "Incus socket `{}` is not present",
                self.socket_path.display()
            )));
        }
        let response = self.send(HttpMethod::Get, "/1.0").await?;
        if !response.is_success() {
            return Err(http_error(response.status, &error_detail(&response)));
        }
        let envelope: Envelope<serde_json::Value> = Envelope::parse(&response.body)?;
        if envelope.is_failure() {
            return Err(RuntimeError::Backend(format!(
                "Incus health check failed: {}",
                envelope.error_message()
            )));
        }
        tracing::debug!(
            socket = %self.socket_path.display(),
            "Incus runtime provider is reachable"
        );
        Ok(())
    }

    /// Send a request without a body.
    async fn send(&self, method: HttpMethod, path: &str) -> RuntimeResult<HttpResponse> {
        self.transport.request(method, path, None).await
    }

    /// Send a JSON request.
    async fn send_json(
        &self,
        method: HttpMethod,
        path: &str,
        body: &serde_json::Value,
    ) -> RuntimeResult<HttpResponse> {
        let payload = serde_json::to_vec(body).map_err(|err| {
            RuntimeError::Backend(format!("failed to encode the Incus request body: {err}"))
        })?;
        self.transport.request(method, path, Some(payload)).await
    }

    /// Decode a synchronous envelope payload.
    fn sync_metadata<T: DeserializeOwned>(
        &self,
        response: HttpResponse,
        context: &str,
    ) -> RuntimeResult<T> {
        if !response.is_success() {
            return Err(http_error(response.status, &error_detail(&response)));
        }
        let envelope: Envelope<T> = Envelope::parse(&response.body)?;
        if envelope.is_failure() {
            return Err(RuntimeError::Backend(format!(
                "{context}: {}",
                envelope.error_message()
            )));
        }
        envelope.into_metadata()
    }

    /// Wait for the operation returned by a mutating request, if any.
    async fn finish_operation_response(
        &self,
        response: HttpResponse,
        context: &str,
    ) -> RuntimeResult<()> {
        if !response.is_success() {
            return Err(http_error(response.status, &error_detail(&response)));
        }
        let envelope: Envelope<serde_json::Value> = Envelope::parse(&response.body)?;
        if envelope.is_failure() {
            return Err(RuntimeError::Backend(format!(
                "{context}: {}",
                envelope.error_message()
            )));
        }
        if envelope.kind.eq_ignore_ascii_case("async") {
            let metadata = envelope.into_metadata()?;
            let operation: Operation = serde_json::from_value(metadata).map_err(|err| {
                RuntimeError::Backend(format!("{context}: malformed Incus operation: {err}"))
            })?;
            if operation.id.is_empty() {
                return Err(RuntimeError::Backend(format!(
                    "{context}: Incus returned an asynchronous response without an operation id"
                )));
            }
            self.wait_operation(&operation.id, context).await?;
        }
        Ok(())
    }

    /// Poll `/1.0/operations/{id}/wait` until the operation finishes or the
    /// configured deadline expires.
    async fn wait_operation(&self, operation_id: &str, context: &str) -> RuntimeResult<()> {
        let deadline = tokio::time::Instant::now() + self.operation_timeout;
        let encoded_id = encode_path_segment(operation_id);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(RuntimeError::Timeout(format!(
                    "{context}: Incus operation `{operation_id}` did not finish within {:?}",
                    self.operation_timeout
                )));
            }
            let timeout_secs = remaining.as_secs().clamp(1, 300);
            let path = format!("/1.0/operations/{encoded_id}/wait?timeout={timeout_secs}");
            let response = self.send(HttpMethod::Get, &path).await?;
            if response.status == 408 || response.status == 504 {
                // Server-side wait timed out but the operation is still running.
                tokio::time::sleep(remaining.min(Duration::from_millis(100))).await;
                continue;
            }
            if !response.is_success() {
                return Err(http_error(response.status, &error_detail(&response)));
            }
            let envelope: Envelope<Operation> = Envelope::parse(&response.body)?;
            if envelope.is_failure() {
                return Err(RuntimeError::Backend(format!(
                    "{context}: {}",
                    envelope.error_message()
                )));
            }
            let operation = envelope.into_metadata()?;
            if operation.is_failure() {
                return Err(RuntimeError::Backend(format!(
                    "{context}: {}",
                    operation.error_message()
                )));
            }
            if operation.is_success() {
                return Ok(());
            }
            tokio::time::sleep(remaining.min(Duration::from_millis(100))).await;
        }
    }

    /// Run a lifecycle action (`start`, `stop`, `restart`) on an instance.
    async fn state_action(&self, name: &str, action: &str, force: bool) -> RuntimeResult<()> {
        validate_instance_name(name)?;
        let body = json!({
            "action": action,
            "force": force,
            "timeout": 60,
        });
        let path = format!("{}/state", instance_path(name));
        let response = self.send_json(HttpMethod::Post, &path, &body).await?;
        self.finish_operation_response(response, &format!("{action} instance `{name}`"))
            .await
    }

    /// Build the Incus `config` object for a new instance.
    fn instance_config(
        &self,
        env: &BTreeMap<String, String>,
        limits: &ResourceLimits,
    ) -> serde_json::Value {
        let mut config = serde_json::Map::new();
        if let Some(cpu) = limits.cpu_limit {
            config.insert("limits.cpu".to_owned(), json!(format_cpu(cpu)));
        }
        if let Some(memory_mb) = limits.memory_mb {
            config.insert("limits.memory".to_owned(), json!(format!("{memory_mb}MiB")));
        }
        for (key, value) in env {
            config.insert(format!("environment.{key}"), json!(value));
        }
        serde_json::Value::Object(config)
    }

    /// Build the `devices` object that attaches the instance to a segment bridge.
    fn segment_devices(&self, segment: Option<NetworkSegment>) -> serde_json::Value {
        let mut devices = serde_json::Map::new();
        if let Some(segment) = segment {
            let parent = match segment {
                NetworkSegment::Lan => self.lan_bridge.as_str(),
                NetworkSegment::Dmz => self.dmz_bridge.as_str(),
            };
            devices.insert(
                "eth0".to_owned(),
                json!({
                    "type": "nic",
                    "name": "eth0",
                    "nictype": "bridged",
                    "parent": parent,
                }),
            );
        }
        serde_json::Value::Object(devices)
    }

    /// Build the JSON body used to create a container or VM.
    fn create_body(&self, spec: &InstanceSpec) -> serde_json::Value {
        json!({
            "name": spec.name,
            "type": match spec.kind {
                RuntimeKind::Container => "container",
                RuntimeKind::Vm => "virtual-machine",
            },
            "source": {
                "type": "image",
                "alias": spec.image,
            },
            "profiles": ["default"],
            "config": self.instance_config(&spec.env, &spec.limits),
            "devices": self.segment_devices(spec.segment),
            "start": spec.start,
        })
    }
}

#[async_trait]
impl RuntimeProvider for IncusProvider {
    fn provider_name(&self) -> &'static str {
        "incus"
    }

    fn available(&self) -> bool {
        self.transport.available()
    }

    async fn list(&self) -> RuntimeResult<Vec<InstanceSummary>> {
        let response = self
            .send(HttpMethod::Get, "/1.0/instances?recursion=1")
            .await?;
        let instances: Vec<IncusInstance> = self.sync_metadata(response, "list instances")?;
        Ok(instances
            .into_iter()
            .map(IncusInstance::into_summary)
            .collect())
    }

    async fn get(&self, name: &str) -> RuntimeResult<InstanceSummary> {
        validate_instance_name(name)?;
        let response = self.send(HttpMethod::Get, &instance_path(name)).await?;
        let instance: IncusInstance =
            self.sync_metadata(response, &format!("get instance `{name}`"))?;
        Ok(instance.into_summary())
    }

    async fn create(&self, spec: InstanceSpec) -> RuntimeResult<InstanceSummary> {
        spec.validate()
            .map_err(|err| RuntimeError::Invalid(err.to_string()))?;
        let body = self.create_body(&spec);
        let response = self
            .send_json(HttpMethod::Post, "/1.0/instances", &body)
            .await?;
        self.finish_operation_response(response, &format!("create instance `{}`", spec.name))
            .await?;
        self.get(&spec.name).await
    }

    async fn start(&self, name: &str) -> RuntimeResult<()> {
        self.state_action(name, "start", false).await
    }

    async fn stop(&self, name: &str) -> RuntimeResult<()> {
        self.state_action(name, "stop", true).await
    }

    async fn restart(&self, name: &str) -> RuntimeResult<()> {
        self.state_action(name, "restart", true).await
    }

    async fn delete(&self, name: &str) -> RuntimeResult<()> {
        validate_instance_name(name)?;
        let response = self.send(HttpMethod::Delete, &instance_path(name)).await?;
        self.finish_operation_response(response, &format!("delete instance `{name}`"))
            .await
    }

    async fn update_limits(&self, name: &str, limits: ResourceLimits) -> RuntimeResult<()> {
        validate_instance_name(name)?;
        limits
            .validate()
            .map_err(|err| RuntimeError::Invalid(err.to_string()))?;
        let mut config = serde_json::Map::new();
        config.insert(
            "limits.cpu".to_owned(),
            json!(limits.cpu_limit.map_or_else(String::new, format_cpu)),
        );
        config.insert(
            "limits.memory".to_owned(),
            json!(limits
                .memory_mb
                .map_or_else(String::new, |memory_mb| format!("{memory_mb}MiB"))),
        );
        let body = json!({ "config": serde_json::Value::Object(config) });
        let response = self
            .send_json(HttpMethod::Patch, &instance_path(name), &body)
            .await?;
        self.finish_operation_response(response, &format!("update limits of instance `{name}`"))
            .await
    }

    async fn list_snapshots(&self, name: &str) -> RuntimeResult<Vec<SnapshotSummary>> {
        validate_instance_name(name)?;
        let path = format!("{}/snapshots?recursion=1", instance_path(name));
        let response = self.send(HttpMethod::Get, &path).await?;
        let snapshots: Vec<IncusSnapshot> =
            self.sync_metadata(response, &format!("list snapshots of instance `{name}`"))?;
        Ok(snapshots
            .into_iter()
            .map(IncusSnapshot::into_summary)
            .collect())
    }

    async fn snapshot(&self, name: &str, snapshot: &str) -> RuntimeResult<SnapshotSummary> {
        validate_instance_name(name)?;
        validate_snapshot_name(snapshot)?;
        let body = json!({ "name": snapshot, "stateful": false });
        let path = format!("{}/snapshots", instance_path(name));
        let response = self.send_json(HttpMethod::Post, &path, &body).await?;
        self.finish_operation_response(
            response,
            &format!("create snapshot `{snapshot}` of instance `{name}`"),
        )
        .await?;
        Ok(SnapshotSummary {
            name: snapshot.to_owned(),
            created_at: Some(chrono::Utc::now()),
            size_bytes: 0,
        })
    }

    async fn restore(&self, name: &str, snapshot: &str) -> RuntimeResult<()> {
        validate_instance_name(name)?;
        validate_snapshot_name(snapshot)?;
        let body = json!({ "restore": true });
        let path = snapshot_path(name, snapshot);
        let response = self.send_json(HttpMethod::Post, &path, &body).await?;
        self.finish_operation_response(
            response,
            &format!("restore instance `{name}` from snapshot `{snapshot}`"),
        )
        .await
    }

    async fn delete_snapshot(&self, name: &str, snapshot: &str) -> RuntimeResult<()> {
        validate_instance_name(name)?;
        validate_snapshot_name(snapshot)?;
        let path = snapshot_path(name, snapshot);
        let response = self.send(HttpMethod::Delete, &path).await?;
        self.finish_operation_response(
            response,
            &format!("delete snapshot `{snapshot}` of instance `{name}`"),
        )
        .await
    }

    async fn logs(&self, name: &str, lines: usize) -> RuntimeResult<String> {
        validate_instance_name(name)?;
        let path = format!("{}/logs", instance_path(name));
        let response = self.send(HttpMethod::Get, &path).await?;
        if !response.is_success() {
            // The logs endpoint is optional in some Incus builds; keep the UI
            // resilient instead of failing the whole instance view.
            tracing::debug!(
                instance = name,
                status = response.status,
                "Incus logs endpoint unavailable, returning fallback message"
            );
            return Ok(fallback_logs_message(name));
        }
        Ok(tail_lines(&response.text(), lines))
    }

    #[cfg(unix)]
    async fn exec(&self, name: &str, command: &[String]) -> RuntimeResult<ExecOutput> {
        validate_instance_name(name)?;
        if command.is_empty() {
            return Err(RuntimeError::Invalid(
                "exec requires at least one command argument".to_owned(),
            ));
        }
        let mut process = tokio::process::Command::new("incus");
        process.arg("exec").arg(name).arg("--").args(command);
        let output = process.output().await.map_err(|err| {
            RuntimeError::Unavailable(format!("cannot run `incus exec` on this host: {err}"))
        })?;
        Ok(ExecOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code(),
        })
    }

    #[cfg(not(unix))]
    async fn exec(&self, _name: &str, _command: &[String]) -> RuntimeResult<ExecOutput> {
        Err(RuntimeError::Unavailable(
            "exec requires a Unix host with the Incus CLI installed".to_owned(),
        ))
    }

    async fn deploy_oci(&self, spec: OciDeploySpec) -> RuntimeResult<InstanceSummary> {
        spec.validate()?;
        let (server, alias) = parse_oci_reference(&spec.image);
        let body = json!({
            "name": spec.name,
            "type": "container",
            "source": {
                "type": "image",
                "protocol": "oci",
                "server": server,
                "alias": alias,
                "mode": "pull",
            },
            "profiles": ["default"],
            "config": self.instance_config(&spec.env, &spec.limits),
            "devices": self.segment_devices(Some(spec.segment)),
            "start": true,
        });
        let response = self
            .send_json(HttpMethod::Post, "/1.0/instances", &body)
            .await?;
        self.finish_operation_response(
            response,
            &format!("deploy OCI application `{}`", spec.name),
        )
        .await?;
        self.get(&spec.name).await
    }
}

/// Validate an instance name using the shared core rules.
fn validate_instance_name(name: &str) -> RuntimeResult<()> {
    hs_core::validation::instance_name(name).map_err(|err| RuntimeError::Invalid(err.to_string()))
}

/// Validate a snapshot name: lowercase letters, digits, `-`, `_` and `.`.
fn validate_snapshot_name(snapshot: &str) -> RuntimeResult<()> {
    let valid = !snapshot.is_empty()
        && snapshot.len() <= 63
        && snapshot
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(RuntimeError::Invalid(format!(
            "invalid snapshot name `{snapshot}`: use lowercase letters, digits, `-`, `_` or `.`"
        )))
    }
}

/// Percent-encode a path segment (instance and snapshot names travel in URLs).
fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(*byte));
        } else {
            // Writing to a String cannot fail.
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// Path of a single instance resource.
fn instance_path(name: &str) -> String {
    format!("/1.0/instances/{}", encode_path_segment(name))
}

/// Path of a single instance snapshot resource.
fn snapshot_path(name: &str, snapshot: &str) -> String {
    format!(
        "{}/snapshots/{}",
        instance_path(name),
        encode_path_segment(snapshot)
    )
}

/// Canonical message returned when Incus cannot provide instance logs.
fn fallback_logs_message(name: &str) -> String {
    format!("logs are not available for instance `{name}` on this host")
}

/// Format a CPU limit the way Incus expects it (`2`, `0.5`, ...).
fn format_cpu(cpu: f32) -> String {
    if (cpu.fract()).abs() < f32::EPSILON {
        format!("{cpu:.0}")
    } else {
        format!("{cpu}")
    }
}

/// Map an HTTP status code to a [`RuntimeError`].
fn http_error(status: u16, detail: &str) -> RuntimeError {
    match status {
        404 => RuntimeError::NotFound(detail.to_owned()),
        409 => RuntimeError::Conflict(detail.to_owned()),
        400 | 412 | 422 => RuntimeError::Invalid(detail.to_owned()),
        401 | 403 => {
            RuntimeError::Unavailable(format!("not authorized to use the Incus socket: {detail}"))
        }
        408 | 504 => RuntimeError::Timeout(detail.to_owned()),
        _ => RuntimeError::Backend(format!("Incus returned HTTP {status}: {detail}")),
    }
}

/// Best-effort error description from an Incus error response.
fn error_detail(response: &HttpResponse) -> String {
    if let Ok(envelope) = Envelope::<serde_json::Value>::parse(&response.body) {
        if !envelope.error.is_empty() {
            return envelope.error;
        }
    }
    let text = response.text();
    let text = text.trim();
    if text.is_empty() {
        format!("HTTP status {}", response.status)
    } else {
        text.chars().take(300).collect()
    }
}

/// Split a full OCI image reference into `(server, alias)` as Incus expects.
///
/// Plain Docker Hub names map to `docker.io`, while registries with a dot or a
/// port in the first path segment are used verbatim:
/// - `ghcr.io/home-assistant/home-assistant:stable` ->
///   `("ghcr.io", "home-assistant/home-assistant:stable")`
/// - `nextcloud:stable` -> `("docker.io", "nextcloud:stable")`
/// - `adguard/adguardhome:latest` -> `("docker.io", "adguard/adguardhome:latest")`
fn parse_oci_reference(image: &str) -> (String, String) {
    let image = image.trim();
    match image.split_once('/') {
        Some((first, rest))
            if first.contains('.')
                || first.contains(':')
                || first.eq_ignore_ascii_case("localhost") =>
        {
            (first.to_owned(), rest.to_owned())
        }
        _ => ("docker.io".to_owned(), image.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::Mutex;

    use super::*;
    use crate::RuntimeStatus;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedRequest {
        method: HttpMethod,
        path: String,
        body: Option<String>,
    }

    #[derive(Debug, Clone)]
    struct Scripted {
        status: u16,
        body: String,
    }

    /// Scripted transport: every request pops the next scripted response.
    struct FakeTransport {
        available: bool,
        responses: Mutex<VecDeque<Scripted>>,
        requests: Mutex<Vec<RecordedRequest>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<(u16, String)>) -> Self {
            let script = responses
                .into_iter()
                .map(|(status, body)| Scripted { status, body })
                .collect();
            Self {
                available: true,
                responses: Mutex::new(script),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<RecordedRequest> {
            self.requests
                .lock()
                .expect("request log is not poisoned")
                .clone()
        }
    }

    #[async_trait]
    impl IncusTransport for FakeTransport {
        fn available(&self) -> bool {
            self.available
        }

        async fn request(
            &self,
            method: HttpMethod,
            path: &str,
            body: Option<Vec<u8>>,
        ) -> RuntimeResult<HttpResponse> {
            self.requests
                .lock()
                .expect("request log is not poisoned")
                .push(RecordedRequest {
                    method,
                    path: path.to_owned(),
                    body: body.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
                });
            let mut responses = self
                .responses
                .lock()
                .expect("response script is not poisoned");
            let Some(script) = responses.pop_front() else {
                return Err(RuntimeError::Backend(format!(
                    "unexpected request {method:?} {path}"
                )));
            };
            Ok(HttpResponse {
                status: script.status,
                body: script.body.into_bytes(),
            })
        }
    }

    fn sync_metadata(metadata: serde_json::Value) -> (u16, String) {
        (
            200,
            json!({
                "type": "sync",
                "status": "Success",
                "status_code": 200,
                "error": "",
                "error_code": 0,
                "metadata": metadata,
            })
            .to_string(),
        )
    }

    fn async_operation(operation_id: &str) -> (u16, String) {
        (
            200,
            json!({
                "type": "async",
                "status": "Operation created",
                "status_code": 100,
                "error": "",
                "error_code": 0,
                "metadata": {
                    "id": operation_id,
                    "status": "Running",
                    "status_code": 100,
                },
            })
            .to_string(),
        )
    }

    fn operation_success(operation_id: &str) -> (u16, String) {
        sync_metadata(json!({
            "id": operation_id,
            "status": "Success",
            "status_code": 200,
        }))
    }

    fn operation_running(operation_id: &str) -> (u16, String) {
        sync_metadata(json!({
            "id": operation_id,
            "status": "Running",
            "status_code": 100,
        }))
    }

    fn error_response(status: u16, message: &str) -> (u16, String) {
        (
            status,
            json!({
                "type": "error",
                "status": "Failure",
                "status_code": status,
                "error": message,
                "error_code": status,
            })
            .to_string(),
        )
    }

    fn instance_json(name: &str, status: &str) -> serde_json::Value {
        json!({
            "name": name,
            "status": status,
            "type": "container",
            "config": {
                "limits.cpu": "2",
                "limits.memory": "1024MiB",
                "image.description": "Debian 13"
            }
        })
    }

    fn provider_with_script(
        responses: Vec<(u16, String)>,
        timeout: Duration,
    ) -> (IncusProvider, Arc<FakeTransport>) {
        let transport = Arc::new(FakeTransport::new(responses));
        let provider = IncusProvider::with_transport("/run/incus.sock", transport.clone())
            .with_operation_timeout(timeout);
        (provider, transport)
    }

    fn provider_with_script_default(
        responses: Vec<(u16, String)>,
    ) -> (IncusProvider, Arc<FakeTransport>) {
        provider_with_script(responses, Duration::from_secs(5))
    }

    fn sample_spec() -> InstanceSpec {
        let mut env = BTreeMap::new();
        env.insert("TZ".to_owned(), "Europe/Madrid".to_owned());
        InstanceSpec {
            name: "demo".to_owned(),
            kind: RuntimeKind::Container,
            image: "images:debian/13".to_owned(),
            segment: Some(NetworkSegment::Lan),
            limits: ResourceLimits {
                cpu_limit: Some(2.0),
                memory_mb: Some(1024),
            },
            env,
            start: true,
        }
    }

    fn sample_oci_spec() -> OciDeploySpec {
        OciDeploySpec {
            name: "home-assistant".to_owned(),
            image: "ghcr.io/home-assistant/home-assistant:stable".to_owned(),
            segment: NetworkSegment::Dmz,
            env: BTreeMap::from([("TZ".to_owned(), "UTC".to_owned())]),
            limits: ResourceLimits::default(),
        }
    }

    #[test]
    fn path_encoding_escapes_reserved_characters() {
        assert_eq!(encode_path_segment("plain-name_1.2~"), "plain-name_1.2~");
        assert_eq!(encode_path_segment("Café Name/x"), "Caf%C3%A9%20Name%2Fx");
        assert_eq!(encode_path_segment("a?b#c"), "a%3Fb%23c");
        assert_eq!(instance_path("demo"), "/1.0/instances/demo");
        assert_eq!(
            snapshot_path("demo", "snap 0"),
            "/1.0/instances/demo/snapshots/snap%200"
        );
    }

    #[tokio::test]
    async fn invalid_names_are_rejected_without_requests() {
        let (provider, transport) = provider_with_script_default(Vec::new());
        assert!(matches!(
            provider.get("Bad Name").await,
            Err(RuntimeError::Invalid(_))
        ));
        assert!(matches!(
            provider.start("UPPER").await,
            Err(RuntimeError::Invalid(_))
        ));
        assert!(matches!(
            provider.snapshot("demo", "Bad Snapshot").await,
            Err(RuntimeError::Invalid(_))
        ));
        assert!(transport.requests().is_empty());
    }

    #[tokio::test]
    async fn list_maps_incus_instances_to_summaries() {
        let (provider, transport) = provider_with_script_default(vec![sync_metadata(json!([
            instance_json("alpha", "Running"),
            instance_json("beta", "Stopped")
        ]))]);
        let instances = provider.list().await.expect("list instances");
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].name, "alpha");
        assert_eq!(instances[0].status, RuntimeStatus::Running);
        assert_eq!(instances[0].limits.cpu_limit, Some(2.0));
        assert_eq!(instances[0].limits.memory_mb, Some(1024));
        assert_eq!(instances[1].status, RuntimeStatus::Stopped);
        assert_eq!(
            transport.requests()[0].path,
            "/1.0/instances?recursion=1".to_owned()
        );
    }

    #[tokio::test]
    async fn get_maps_state_addresses() {
        let (provider, _transport) = provider_with_script_default(vec![sync_metadata(json!({
            "name": "alpha",
            "status": "Running",
            "type": "container",
            "config": {},
            "state": {
                "status": "Running",
                "network": {
                    "eth0": {"addresses": [
                        {"family": "inet", "address": "10.0.0.7", "scope": "global"},
                        {"family": "inet", "address": "127.0.0.1", "scope": "local"}
                    ]}
                }
            }
        }))]);
        let instance = provider.get("alpha").await.expect("get instance");
        assert_eq!(instance.ipv4_addresses, vec!["10.0.0.7".to_owned()]);
        assert_eq!(instance.status, RuntimeStatus::Running);
    }

    #[tokio::test]
    async fn get_missing_instance_maps_to_not_found() {
        let (provider, _transport) =
            provider_with_script_default(vec![error_response(404, "Instance not found")]);
        match provider.get("ghost").await {
            Err(RuntimeError::NotFound(detail)) => assert!(detail.contains("Instance not found")),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_sends_expected_body_and_waits_for_operation() {
        let (provider, transport) = provider_with_script_default(vec![
            async_operation("op-create"),
            operation_success("op-create"),
            sync_metadata(instance_json("demo", "Running")),
        ]);
        let summary = provider
            .create(sample_spec())
            .await
            .expect("create instance");
        assert_eq!(summary.name, "demo");
        assert_eq!(summary.status, RuntimeStatus::Running);

        let requests = transport.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].method, HttpMethod::Post);
        assert_eq!(requests[0].path, "/1.0/instances");
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_ref().expect("create body")).unwrap();
        assert_eq!(body["name"], "demo");
        assert_eq!(body["type"], "container");
        assert_eq!(body["source"]["alias"], "images:debian/13");
        assert_eq!(body["config"]["limits.cpu"], "2");
        assert_eq!(body["config"]["limits.memory"], "1024MiB");
        assert_eq!(body["config"]["environment.TZ"], "Europe/Madrid");
        assert_eq!(body["devices"]["eth0"]["parent"], DEFAULT_LAN_BRIDGE);
        assert_eq!(body["start"], true);
        assert!(
            requests[1]
                .path
                .starts_with("/1.0/operations/op-create/wait?timeout="),
            "unexpected wait path: {}",
            requests[1].path
        );
        assert_eq!(requests[2].path, "/1.0/instances/demo");
    }

    #[tokio::test]
    async fn create_on_dmz_uses_dmz_bridge() {
        let mut spec = sample_spec();
        spec.segment = Some(NetworkSegment::Dmz);
        let (provider, transport) = provider_with_script_default(vec![
            async_operation("op-1"),
            operation_success("op-1"),
            sync_metadata(instance_json("demo", "Running")),
        ]);
        provider.create(spec).await.expect("create instance");
        let body: serde_json::Value =
            serde_json::from_str(transport.requests()[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(body["devices"]["eth0"]["parent"], DEFAULT_DMZ_BRIDGE);
    }

    #[tokio::test]
    async fn start_and_stop_post_state_actions() {
        let (provider, transport) = provider_with_script_default(vec![
            async_operation("op-start"),
            operation_success("op-start"),
            async_operation("op-stop"),
            operation_success("op-stop"),
        ]);
        provider.start("demo").await.expect("start instance");
        provider.stop("demo").await.expect("stop instance");
        let requests = transport.requests();
        let start_body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(requests[0].path, "/1.0/instances/demo/state");
        assert_eq!(start_body["action"], "start");
        let stop_body: serde_json::Value =
            serde_json::from_str(requests[2].body.as_ref().unwrap()).unwrap();
        assert_eq!(stop_body["action"], "stop");
        assert_eq!(stop_body["force"], true);
    }

    #[tokio::test]
    async fn delete_waits_for_operation_and_maps_not_found() {
        let (provider, transport) = provider_with_script_default(vec![
            async_operation("op-del"),
            operation_success("op-del"),
        ]);
        provider.delete("demo").await.expect("delete instance");
        assert_eq!(transport.requests()[0].method, HttpMethod::Delete);
        assert_eq!(transport.requests()[0].path, "/1.0/instances/demo");

        let (provider, _transport) =
            provider_with_script_default(vec![error_response(404, "Instance not found")]);
        assert!(matches!(
            provider.delete("ghost").await,
            Err(RuntimeError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn update_limits_patches_config() {
        let (provider, transport) = provider_with_script_default(vec![
            async_operation("op-limits"),
            operation_success("op-limits"),
        ]);
        provider
            .update_limits(
                "demo",
                ResourceLimits {
                    cpu_limit: Some(0.5),
                    memory_mb: Some(2048),
                },
            )
            .await
            .expect("update limits");
        let request = &transport.requests()[0];
        assert_eq!(request.method, HttpMethod::Patch);
        let body: serde_json::Value = serde_json::from_str(request.body.as_ref().unwrap()).unwrap();
        assert_eq!(body["config"]["limits.cpu"], "0.5");
        assert_eq!(body["config"]["limits.memory"], "2048MiB");
    }

    #[tokio::test]
    async fn snapshot_lifecycle_endpoints() {
        let (provider, transport) = provider_with_script_default(vec![
            async_operation("op-snap"),
            operation_success("op-snap"),
            sync_metadata(json!([{
                "name": "snap0",
                "created_at": "2026-01-01T00:00:00Z",
                "size": 4096
            }])),
            async_operation("op-restore"),
            operation_success("op-restore"),
            async_operation("op-del-snap"),
            operation_success("op-del-snap"),
        ]);
        let created = provider.snapshot("demo", "snap0").await.expect("snapshot");
        assert_eq!(created.name, "snap0");
        let snapshots = provider
            .list_snapshots("demo")
            .await
            .expect("list snapshots");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].size_bytes, 4096);
        provider.restore("demo", "snap0").await.expect("restore");
        provider
            .delete_snapshot("demo", "snap0")
            .await
            .expect("delete snapshot");
        let paths: Vec<_> = transport
            .requests()
            .into_iter()
            .map(|request| request.path)
            .collect();
        assert!(paths.contains(&"/1.0/instances/demo/snapshots".to_owned()));
        assert!(paths.contains(&"/1.0/instances/demo/snapshots/snap0".to_owned()));
    }

    #[tokio::test]
    async fn operation_timeout_is_reported() {
        let (provider, _transport) = provider_with_script(
            vec![
                async_operation("op-slow"),
                operation_running("op-slow"),
                operation_running("op-slow"),
                operation_running("op-slow"),
                operation_running("op-slow"),
                operation_running("op-slow"),
            ],
            Duration::from_millis(30),
        );
        match provider.start("demo").await {
            Err(RuntimeError::Timeout(_)) => {}
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn operation_failure_is_reported_as_backend_error() {
        let (provider, _transport) = provider_with_script_default(vec![
            async_operation("op-bad"),
            error_response(500, "image pull failed"),
        ]);
        match provider.start("demo").await {
            Err(RuntimeError::Backend(detail)) => assert!(detail.contains("image pull failed")),
            other => panic!("expected Backend, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn logs_fall_back_when_endpoint_is_unavailable() {
        let (provider, _transport) = provider_with_script_default(vec![
            error_response(404, "not available"),
            sync_metadata(serde_json::Value::Null),
        ]);
        let fallback = provider.logs("demo", 10).await.expect("fallback logs");
        assert!(fallback.contains("demo"));
        let text = "line1\nline2\nline3\n";
        let (provider, _transport) = provider_with_script_default(vec![(200, text.to_owned())]);
        assert_eq!(provider.logs("demo", 2).await.unwrap(), "line2\nline3");
    }

    #[tokio::test]
    async fn deploy_oci_builds_oci_image_source() {
        let (provider, transport) = provider_with_script_default(vec![
            async_operation("op-oci"),
            operation_success("op-oci"),
            sync_metadata(json!({
                "name": "home-assistant",
                "status": "Running",
                "type": "container",
                "config": {"image.description": "Home Assistant"}
            })),
        ]);
        let summary = provider
            .deploy_oci(sample_oci_spec())
            .await
            .expect("deploy OCI app");
        assert_eq!(summary.name, "home-assistant");
        assert_eq!(summary.status, RuntimeStatus::Running);
        let request = &transport.requests()[0];
        let body: serde_json::Value = serde_json::from_str(request.body.as_ref().unwrap()).unwrap();
        assert_eq!(body["source"]["type"], "image");
        assert_eq!(body["source"]["protocol"], "oci");
        assert_eq!(body["source"]["server"], "ghcr.io");
        assert_eq!(
            body["source"]["alias"],
            "home-assistant/home-assistant:stable"
        );
        assert_eq!(body["source"]["mode"], "pull");
        assert_eq!(body["start"], true);
        assert_eq!(body["devices"]["eth0"]["parent"], DEFAULT_DMZ_BRIDGE);
    }

    #[test]
    fn oci_reference_parsing() {
        assert_eq!(
            parse_oci_reference("ghcr.io/immich-app/immich-server:release"),
            (
                "ghcr.io".to_owned(),
                "immich-app/immich-server:release".to_owned()
            )
        );
        assert_eq!(
            parse_oci_reference("nextcloud:stable"),
            ("docker.io".to_owned(), "nextcloud:stable".to_owned())
        );
        assert_eq!(
            parse_oci_reference("adguard/adguardhome:latest"),
            (
                "docker.io".to_owned(),
                "adguard/adguardhome:latest".to_owned()
            )
        );
        assert_eq!(
            parse_oci_reference("localhost:5000/app:tag"),
            ("localhost:5000".to_owned(), "app:tag".to_owned())
        );
    }

    #[test]
    fn http_errors_map_to_runtime_errors() {
        assert!(matches!(http_error(404, "x"), RuntimeError::NotFound(_)));
        assert!(matches!(http_error(409, "x"), RuntimeError::Conflict(_)));
        assert!(matches!(http_error(400, "x"), RuntimeError::Invalid(_)));
        assert!(matches!(http_error(412, "x"), RuntimeError::Invalid(_)));
        assert!(matches!(http_error(401, "x"), RuntimeError::Unavailable(_)));
        assert!(matches!(http_error(403, "x"), RuntimeError::Unavailable(_)));
        assert!(matches!(http_error(504, "x"), RuntimeError::Timeout(_)));
        assert!(matches!(http_error(500, "x"), RuntimeError::Backend(_)));
    }

    #[test]
    fn cpu_formatting_matches_incus_expectations() {
        assert_eq!(format_cpu(2.0), "2");
        assert_eq!(format_cpu(0.5), "0.5");
        assert_eq!(format_cpu(1.25), "1.25");
    }

    #[tokio::test]
    async fn check_requires_available_transport() {
        let provider = IncusProvider::new("/nonexistent/hs-test/incus.sock");
        assert!(!provider.available());
        match provider.check().await {
            Err(RuntimeError::Unavailable(_)) => {}
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn default_provider_is_unavailable_on_non_unix() {
        let provider = IncusProvider::new("/nonexistent/hs-test/incus.sock");
        assert!(!provider.available());
        if cfg!(unix) {
            return;
        }
        assert!(matches!(
            provider.list().await,
            Err(RuntimeError::Unavailable(_))
        ));
        assert!(matches!(
            provider.start("demo").await,
            Err(RuntimeError::Unavailable(_))
        ));
        assert!(matches!(
            provider.deploy_oci(sample_oci_spec()).await,
            Err(RuntimeError::Unavailable(_))
        ));
        assert!(matches!(
            provider.exec("demo", &["true".to_owned()]).await,
            Err(RuntimeError::Unavailable(_))
        ));
    }

    #[tokio::test]
    async fn check_decodes_health_envelope() {
        let (provider, _transport) = provider_with_script_default(vec![sync_metadata(json!({}))]);
        provider.check().await.expect("healthy Incus");
        let (provider, _transport) =
            provider_with_script_default(vec![error_response(500, "daemon offline")]);
        assert!(provider.check().await.is_err());
    }
}
