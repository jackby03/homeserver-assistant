//! Outbound ports implemented by `hs-daemon`.
//!
//! `hs-tools` defines the smallest set of async traits it needs to execute the
//! built-in tools. The daemon wires the concrete adapters (Incus, Home
//! Assistant, PipeWire, HTTP client) at the composition root, which keeps this
//! crate free of platform dependencies and easy to test with fakes.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use hs_core::models::InstanceSummary;

/// Read-only projection of a Home Assistant entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityLite {
    /// Entity id, e.g. `light.kitchen`.
    pub entity_id: String,
    /// Normalized state string, e.g. `on`.
    pub state: String,
    /// Friendly name shown to the user.
    pub name: String,
}

/// Manage the workload runtime (containers and virtual machines).
#[async_trait]
pub trait RuntimeControl: Send + Sync {
    /// List every workload managed by the runtime provider.
    ///
    /// # Errors
    /// Returns a human-readable message when the provider is unreachable.
    async fn list_instances(&self) -> Result<Vec<InstanceSummary>, String>;

    /// Start a stopped workload.
    ///
    /// # Errors
    /// Returns a human-readable message when the provider rejects the action.
    async fn start_instance(&self, name: &str) -> Result<(), String>;

    /// Stop a running workload.
    ///
    /// # Errors
    /// Returns a human-readable message when the provider rejects the action.
    async fn stop_instance(&self, name: &str) -> Result<(), String>;
}

/// Control the home automation hub (Home Assistant).
#[async_trait]
pub trait DomoticsControl: Send + Sync {
    /// List the entities known to the hub.
    ///
    /// # Errors
    /// Returns a human-readable message when the hub is unreachable.
    async fn list_entities(&self) -> Result<Vec<EntityLite>, String>;

    /// Call a Home Assistant service.
    ///
    /// # Errors
    /// Returns a human-readable message when the hub rejects the call.
    async fn call_service(
        &self,
        domain: &str,
        service: &str,
        data: serde_json::Value,
    ) -> Result<(), String>;
}

/// Control the household audio playback.
#[async_trait]
pub trait AudioControl: Send + Sync {
    /// Start or resume playback.
    ///
    /// # Errors
    /// Returns a human-readable message when the sink is unavailable.
    async fn play(&self) -> Result<(), String>;

    /// Pause playback.
    ///
    /// # Errors
    /// Returns a human-readable message when the sink is unavailable.
    async fn pause(&self) -> Result<(), String>;

    /// Set the output volume as a percentage (`0..=100`).
    ///
    /// # Errors
    /// Returns a human-readable message when the sink is unavailable.
    async fn set_volume(&self, percent: u8) -> Result<(), String>;
}

/// Read host-level health metrics.
#[async_trait]
pub trait SystemControl: Send + Sync {
    /// Return a structured overview of the host (CPU, memory, disks, sensors).
    ///
    /// # Errors
    /// Returns a human-readable message when metrics cannot be collected.
    async fn overview(&self) -> Result<serde_json::Value, String>;
}

/// Deliver outbound HTTP webhook calls.
#[async_trait]
pub trait WebhookClient: Send + Sync {
    /// POST `payload` to `url` and return the response status code.
    ///
    /// Adapters are responsible for enforcing the appliance egress policy
    /// (allowed destinations, timeouts and TLS validation).
    ///
    /// # Errors
    /// Returns a human-readable message when the request fails.
    async fn post(&self, url: &str, payload: serde_json::Value) -> Result<u16, String>;
}

/// One stored household memory item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Stable key chosen by the model (e.g. `cumpleanos_mama`).
    pub key: String,
    /// Value to remember.
    pub value: String,
}

/// Persistent per-profile memory used by the assistant.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Create or replace `key` for `profile_id`.
    ///
    /// # Errors
    /// Returns a human-readable message when persistence fails.
    async fn remember(
        &self,
        profile_id: &str,
        key: &str,
        value: &str,
    ) -> Result<MemoryEntry, String>;

    /// List memories, optionally filtered by a case-insensitive substring.
    ///
    /// # Errors
    /// Returns a human-readable message when persistence fails.
    async fn recall(
        &self,
        profile_id: &str,
        query: Option<&str>,
    ) -> Result<Vec<MemoryEntry>, String>;

    /// Remove a memory. Returns `true` when it existed.
    ///
    /// # Errors
    /// Returns a human-readable message when persistence fails.
    async fn forget(&self, profile_id: &str, key: &str) -> Result<bool, String>;
}

/// Small in-memory [`MemoryStore`] used by tests and ephemeral runs.
#[derive(Debug, Default)]
pub struct EphemeralMemoryStore {
    entries: std::sync::Mutex<Vec<(String, MemoryEntry)>>,
}

#[async_trait]
impl MemoryStore for EphemeralMemoryStore {
    async fn remember(
        &self,
        profile_id: &str,
        key: &str,
        value: &str,
    ) -> Result<MemoryEntry, String> {
        let entry = MemoryEntry {
            key: key.to_owned(),
            value: value.to_owned(),
        };
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "memory lock poisoned".to_owned())?;
        let scope = format!("{profile_id}:{key}");
        entries.retain(|(stored, _)| stored != &scope);
        entries.push((scope, entry.clone()));
        Ok(entry)
    }

    async fn recall(
        &self,
        profile_id: &str,
        query: Option<&str>,
    ) -> Result<Vec<MemoryEntry>, String> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| "memory lock poisoned".to_owned())?;
        let prefix = format!("{profile_id}:");
        let needle = query.map(str::to_lowercase);
        Ok(entries
            .iter()
            .filter(|(scope, _)| scope.starts_with(&prefix))
            .map(|(_, entry)| entry)
            .filter(|entry| {
                needle.as_ref().is_none_or(|needle| {
                    needle.is_empty()
                        || entry.key.to_lowercase().contains(needle)
                        || entry.value.to_lowercase().contains(needle)
                })
            })
            .cloned()
            .collect())
    }

    async fn forget(&self, profile_id: &str, key: &str) -> Result<bool, String> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "memory lock poisoned".to_owned())?;
        let scope = format!("{profile_id}:{key}");
        let before = entries.len();
        entries.retain(|(stored, _)| stored != &scope);
        Ok(entries.len() != before)
    }
}
/// Bundle of outbound adapters required to execute tools.
///
/// Cloning is cheap: every field is an `Arc`.
#[derive(Clone)]
pub struct ToolPorts {
    /// Workload runtime adapter.
    pub runtime: Arc<dyn RuntimeControl>,
    /// Home automation adapter.
    pub domotics: Arc<dyn DomoticsControl>,
    /// Audio playback adapter.
    pub audio: Arc<dyn AudioControl>,
    /// Host metrics adapter.
    pub system: Arc<dyn SystemControl>,
    /// Outbound webhook adapter.
    pub webhooks: Arc<dyn WebhookClient>,
    /// Per-profile assistant memory.
    pub memory: Arc<dyn MemoryStore>,
}

impl std::fmt::Debug for ToolPorts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ToolPorts").finish_non_exhaustive()
    }
}
