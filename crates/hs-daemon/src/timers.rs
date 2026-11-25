//! Persistent reminder timers that fire through the kiosk.
//!
//! Timers are stored in `data/timers.json` and a lightweight scheduler checks
//! them once per second. When due, the timer is removed and the kiosk receives a
//! `Speech` + `Notification` event so the assistant speaks the reminder even if
//! the chat panel is closed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use hs_core::events::{KioskEvent, NotificationLevel};
use hs_tools::{TimerControl, TimerEntry};
use serde::{Deserialize, Serialize};

use crate::events::EventBus;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredTimer {
    id: String,
    profile_id: String,
    label: String,
    due_at: u64,
}

/// In-process scheduler with JSON persistence.
#[derive(Debug)]
pub struct TimerService {
    path: PathBuf,
    timers: Mutex<Vec<StoredTimer>>,
    events: EventBus,
}

impl TimerService {
    /// Load pending timers and create the service.
    #[must_use]
    pub fn new(data_dir: &Path, events: EventBus) -> Arc<Self> {
        let path = data_dir.join("timers.json");
        let timers = std::fs::read_to_string(&path)
            .ok()
            .and_then(|contents| serde_json::from_str::<Vec<StoredTimer>>(&contents).ok())
            .unwrap_or_default();
        Arc::new(Self {
            path,
            timers: Mutex::new(timers),
            events,
        })
    }

    /// Start the once-per-second scheduler.
    pub fn start(self: &Arc<Self>) {
        let service = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                ticker.tick().await;
                service.fire_due();
            }
        });
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    }

    fn persist(&self, timers: &[StoredTimer]) {
        if let Ok(contents) = serde_json::to_vec_pretty(timers) {
            if let Some(parent) = self.path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let temporary = self.path.with_extension("json.tmp");
            if std::fs::write(&temporary, contents).is_ok() {
                let _ = std::fs::rename(&temporary, &self.path);
            }
        }
    }

    fn fire_due(&self) {
        let now = Self::now();
        let mut due = Vec::new();
        {
            let Ok(mut timers) = self.timers.lock() else {
                return;
            };
            let mut remaining = Vec::with_capacity(timers.len());
            for timer in timers.drain(..) {
                if timer.due_at <= now {
                    due.push(timer);
                } else {
                    remaining.push(timer);
                }
            }
            *timers = remaining;
            if !due.is_empty() {
                let snapshot = timers.clone();
                drop(timers);
                self.persist(&snapshot);
            }
        }
        for timer in due {
            tracing::info!(label = %timer.label, profile = %timer.profile_id, "timer fired");
            self.events.publish_kiosk(KioskEvent::Speech {
                text: format!("Recordatorio: {}", timer.label),
            });
            self.events.publish_kiosk(KioskEvent::Notification {
                level: NotificationLevel::Info,
                message: format!("Recordatorio: {}", timer.label),
            });
        }
    }
}

#[async_trait]
impl TimerControl for TimerService {
    async fn create(
        &self,
        profile_id: &str,
        label: &str,
        duration_secs: u64,
    ) -> Result<TimerEntry, String> {
        let timer = StoredTimer {
            id: uuid::Uuid::new_v4().to_string(),
            profile_id: profile_id.to_owned(),
            label: label.to_owned(),
            due_at: Self::now() + duration_secs,
        };
        let entry = TimerEntry {
            id: timer.id.clone(),
            label: timer.label.clone(),
            due_at: timer.due_at,
        };
        let snapshot = {
            let mut timers = self
                .timers
                .lock()
                .map_err(|_| "timer lock poisoned".to_owned())?;
            timers.push(timer);
            timers.clone()
        };
        self.persist(&snapshot);
        Ok(entry)
    }

    async fn list(&self, profile_id: &str) -> Result<Vec<TimerEntry>, String> {
        let timers = self
            .timers
            .lock()
            .map_err(|_| "timer lock poisoned".to_owned())?;
        let mut entries: Vec<TimerEntry> = timers
            .iter()
            .filter(|timer| timer.profile_id == profile_id)
            .map(|timer| TimerEntry {
                id: timer.id.clone(),
                label: timer.label.clone(),
                due_at: timer.due_at,
            })
            .collect();
        entries.sort_by_key(|entry| entry.due_at);
        Ok(entries)
    }

    async fn cancel(&self, profile_id: &str, id: &str) -> Result<bool, String> {
        let snapshot = {
            let mut timers = self
                .timers
                .lock()
                .map_err(|_| "timer lock poisoned".to_owned())?;
            let before = timers.len();
            timers.retain(|timer| !(timer.profile_id == profile_id && timer.id == id));
            if timers.len() == before {
                return Ok(false);
            }
            timers.clone()
        };
        self.persist(&snapshot);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn timers_are_persisted_listed_and_cancelled() {
        let directory = std::env::temp_dir().join(format!("hs-timers-{}", uuid::Uuid::new_v4()));
        let service = TimerService::new(&directory, EventBus::new(16));
        let created = service
            .create("alice", "sacar la basura", 600)
            .await
            .expect("create");
        assert_eq!(service.list("alice").await.expect("list").len(), 1);
        assert!(service.cancel("alice", &created.id).await.expect("cancel"));
        assert!(service.list("alice").await.expect("list").is_empty());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
