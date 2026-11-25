//! Real-time events exchanged with the wall kiosk.
//!
//! The daemon keeps a `tokio::sync::broadcast` channel of [`KioskEvent`]s. The
//! vision service, the audio controller and the agent publish into it; the
//! kiosk WebSocket forwards them to the browser. Events are intentionally
//! small and JSON-tagged (`{"type": "...", ...}`).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::Profile;

/// Visual state of the animated avatar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvatarState {
    /// Breathing idle state with the clock.
    Idle,
    /// Eyes follow a person in front of the camera.
    Tracking,
    /// Microphone is capturing (VAD active).
    Listening,
    /// LLM or tools are working.
    Thinking,
    /// TTS/audio playback in progress.
    Speaking,
    /// Transition when a recognized profile takes over.
    ProfileSwitch,
}

impl AvatarState {
    /// Stable identifier used by the API.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Tracking => "tracking",
            Self::Listening => "listening",
            Self::Thinking => "thinking",
            Self::Speaking => "speaking",
            Self::ProfileSwitch => "profile_switch",
        }
    }
}

/// Normalized gaze point in `[0, 1]` coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TrackingPoint {
    /// Horizontal coordinate, `0.0` left to `1.0` right.
    pub x: f32,
    /// Vertical coordinate, `0.0` top to `1.0` bottom.
    pub y: f32,
    /// Detector confidence in `[0, 1]`.
    pub confidence: f32,
}

/// Music playback state mirrored by the kiosk controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MusicState {
    /// Whether playback is active.
    pub playing: bool,
    /// Current track title.
    pub track: Option<String>,
    /// Current artist.
    pub artist: Option<String>,
    /// Volume level 0..=100.
    pub volume: u8,
    /// Output sink description.
    pub sink: Option<String>,
}

impl Default for MusicState {
    fn default() -> Self {
        Self {
            playing: false,
            track: None,
            artist: None,
            volume: 60,
            sink: None,
        }
    }
}

/// Coarse agent activity shown as a discreet indicator on the kiosk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivity {
    /// Not doing anything.
    Idle,
    /// Capturing user speech.
    Listening,
    /// Processing with the LLM/tools.
    Thinking,
    /// Producing speech.
    Speaking,
    /// Last request failed.
    Error,
}

/// Network health summary shown in the kiosk status bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkStatus {
    /// Whether the appliance has connectivity.
    pub online: bool,
    /// Whether encrypted DNS resolution works.
    pub dns_ok: bool,
    /// Round-trip latency of the DNS probe, when available.
    pub latency_ms: Option<u32>,
}

/// Severity of a kiosk notification/toast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationLevel {
    /// Neutral information.
    Info,
    /// Operation succeeded.
    Success,
    /// Non-fatal problem.
    Warning,
    /// Operation failed.
    Error,
}

/// Events pushed to the wall kiosk over WebSocket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KioskEvent {
    /// Avatar state transition.
    Avatar {
        /// New state.
        state: AvatarState,
        /// Profile owning the avatar, when known.
        profile_id: Option<Uuid>,
    },
    /// Gaze point update; `None` means "nobody detected".
    Tracking {
        /// Point in normalized screen coordinates.
        point: Option<TrackingPoint>,
    },
    /// A recognized profile became active.
    ProfileActivated {
        /// Full profile so the kiosk can render the avatar immediately.
        profile: Profile,
    },
    /// The assistant voice/profile settings changed in the admin console.
    Voice {
        /// Configured assistant name (also the wake word).
        assistant_name: String,
        /// Whether wake-word listening is enabled.
        wake_word_enabled: bool,
        /// Activation mode id (`wake_word`, `push_to_talk`, `both`).
        activation_mode: String,
        /// STT/TTS language (`es`, `en`).
        language: String,
    },
    /// The household called the assistant by its wake word.
    Wake {
        /// Configured assistant name (`Aura`).
        assistant_name: String,
        /// Locally transcribed request that followed the name, when present.
        trailing_query: Option<String>,
    },
    /// The assistant produced a reply that the kiosk should speak locally.
    Speech {
        /// Reply text, rendered with the browser/OS local voice.
        text: String,
    },
    /// Music state changed.
    Music {
        /// New music state.
        state: MusicState,
    },
    /// Agent activity changed.
    Agent {
        /// New activity.
        activity: AgentActivity,
    },
    /// Network status changed.
    Network {
        /// New status.
        status: NetworkStatus,
    },
    /// Transient notification.
    Notification {
        /// Severity.
        level: NotificationLevel,
        /// English technical message; the UI localizes known prefixes.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kiosk_events_are_json_tagged() {
        let event = KioskEvent::Tracking {
            point: Some(TrackingPoint {
                x: 0.25,
                y: 0.75,
                confidence: 0.9,
            }),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "tracking");
        assert_eq!(json["point"]["x"], 0.25);
    }

    #[test]
    fn avatar_state_ids_are_stable() {
        assert_eq!(AvatarState::ProfileSwitch.id(), "profile_switch");
        assert_eq!(AvatarState::Idle.id(), "idle");
    }
}
