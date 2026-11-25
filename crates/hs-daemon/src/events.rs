//! Real-time event bus shared by HTTP handlers and background tasks.
//!
//! Two independent broadcast channels are kept:
//! - [`ServerEvent`]: admin portal live updates (metrics, setup progress, audit).
//! - [`KioskEvent`]: wall kiosk updates (avatar, tracking, music, ...).
//!
//! Slow subscribers are dropped by `tokio::broadcast` semantics; the UI
//! reconnects and refetches, which is the correct behavior for a dashboard.

use hs_core::events::KioskEvent;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Default broadcast buffer size.
pub const EVENT_BUFFER: usize = 256;

/// Admin portal events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// Periodic system overview. Serialized from `hs_system::Overview`.
    Metrics {
        /// Overview payload (kept as JSON to decouple the event bus).
        overview: serde_json::Value,
    },
    /// Runtime instance state changed.
    Instance {
        /// Instance name.
        name: String,
        /// Normalized status identifier.
        status: String,
    },
    /// Setup wizard deployment progress.
    SetupProgress {
        /// Step number (1-6).
        step: u8,
        /// Human readable English label.
        label: String,
        /// Completion percentage (0-100).
        percent: u8,
        /// Whether the step finished.
        done: bool,
    },
    /// Setup completed; the portal should reload.
    SetupFinished,
    /// Discovery results changed.
    Discovery {
        /// Number of known devices.
        count: usize,
    },
    /// New audit entry (also mirrored by REST).
    Audit {
        /// The entry.
        entry: hs_core::models::AuditEntry,
    },
    /// Generic notification for the admin UI.
    Notification {
        /// Severity identifier.
        level: String,
        /// English technical message.
        message: String,
    },
}

/// Bus holding both broadcast channels.
#[derive(Debug, Clone)]
pub struct EventBus {
    server: broadcast::Sender<ServerEvent>,
    kiosk: broadcast::Sender<KioskEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(EVENT_BUFFER)
    }
}

impl EventBus {
    /// Create a bus with the given channel capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (server, _) = broadcast::channel(capacity);
        let (kiosk, _) = broadcast::channel(capacity);
        Self { server, kiosk }
    }

    /// Subscribe to admin portal events.
    #[must_use]
    pub fn subscribe_server(&self) -> broadcast::Receiver<ServerEvent> {
        self.server.subscribe()
    }

    /// Subscribe to kiosk events.
    #[must_use]
    pub fn subscribe_kiosk(&self) -> broadcast::Receiver<KioskEvent> {
        self.kiosk.subscribe()
    }

    /// Publish an admin event. Errors (no receivers) are ignored on purpose.
    pub fn publish_server(&self, event: ServerEvent) {
        let _ = self.server.send(event);
    }

    /// Publish a kiosk event. Errors (no receivers) are ignored on purpose.
    pub fn publish_kiosk(&self, event: KioskEvent) {
        let _ = self.kiosk.send(event);
    }

    /// Number of connected admin subscribers (best effort, for diagnostics).
    #[must_use]
    pub fn server_receiver_count(&self) -> usize {
        self.server.receiver_count()
    }

    /// Number of connected kiosk subscribers.
    #[must_use]
    pub fn kiosk_receiver_count(&self) -> usize {
        self.kiosk.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publishes_to_both_channels() {
        let bus = EventBus::new(8);
        let mut server = bus.subscribe_server();
        let mut kiosk = bus.subscribe_kiosk();

        bus.publish_server(ServerEvent::SetupFinished);
        bus.publish_kiosk(KioskEvent::Avatar {
            state: hs_core::events::AvatarState::Idle,
            profile_id: None,
        });

        assert!(matches!(
            server.recv().await.unwrap(),
            ServerEvent::SetupFinished
        ));
        assert!(matches!(
            kiosk.recv().await.unwrap(),
            KioskEvent::Avatar { .. }
        ));
    }

    #[test]
    fn publishing_without_subscribers_is_silent() {
        let bus = EventBus::new(4);
        bus.publish_server(ServerEvent::SetupFinished);
        bus.publish_kiosk(KioskEvent::Notification {
            level: hs_core::events::NotificationLevel::Info,
            message: "hello".to_owned(),
        });
    }

    #[test]
    fn server_events_are_json_tagged() {
        let event = ServerEvent::SetupProgress {
            step: 2,
            label: "Configuring network".to_owned(),
            percent: 40,
            done: false,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "setup_progress");
        assert_eq!(json["step"], 2);
    }
}
