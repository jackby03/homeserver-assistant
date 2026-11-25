//! Wall kiosk state machine.
//!
//! The kiosk UI is a thin renderer: the daemon owns the active profile, avatar
//! state, gaze point, music state and network status, and streams every change
//! over the kiosk WebSocket. New kiosk clients receive a snapshot first.

use std::sync::RwLock;

use hs_core::config::VoiceConfig;
use hs_core::events::{
    AgentActivity, AvatarState, KioskEvent, MusicState, NetworkStatus, NotificationLevel,
    TrackingPoint,
};
use hs_core::models::Profile;

use crate::events::EventBus;

/// Snapshot returned by `GET /api/v1/kiosk/state`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KioskSnapshot {
    /// Device display name.
    pub device_name: String,
    /// Active profile, if any.
    pub profile: Option<Profile>,
    /// Current avatar animation state.
    pub avatar_state: AvatarState,
    /// Current gaze point.
    pub tracking: Option<TrackingPoint>,
    /// Music playback state.
    pub music: MusicState,
    /// Network health.
    pub network: NetworkStatus,
    /// Agent activity.
    pub agent_activity: AgentActivity,
}

/// Mutable kiosk state guarded by a `RwLock`.
#[derive(Debug)]
struct KioskState {
    device_name: String,
    profile: Option<Profile>,
    avatar_state: AvatarState,
    tracking: Option<TrackingPoint>,
    music: MusicState,
    network: NetworkStatus,
    agent_activity: AgentActivity,
}

/// Kiosk state manager. Cheap to clone through an `Arc`.
#[derive(Debug)]
pub struct KioskManager {
    bus: EventBus,
    state: RwLock<KioskState>,
}

impl KioskManager {
    /// Create the manager with idle defaults.
    #[must_use]
    pub fn new(bus: EventBus, device_name: impl Into<String>) -> Self {
        Self {
            bus,
            state: RwLock::new(KioskState {
                device_name: device_name.into(),
                profile: None,
                avatar_state: AvatarState::Idle,
                tracking: None,
                music: MusicState::default(),
                network: NetworkStatus {
                    online: false,
                    dns_ok: false,
                    latency_ms: None,
                },
                agent_activity: AgentActivity::Idle,
            }),
        }
    }

    /// Update the device display name.
    pub fn set_device_name(&self, name: impl Into<String>) {
        if let Ok(mut state) = self.state.write() {
            state.device_name = name.into();
        }
    }

    /// Current snapshot.
    #[must_use]
    pub fn snapshot(&self) -> KioskSnapshot {
        let state = self
            .state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        KioskSnapshot {
            device_name: state.device_name.clone(),
            profile: state.profile.clone(),
            avatar_state: state.avatar_state,
            tracking: state.tracking,
            music: state.music.clone(),
            network: state.network.clone(),
            agent_activity: state.agent_activity,
        }
    }

    /// Activate a profile (manual selection or face recognition).
    pub fn activate_profile(&self, profile: Profile) {
        {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.profile = Some(profile.clone());
            state.avatar_state = AvatarState::ProfileSwitch;
        }
        self.bus.publish_kiosk(KioskEvent::ProfileActivated {
            profile: profile.clone(),
        });
        self.publish_avatar(AvatarState::ProfileSwitch);
        // Return to idle breathing after the morph animation.
        self.publish_avatar(AvatarState::Idle);
    }

    /// Clear the active profile (guest mode).
    pub fn clear_profile(&self) {
        if let Ok(mut state) = self.state.write() {
            state.profile = None;
            state.avatar_state = AvatarState::Idle;
        }
        self.publish_avatar(AvatarState::Idle);
    }

    /// Set the avatar state explicitly (listening/speaking/thinking).
    pub fn set_avatar_state(&self, avatar_state: AvatarState) {
        {
            let Ok(mut state) = self.state.write() else {
                return;
            };
            state.avatar_state = avatar_state;
        }
        self.publish_avatar(avatar_state);
    }

    /// Update the gaze point and derive tracking/idle avatar states.
    pub fn set_tracking(&self, point: Option<TrackingPoint>) {
        let next_state = if point.is_some() {
            AvatarState::Tracking
        } else {
            AvatarState::Idle
        };
        {
            let Ok(mut state) = self.state.write() else {
                return;
            };
            state.tracking = point;
            // Do not override active conversation states.
            if matches!(
                state.avatar_state,
                AvatarState::Idle | AvatarState::Tracking | AvatarState::ProfileSwitch
            ) {
                state.avatar_state = next_state;
            }
        }
        self.bus.publish_kiosk(KioskEvent::Tracking { point });
        self.publish_avatar(next_state);
    }

    /// Set the agent activity indicator.
    pub fn set_agent_activity(&self, activity: AgentActivity) {
        {
            let Ok(mut state) = self.state.write() else {
                return;
            };
            state.agent_activity = activity;
        }
        self.bus.publish_kiosk(KioskEvent::Agent { activity });
    }

