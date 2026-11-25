//! Serde models and mapping helpers for the Incus REST API.
//!
//! Incus answers with a JSON envelope of the shape
//! `{"type":"sync|async","status":"Success","status_code":200,
//!   "error":"","error_code":0,"metadata": ... }`.
//! The types in this module intentionally model only the fields the adapter
//! needs; unknown fields are ignored so that minor Incus upgrades do not break
//! the appliance.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::{
    InstanceSummary, ResourceLimits, RuntimeError, RuntimeKind, RuntimeStatus, SnapshotSummary,
};

/// Incus response envelope.
#[derive(Debug, Deserialize)]
pub(crate) struct Envelope<T> {
    /// `sync` for immediate responses, `async` for background operations.
    #[serde(default, rename = "type")]
    pub(crate) kind: String,
    /// Status name (`Success`, `Operation created`, `Failure`, ...).
    #[serde(default)]
    pub(crate) status: String,
    /// HTTP-like status code mirrored in the JSON payload.
    #[serde(default)]
    pub(crate) status_code: u16,
    /// Human readable error message (empty on success).
    #[serde(default)]
    pub(crate) error: String,
    /// Incus error code (0 on success).
    #[serde(default)]
    pub(crate) error_code: u64,
    /// Response payload.
    pub(crate) metadata: Option<T>,
}

impl<T: DeserializeOwned> Envelope<T> {
    /// Parse an envelope from a raw response body.
    pub(crate) fn parse(body: &[u8]) -> Result<Self, RuntimeError> {
        serde_json::from_slice(body).map_err(|err| {
            RuntimeError::Backend(format!("malformed Incus response payload: {err}"))
        })
    }

    /// Whether the envelope reports a failure.
    pub(crate) fn is_failure(&self) -> bool {
        self.error_code != 0 || self.status.eq_ignore_ascii_case("failure")
    }

    /// Error message reported by the envelope.
    pub(crate) fn error_message(&self) -> String {
        if self.error.is_empty() {
            format!(
                "Incus reported status `{}` ({})",
                self.status, self.status_code
            )
        } else {
            self.error.clone()
        }
    }

    /// Consume the envelope and return the metadata payload.
    pub(crate) fn into_metadata(self) -> Result<T, RuntimeError> {
        self.metadata
            .ok_or_else(|| RuntimeError::Backend("Incus response has no metadata".to_owned()))
    }
}

/// Background operation returned by asynchronous Incus calls.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Operation {
    /// Operation UUID.
    #[serde(default)]
    pub(crate) id: String,
    /// Operation status (`Running`, `Success`, `Failure`).
    #[serde(default)]
    pub(crate) status: String,
    /// Incus status code.
    #[serde(default)]
    pub(crate) status_code: u16,
    /// Error message on failure.
    #[serde(default)]
    pub(crate) error: String,
    /// Incus error code (0 on success).
    #[serde(default)]
    pub(crate) error_code: u64,
}

impl Operation {
    /// Whether the operation finished successfully.
    pub(crate) fn is_success(&self) -> bool {
        self.status.eq_ignore_ascii_case("success") && self.error_code == 0
    }

    /// Whether the operation finished with an error.
    pub(crate) fn is_failure(&self) -> bool {
        self.error_code != 0 || self.status.eq_ignore_ascii_case("failure")
    }

    /// Human readable error description.
    pub(crate) fn error_message(&self) -> String {
        if self.error.is_empty() {
            format!(
                "Incus operation ended with status `{}` ({})",
                self.status, self.status_code
            )
        } else {
            self.error.clone()
        }
    }
}

/// Instance object returned by `/1.0/instances`.
#[derive(Debug, Deserialize)]
pub(crate) struct IncusInstance {
    /// Instance name.
    #[serde(default)]
    pub(crate) name: String,
    /// Instance status (`Running`, `Stopped`, ...).
    #[serde(default)]
    pub(crate) status: String,
    /// Instance kind: `container` or `virtual-machine`.
    #[serde(default, rename = "type")]
    pub(crate) kind: String,
    /// Instance configuration keys (`limits.cpu`, `environment.*`, ...).
    #[serde(default)]
    pub(crate) config: BTreeMap<String, String>,
    /// Runtime state; present in single-instance responses.
    #[serde(default)]
    pub(crate) state: Option<InstanceState>,
    /// Creation timestamp.
    #[serde(default)]
    pub(crate) created_at: Option<DateTime<Utc>>,
    /// Inline snapshots when the query asked for them.
    #[serde(default)]
    pub(crate) snapshots: Option<Vec<IncusSnapshot>>,
}

