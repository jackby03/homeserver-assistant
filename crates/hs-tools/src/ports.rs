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
/// A pending reminder/timer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimerEntry {
    /// Stable timer id.
    pub id: String,
    /// Human label spoken when it fires.
    pub label: String,
    /// Due time as Unix seconds UTC.
    pub due_at: u64,
}

/// Schedule reminders that the daemon fires through the kiosk.
#[async_trait]
pub trait TimerControl: Send + Sync {
    /// Create a timer for `profile_id` that fires after `duration_secs`.
    ///
    /// # Errors
    /// Returns a human-readable message when persistence fails.
    async fn create(
        &self,
        profile_id: &str,
        label: &str,
        duration_secs: u64,
    ) -> Result<TimerEntry, String>;

    /// List pending timers for `profile_id`.
    ///
    /// # Errors
    /// Returns a human-readable message when persistence fails.
    async fn list(&self, profile_id: &str) -> Result<Vec<TimerEntry>, String>;

    /// Cancel a timer owned by `profile_id`; returns whether it existed.
    ///
    /// # Errors
    /// Returns a human-readable message when persistence fails.
    async fn cancel(&self, profile_id: &str, id: &str) -> Result<bool, String>;
}

/// Small in-memory [`TimerControl`] used by tests and ephemeral runs.
#[derive(Debug, Default)]
pub struct EphemeralTimerStore {
    timers: std::sync::Mutex<Vec<(String, TimerEntry)>>,
    next: std::sync::atomic::AtomicU64,
}

#[async_trait]
impl TimerControl for EphemeralTimerStore {
    async fn create(
        &self,
        profile_id: &str,
        label: &str,
        duration_secs: u64,
    ) -> Result<TimerEntry, String> {
        let id = format!(
            "timer-{}",
            self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        );
        let due_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_secs()
            + duration_secs;
        let entry = TimerEntry {
            id,
            label: label.to_owned(),
            due_at,
        };
        self.timers
            .lock()
            .map_err(|_| "timer lock poisoned".to_owned())?
            .push((profile_id.to_owned(), entry.clone()));
        Ok(entry)
    }

    async fn list(&self, profile_id: &str) -> Result<Vec<TimerEntry>, String> {
        Ok(self
            .timers
            .lock()
            .map_err(|_| "timer lock poisoned".to_owned())?
            .iter()
            .filter(|(owner, _)| owner == profile_id)
            .map(|(_, entry)| entry.clone())
            .collect())
    }

    async fn cancel(&self, profile_id: &str, id: &str) -> Result<bool, String> {
        let mut timers = self
            .timers
            .lock()
            .map_err(|_| "timer lock poisoned".to_owned())?;
        let before = timers.len();
        timers.retain(|(owner, entry)| !(owner == profile_id && entry.id == id));
        Ok(timers.len() != before)
    }
}

/// One current news item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewsItem {
    /// Headline.
    pub title: String,
    /// Source/publication name.
    pub source: String,
    /// Link to the original article.
    pub link: String,
    /// Publication date as returned by the feed.
    pub published_at: String,
}

/// Read-only current-events search (RSS/news feeds).
#[async_trait]
pub trait NewsSearch: Send + Sync {
    /// Return the latest items, optionally filtered by `query`.
    ///
    /// # Errors
    /// Returns a human-readable message when the feed is unreachable.
    async fn latest(&self, query: Option<&str>, limit: usize) -> Result<Vec<NewsItem>, String>;
}

/// Empty news adapter used by tests and ephemeral runs.
#[derive(Debug, Default)]
pub struct EphemeralNews;

#[async_trait]
impl NewsSearch for EphemeralNews {
    async fn latest(&self, _query: Option<&str>, limit: usize) -> Result<Vec<NewsItem>, String> {
        Ok((0..limit.min(2))
            .map(|index| NewsItem {
                title: format!("Test headline {}", index + 1),
                source: "Test feed".to_owned(),
                link: format!("https://example.test/{index}"),
                published_at: "2026-11-25T10:00:00Z".to_owned(),
            })
            .collect())
    }
}
/// Allow-listed outbound actions such as Composio calendar/email tools.
#[async_trait]
pub trait ExternalActions: Send + Sync {
    /// Execute a named capability with already validated arguments.
    ///
    /// # Errors
    /// Returns a human-readable message when the integration is disabled,
    /// not authenticated or the provider rejects the action.
    async fn execute(
        &self,
        capability: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}

/// Deterministic external-action fake used by tests.
#[derive(Debug, Default)]
pub struct EphemeralExternal;

#[async_trait]
impl ExternalActions for EphemeralExternal {
    async fn execute(
        &self,
        capability: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({
            "capability": capability,
            "arguments": arguments,
            "status": "ok",
        }))
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
    /// Per-profile reminders/timers.
    pub timers: Arc<dyn TimerControl>,
    /// Read-only current-events feeds.
    pub news: Arc<dyn NewsSearch>,
    /// Allow-listed external capabilities (Composio calendar/email).
    pub external: Arc<dyn ExternalActions>,
}

impl std::fmt::Debug for ToolPorts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ToolPorts").finish_non_exhaustive()
    }
}