    /// Set the music playback state.
    pub fn set_music(&self, music: MusicState) {
        {
            let Ok(mut state) = self.state.write() else {
                return;
            };
            state.music = music.clone();
        }
        self.bus.publish_kiosk(KioskEvent::Music { state: music });
    }

    /// Set the network health status.
    pub fn set_network(&self, network: NetworkStatus) {
        {
            let Ok(mut state) = self.state.write() else {
                return;
            };
            state.network = network.clone();
        }
        self.bus
            .publish_kiosk(KioskEvent::Network { status: network });
    }

    /// Publish a transient notification to the kiosk.
    pub fn notify(&self, level: NotificationLevel, message: impl Into<String>) {
        self.bus.publish_kiosk(KioskEvent::Notification {
            level,
            message: message.into(),
        });
    }

    /// Notify the kiosk that the assistant name/voice settings changed.
    pub fn voice_updated(&self, config: &VoiceConfig) {
        self.bus.publish_kiosk(KioskEvent::Voice {
            assistant_name: config.assistant_name.clone(),
            wake_word_enabled: config.wake_word_enabled,
            activation_mode: config.activation_mode.id().to_owned(),
            language: config.language.clone(),
        });
    }

    /// Ask the kiosk to speak an assistant reply with its local voice.
    pub fn speak(&self, text: impl Into<String>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        self.bus.publish_kiosk(KioskEvent::Speech { text });
    }

    /// Access the kiosk event bus (used by WebSocket handlers).
    #[must_use]
    pub fn bus(&self) -> &EventBus {
        &self.bus
    }

    fn publish_avatar(&self, state: AvatarState) {
        let profile_id = self
            .state
            .read()
            .ok()
            .and_then(|guard| guard.profile.as_ref().map(|profile| profile.id));
        self.bus
            .publish_kiosk(KioskEvent::Avatar { state, profile_id });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hs_core::models::{Avatar, ProfileRole, Tone};
    use uuid::Uuid;

    fn profile() -> Profile {
        Profile {
            id: Uuid::new_v4(),
            name: "Mamá".to_owned(),
            role: ProfileRole::Adult,
            avatar: Avatar::default(),
            language: "es".to_owned(),
            tone: Tone::Friendly,
            allowed_tools: Vec::new(),
            face_enrolled: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn activates_profile_and_emits_events() {
        let bus = EventBus::new(16);
        let mut receiver = bus.subscribe_kiosk();
        let kiosk = KioskManager::new(bus, "Salón");
        let profile = profile();
        kiosk.activate_profile(profile.clone());
        assert_eq!(kiosk.snapshot().profile.unwrap().id, profile.id);

        let mut types = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            types.push(match event {
                KioskEvent::ProfileActivated { .. } => "profile_activated",
                KioskEvent::Avatar { .. } => "avatar",
                other => panic!("unexpected event: {other:?}"),
            });
        }
        assert_eq!(types[0], "profile_activated");
        assert_eq!(types[1], "avatar");
    }

    #[test]
    fn tracking_updates_snapshot_and_state() {
        let bus = EventBus::new(16);
        let kiosk = KioskManager::new(bus, "Salón");
        kiosk.set_tracking(Some(TrackingPoint {
            x: 0.25,
            y: 0.5,
            confidence: 0.9,
        }));
        let snapshot = kiosk.snapshot();
        assert_eq!(snapshot.avatar_state, AvatarState::Tracking);
        assert!(snapshot.tracking.is_some());

        kiosk.set_tracking(None);
        let snapshot = kiosk.snapshot();
        assert_eq!(snapshot.avatar_state, AvatarState::Idle);
        assert!(snapshot.tracking.is_none());
    }

    #[test]
    fn conversation_state_is_not_overridden_by_tracking() {
        let bus = EventBus::new(16);
        let kiosk = KioskManager::new(bus, "Salón");
        kiosk.set_avatar_state(AvatarState::Listening);
        kiosk.set_tracking(Some(TrackingPoint {
            x: 0.5,
            y: 0.5,
            confidence: 1.0,
        }));
        assert_eq!(kiosk.snapshot().avatar_state, AvatarState::Listening);
    }

    #[test]
    fn music_and_network_are_mirrored() {
        let bus = EventBus::new(16);
        let kiosk = KioskManager::new(bus, "Salón");
        kiosk.set_music(MusicState {
            playing: true,
            track: Some("Song".to_owned()),
            artist: None,
            volume: 42,
            sink: None,
        });
        kiosk.set_network(NetworkStatus {
            online: true,
            dns_ok: true,
            latency_ms: Some(12),
        });
        let snapshot = kiosk.snapshot();
        assert!(snapshot.music.playing);
        assert_eq!(snapshot.music.volume, 42);
        assert!(snapshot.network.online);
    }
}