/// Runtime state block of an instance.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct InstanceState {
    /// Status reported by the state block (authoritative when present).
    #[serde(default)]
    pub(crate) status: String,
    /// Per-interface network state.
    #[serde(default)]
    pub(crate) network: BTreeMap<String, NetworkState>,
}

/// Network state of a single interface.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct NetworkState {
    /// Addresses assigned to the interface.
    #[serde(default)]
    pub(crate) addresses: Vec<InstanceStateAddress>,
}

/// Address entry of an instance interface.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct InstanceStateAddress {
    /// `inet` or `inet6`.
    #[serde(default)]
    pub(crate) family: String,
    /// Address in presentation form.
    #[serde(default)]
    pub(crate) address: String,
    /// Address scope (`global`, `link`, ...).
    #[serde(default)]
    pub(crate) scope: String,
}

/// Snapshot object returned by `/1.0/instances/{name}/snapshots`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct IncusSnapshot {
    /// Snapshot name.
    #[serde(default)]
    pub(crate) name: String,
    /// Creation timestamp.
    #[serde(default)]
    pub(crate) created_at: Option<DateTime<Utc>>,
    /// Size in bytes when reported.
    #[serde(default)]
    pub(crate) size: u64,
}

/// Map an Incus status string to the normalized [`RuntimeStatus`].
pub(crate) fn map_status(raw: &str) -> RuntimeStatus {
    match raw.trim().to_ascii_lowercase().as_str() {
        "running" => RuntimeStatus::Running,
        "stopped" => RuntimeStatus::Stopped,
        "starting" => RuntimeStatus::Starting,
        "stopping" => RuntimeStatus::Stopping,
        "frozen" | "freezing" => RuntimeStatus::Frozen,
        "error" => RuntimeStatus::Error,
        _ => RuntimeStatus::Unknown,
    }
}

/// Map an Incus instance type string to the normalized [`RuntimeKind`].
pub(crate) fn map_kind(raw: &str) -> RuntimeKind {
    if raw.trim().eq_ignore_ascii_case("virtual-machine") || raw.trim().eq_ignore_ascii_case("vm") {
        RuntimeKind::Vm
    } else {
        RuntimeKind::Container
    }
}

/// Parse an Incus memory limit (`512MiB`, `1GiB`, `1024MB`, raw bytes).
///
/// Incus accepts binary suffixes (`KiB`, `MiB`, `GiB`, `TiB`) and raw byte
/// counts. Decimal suffixes are interpreted as binary multiples as well, which
/// matches what users expect from a memory limit and what Incus itself
/// accepts for these values.
pub(crate) fn parse_memory_mb(value: &str) -> Option<u64> {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    let (number, unit_bytes) = if let Some(rest) = lower.strip_suffix("kib") {
        (rest, KIB)
    } else if let Some(rest) = lower.strip_suffix("mib") {
        (rest, MIB)
    } else if let Some(rest) = lower.strip_suffix("gib") {
        (rest, GIB)
    } else if let Some(rest) = lower.strip_suffix("tib") {
        (rest, TIB)
    } else if let Some(rest) = lower.strip_suffix("kb") {
        (rest, KIB)
    } else if let Some(rest) = lower.strip_suffix("mb") {
        (rest, MIB)
    } else if let Some(rest) = lower.strip_suffix("gb") {
        (rest, GIB)
    } else if let Some(rest) = lower.strip_suffix("tb") {
        (rest, TIB)
    } else if let Some(rest) = lower.strip_suffix("b") {
        (rest, 1)
    } else {
        (lower.as_str(), 1)
    };
    let number: f64 = number.trim().parse().ok()?;
    if !number.is_finite() || number < 0.0 {
        return None;
    }
    let bytes = number * unit_bytes as f64;
    if bytes == 0.0 {
        return None;
    }
    Some((bytes / MIB as f64) as u64)
}

/// Parse a CPU limit such as `2`, `0.5` or `150%` (Incus percentages).
pub(crate) fn parse_cpu_limit(value: &str) -> Option<f32> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(percent) = value.strip_suffix('%') {
        let parsed: f32 = percent.trim().parse().ok()?;
        if parsed > 0.0 && parsed.is_finite() {
            return Some(parsed / 100.0);
        }
        return None;
    }
    let parsed: f32 = value.parse().ok()?;
    if parsed > 0.0 && parsed.is_finite() {
        Some(parsed)
    } else {
        None
    }
}

