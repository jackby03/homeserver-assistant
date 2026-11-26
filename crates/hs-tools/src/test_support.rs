//! Shared fakes used by the unit tests of this crate.
//!
//! The fakes model the outbound ports instead of mocking processes: they count
//! calls and return deterministic data, which keeps the security tests (denied
//! calls must never reach an adapter) fast and hermetic.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use hs_core::models::{
    Avatar, InstanceSummary, Profile, ProfileRole, ResourceLimits, RuntimeKind, RuntimeStatus, Tone,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::ports::{
    AudioControl, DomoticsControl, EntityLite, MemoryEntry, MemoryStore, RuntimeControl,
    SystemControl, ToolPorts, WebhookClient,
};

/// Number of adapter calls observed by the fakes.
#[derive(Debug, Default)]
pub(crate) struct CallCounts {
    pub(crate) runtime: Arc<AtomicUsize>,
    pub(crate) domotics: Arc<AtomicUsize>,
    pub(crate) audio: Arc<AtomicUsize>,
    pub(crate) system: Arc<AtomicUsize>,
    pub(crate) webhooks: Arc<AtomicUsize>,
}

impl CallCounts {
    /// Snapshot in `[runtime, domotics, audio, system, webhooks]` order.
    pub(crate) fn snapshot(&self) -> [usize; 5] {
        [
            self.runtime.load(Ordering::SeqCst),
            self.domotics.load(Ordering::SeqCst),
            self.audio.load(Ordering::SeqCst),
            self.system.load(Ordering::SeqCst),
            self.webhooks.load(Ordering::SeqCst),
        ]
    }
}

/// Build a [`ToolPorts`] bundle backed by fakes.
///
/// Returns the ports, the shared call counters and the last webhook payload
/// delivered to the fake webhook client.
pub(crate) fn fake_ports() -> (ToolPorts, Arc<CallCounts>, Arc<Mutex<Option<Value>>>) {
    let counts = Arc::new(CallCounts::default());
    let webhook_payload = Arc::new(Mutex::new(None));
    let ports = ToolPorts {
        runtime: Arc::new(FakeRuntime {
            calls: Arc::clone(&counts.runtime),
        }),
        domotics: Arc::new(FakeDomotics {
            calls: Arc::clone(&counts.domotics),
        }),
        audio: Arc::new(FakeAudio {
            calls: Arc::clone(&counts.audio),
        }),
        system: Arc::new(FakeSystem {
            calls: Arc::clone(&counts.system),
        }),
        webhooks: Arc::new(FakeWebhooks {
            calls: Arc::clone(&counts.webhooks),
            payload: Arc::clone(&webhook_payload),
        }),
        memory: Arc::new(FakeMemory::default()),
        timers: Arc::new(crate::ports::EphemeralTimerStore::default()),
        news: Arc::new(crate::ports::EphemeralNews),
        external: Arc::new(crate::ports::EphemeralExternal),
    };
    (ports, counts, webhook_payload)
}

/// Deterministic profile used by permission tests.
pub(crate) fn profile_with_tools(tools: &[impl AsRef<str>]) -> Profile {
    Profile {
        id: Uuid::new_v4(),
        name: "Test Profile".to_owned(),
        role: ProfileRole::Owner,
        avatar: Avatar::default(),
        language: "en".to_owned(),
        tone: Tone::Friendly,
        allowed_tools: tools.iter().map(|tool| tool.as_ref().to_owned()).collect(),
        face_enrolled: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

struct FakeRuntime {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl RuntimeControl for FakeRuntime {
    async fn list_instances(&self) -> Result<Vec<InstanceSummary>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![InstanceSummary {
            name: "home-assistant".to_owned(),
            kind: RuntimeKind::Container,
            status: RuntimeStatus::Running,
            image: "images:debian/13".to_owned(),
            ipv4_addresses: vec!["10.0.0.10".to_owned()],
            limits: ResourceLimits::default(),
            created_at: None,
            snapshot_count: 0,
        }])
    }

    async fn start_instance(&self, _name: &str) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn stop_instance(&self, _name: &str) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct FakeDomotics {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl DomoticsControl for FakeDomotics {
    async fn list_entities(&self) -> Result<Vec<EntityLite>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![EntityLite {
            entity_id: "light.kitchen".to_owned(),
            state: "on".to_owned(),
            name: "Kitchen light".to_owned(),
        }])
    }

    async fn call_service(
        &self,
        _domain: &str,
        _service: &str,
        _data: Value,
    ) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct FakeAudio {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl AudioControl for FakeAudio {
    async fn play(&self) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn pause(&self) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn set_volume(&self, _percent: u8) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct FakeSystem {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl SystemControl for FakeSystem {
    async fn overview(&self) -> Result<Value, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({
            "status": "ok",
            "cpu": { "usage_percent": 7.5 },
            "memory": { "used_mb": 512, "total_mb": 4096 }
        }))
    }
}

struct FakeWebhooks {
    calls: Arc<AtomicUsize>,
    payload: Arc<Mutex<Option<Value>>>,
}

#[async_trait]
impl WebhookClient for FakeWebhooks {
    async fn post(&self, _url: &str, payload: Value) -> Result<u16, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut slot) = self.payload.lock() {
            *slot = Some(payload);
        }
        Ok(200)
    }
}

#[derive(Default)]
struct FakeMemory {
    entries: Arc<Mutex<Vec<(String, MemoryEntry)>>>,
}

#[async_trait]
impl MemoryStore for FakeMemory {
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
        let mut entries = self.entries.lock().map_err(|_| "memory lock".to_owned())?;
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
        let entries = self.entries.lock().map_err(|_| "memory lock".to_owned())?;
        let prefix = format!("{profile_id}:");
        let needles = query.map(str::to_lowercase);
        Ok(entries
            .iter()
            .filter(|(scope, _)| scope.starts_with(&prefix))
            .map(|(_, entry)| entry)
            .filter(|entry| {
                needles.as_ref().is_none_or(|needle| {
                    entry.key.to_lowercase().contains(needle)
                        || entry.value.to_lowercase().contains(needle)
                })
            })
            .cloned()
            .collect())
    }

    async fn forget(&self, profile_id: &str, key: &str) -> Result<bool, String> {
        let mut entries = self.entries.lock().map_err(|_| "memory lock".to_owned())?;
        let scope = format!("{profile_id}:{key}");
        let before = entries.len();
        entries.retain(|(stored, _)| stored != &scope);
        Ok(entries.len() != before)
    }
}