/// Extract the image description stored in the instance configuration.
fn image_from_config(config: &BTreeMap<String, String>) -> String {
    config
        .get("image.description")
        .or_else(|| config.get("volatile.base_image"))
        .cloned()
        .unwrap_or_default()
}

/// Collect global IPv4 addresses from an instance state block.
fn ipv4_addresses(state: Option<&InstanceState>) -> Vec<String> {
    let mut addresses = Vec::new();
    if let Some(state) = state {
        for network in state.network.values() {
            for address in &network.addresses {
                if address.family == "inet" && address.scope == "global" {
                    addresses.push(address.address.clone());
                }
            }
        }
    }
    addresses.sort();
    addresses.dedup();
    addresses
}

impl IncusInstance {
    /// Build the normalized summary returned by the provider port.
    pub(crate) fn into_summary(self) -> InstanceSummary {
        let state = self.state.as_ref();
        let status = state
            .map(|state| state.status.as_str())
            .filter(|status| !status.is_empty())
            .unwrap_or(self.status.as_str());
        InstanceSummary {
            name: self.name,
            kind: map_kind(&self.kind),
            status: map_status(status),
            image: image_from_config(&self.config),
            ipv4_addresses: ipv4_addresses(state),
            limits: ResourceLimits {
                cpu_limit: self
                    .config
                    .get("limits.cpu")
                    .and_then(|value| parse_cpu_limit(value)),
                memory_mb: self
                    .config
                    .get("limits.memory")
                    .and_then(|value| parse_memory_mb(value)),
            },
            created_at: self.created_at,
            snapshot_count: self
                .snapshots
                .as_ref()
                .map_or(0, |snapshots| snapshots.len() as u32),
        }
    }
}

impl IncusSnapshot {
    /// Convert into the normalized snapshot summary.
    pub(crate) fn into_summary(self) -> SnapshotSummary {
        SnapshotSummary {
            name: self.name,
            created_at: self.created_at,
            size_bytes: self.size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_covers_incus_states() {
        assert_eq!(map_status("Running"), RuntimeStatus::Running);
        assert_eq!(map_status("running"), RuntimeStatus::Running);
        assert_eq!(map_status("Stopped"), RuntimeStatus::Stopped);
        assert_eq!(map_status("Starting"), RuntimeStatus::Starting);
        assert_eq!(map_status("Stopping"), RuntimeStatus::Stopping);
        assert_eq!(map_status("Frozen"), RuntimeStatus::Frozen);
        assert_eq!(map_status("Freezing"), RuntimeStatus::Frozen);
        assert_eq!(map_status("Error"), RuntimeStatus::Error);
        assert_eq!(map_status("Migrating"), RuntimeStatus::Unknown);
        assert_eq!(map_status(""), RuntimeStatus::Unknown);
    }

    #[test]
    fn kind_mapping() {
        assert_eq!(map_kind("container"), RuntimeKind::Container);
        assert_eq!(map_kind("virtual-machine"), RuntimeKind::Vm);
        assert_eq!(map_kind("VM"), RuntimeKind::Vm);
        assert_eq!(map_kind(""), RuntimeKind::Container);
    }

    #[test]
    fn memory_limit_parsing() {
        assert_eq!(parse_memory_mb("512MiB"), Some(512));
        assert_eq!(parse_memory_mb("1GiB"), Some(1024));
        assert_eq!(parse_memory_mb("2gib"), Some(2048));
        assert_eq!(parse_memory_mb("1024MB"), Some(1024));
        assert_eq!(parse_memory_mb("1048576"), Some(1));
        assert_eq!(parse_memory_mb("512KiB"), Some(0));
        assert_eq!(parse_memory_mb(""), None);
        assert_eq!(parse_memory_mb("0"), None);
        assert_eq!(parse_memory_mb("a lot"), None);
    }

    #[test]
    fn cpu_limit_parsing() {
        assert_eq!(parse_cpu_limit("2"), Some(2.0));
        assert_eq!(parse_cpu_limit("0.5"), Some(0.5));
        assert_eq!(parse_cpu_limit("150%"), Some(1.5));
        assert_eq!(parse_cpu_limit("0"), None);
        assert_eq!(parse_cpu_limit(""), None);
        assert_eq!(parse_cpu_limit("1-2"), None);
    }

    #[test]
    fn instance_json_maps_to_summary() {
        let json = serde_json::json!({
            "name": "home-assistant",
            "status": "Running",
            "type": "container",
            "config": {
                "limits.cpu": "2",
                "limits.memory": "2GiB",
                "image.description": "Ubuntu 24.04 LTS"
            },
            "state": {
                "status": "Running",
                "network": {
                    "eth0": {
                        "addresses": [
                            {"family": "inet", "address": "10.0.0.42", "scope": "global"},
                            {"family": "inet6", "address": "fd00::42", "scope": "global"},
                            {"family": "inet", "address": "10.0.0.255", "scope": "link"}
                        ]
                    }
                }
            },
            "created_at": "2026-01-02T03:04:05Z",
            "snapshots": [{"name": "snap0", "created_at": "2026-01-03T00:00:00Z", "size": 4096}]
        });
        let instance: IncusInstance = serde_json::from_value(json).unwrap();
        let summary = instance.into_summary();
        assert_eq!(summary.name, "home-assistant");
        assert_eq!(summary.kind, RuntimeKind::Container);
        assert_eq!(summary.status, RuntimeStatus::Running);
        assert_eq!(summary.image, "Ubuntu 24.04 LTS");
        assert_eq!(summary.ipv4_addresses, vec!["10.0.0.42".to_owned()]);
        assert_eq!(summary.limits.cpu_limit, Some(2.0));
        assert_eq!(summary.limits.memory_mb, Some(2048));
        assert_eq!(summary.snapshot_count, 1);
        assert!(summary.created_at.is_some());
    }

    #[test]
    fn instance_list_entry_without_state_uses_status_field() {
        let json = serde_json::json!({
            "name": "vm1",
            "status": "Stopped",
            "type": "virtual-machine",
            "config": {}
        });
        let instance: IncusInstance = serde_json::from_value(json).unwrap();
        let summary = instance.into_summary();
        assert_eq!(summary.status, RuntimeStatus::Stopped);
        assert_eq!(summary.kind, RuntimeKind::Vm);
        assert!(summary.ipv4_addresses.is_empty());
        assert_eq!(summary.snapshot_count, 0);
    }

    #[test]
    fn envelope_parsing_and_error_detection() {
        let body = br#"{"type":"sync","status":"Success","status_code":200,"error":"","error_code":0,"metadata":[{"name":"a","status":"Running","type":"container","config":{}}]}"#;
        let envelope: Envelope<Vec<IncusInstance>> = Envelope::parse(body).unwrap();
        assert!(!envelope.is_failure());
        let instances = envelope.into_metadata().unwrap();
        assert_eq!(instances[0].name, "a");

        let failure = br#"{"type":"error","status":"Failure","status_code":404,"error":"Instance not found","error_code":404}"#;
        let envelope: Envelope<serde_json::Value> = Envelope::parse(failure).unwrap();
        assert!(envelope.is_failure());
        assert_eq!(envelope.error_message(), "Instance not found");

        let malformed = Envelope::<serde_json::Value>::parse(b"not json");
        assert!(matches!(malformed, Err(RuntimeError::Backend(_))));
    }

    #[test]
    fn operation_success_and_failure_detection() {
        let success: Operation = serde_json::from_value(serde_json::json!({
            "id": "op-1",
            "status": "Success",
            "status_code": 200
        }))
        .unwrap();
        assert!(success.is_success());
        assert!(!success.is_failure());

        let failure: Operation = serde_json::from_value(serde_json::json!({
            "id": "op-2",
            "status": "Failure",
            "status_code": 500,
            "error": "image pull failed",
            "error_code": 500
        }))
        .unwrap();
        assert!(failure.is_failure());
        assert_eq!(failure.error_message(), "image pull failed");
    }

    #[test]
    fn snapshot_conversion() {
        let snapshot: IncusSnapshot = serde_json::from_value(serde_json::json!({
            "name": "snap0",
            "created_at": "2026-02-03T04:05:06Z",
            "size": 1024
        }))
        .unwrap();
        let summary = snapshot.into_summary();
        assert_eq!(summary.name, "snap0");
        assert_eq!(summary.size_bytes, 1024);
        assert!(summary.created_at.is_some());
    }
}
