//! Always-on voice assistant: wake-word gate, listening window and status.
//!
//! The daemon never captures audio itself. A local listener transcribes speech
//! on the appliance (optional `voice/wakeword.py` helper with a local STT model)
//! and posts the transcript to `POST /api/v1/kiosk/wake`. This module matches
//! the configured assistant name locally, opens a short listening window and
//! notifies the kiosk over the event bus.
//!
//! Privacy: transcripts are used in memory for matching only; they are not
//! written to disk, logged or forwarded anywhere.

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use hs_core::config::{VoiceActivationMode, VoiceConfig};
use hs_core::events::{AgentActivity, AvatarState, KioskEvent};
use hs_core::state::ConfigManager;
use hs_core::voice::WakeWordMatcher;
use serde::{Deserialize, Serialize};

use crate::kiosk::KioskManager;

/// Minimum time between two wake activations (avoids double triggers from the
/// same utterance being transcribed twice by the local listener).
const WAKE_COOLDOWN: Duration = Duration::from_millis(1500);

/// Voice assistant status exposed to the admin portal and kiosk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceStatus {
    /// Configured assistant name.
    pub assistant_name: String,
    /// Whether the always-on listener is enabled.
    pub wake_word_enabled: bool,
    /// Activation mode.
    pub activation_mode: VoiceActivationMode,
    /// Whether the assistant is inside the listening window right now.
    pub listening: bool,
    /// Last successful wake activation.
    pub last_activation: Option<DateTime<Utc>>,
    /// Number of wake activations since process start.
    pub activations: u64,
    /// Whether the local microphone listener reported a heartbeat recently.
    pub listener_connected: bool,
}

/// Result of feeding a local transcript into the wake-word gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakeOutcome {
    /// Whether the assistant name was recognized and the window opened.
    pub activated: bool,
    /// Configured assistant name.
    pub assistant_name: String,
    /// Whether the assistant is now listening.
    pub listening: bool,
    /// Words spoken after the name, when the listener chained a request.
    pub trailing_query: Option<String>,
    /// Machine-readable reason (`activated`, `not_matched`, `wake_disabled`,
    /// `push_to_talk_only`, `cooldown`).
    pub reason: &'static str,
}

#[derive(Debug)]
struct VoiceRuntime {
    listening_until: Option<Instant>,
    last_activation_instant: Option<Instant>,
    last_activation: Option<DateTime<Utc>>,
    activations: u64,
    last_listener_heartbeat: Option<Instant>,
}

/// Wake-word gate and listening-window state machine.
#[derive(Debug)]
pub struct VoiceManager {
    config: Arc<ConfigManager>,
    kiosk: Arc<KioskManager>,
    runtime: RwLock<VoiceRuntime>,
}

impl VoiceManager {
    /// Create the voice manager for the given configuration and kiosk.
    #[must_use]
    pub fn new(config: Arc<ConfigManager>, kiosk: Arc<KioskManager>) -> Self {
        Self {
            config,
            kiosk,
            runtime: RwLock::new(VoiceRuntime {
                listening_until: None,
                last_activation_instant: None,
                last_activation: None,
                activations: 0,
                last_listener_heartbeat: None,
            }),
        }
    }

    /// Current voice configuration snapshot.
    #[must_use]
    pub fn voice_config(&self) -> VoiceConfig {
        self.config.snapshot().voice
    }

    /// Current status for the UI.
    #[must_use]
    pub fn status(&self) -> VoiceStatus {
        let config = self.voice_config();
        let runtime = self.read();
        VoiceStatus {
            assistant_name: config.assistant_name,
            wake_word_enabled: config.wake_word_enabled,
            activation_mode: config.activation_mode,
            listening: runtime
                .listening_until
                .is_some_and(|until| until > Instant::now()),
            last_activation: runtime.last_activation,
            activations: runtime.activations,
            listener_connected: runtime
                .last_listener_heartbeat
                .is_some_and(|at| at.elapsed() < Duration::from_secs(45)),
        }
    }

    /// Record a heartbeat from the local microphone listener.
    pub fn listener_heartbeat(&self) {
        self.write().last_listener_heartbeat = Some(Instant::now());
    }

    /// Whether the local microphone listener is connected (heartbeat < 45 s).
    #[must_use]
    pub fn listener_connected(&self) -> bool {
        self.read()
            .last_listener_heartbeat
            .is_some_and(|at| at.elapsed() < Duration::from_secs(45))
    }

    /// Whether the assistant is inside the post-wake listening window.
    #[must_use]
    pub fn is_listening(&self) -> bool {
        self.read()
            .listening_until
            .is_some_and(|until| until > Instant::now())
    }

    /// Feed a locally produced transcript into the wake-word gate.
    ///
    /// When the configured name is recognized, the kiosk is switched to the
    /// listening state, a `KioskEvent::Wake` is published and a listening
    /// window is opened for the following request.
    pub fn handle_transcript(&self, transcript: &str, confidence: Option<f32>) -> WakeOutcome {
        let config = self.voice_config();

        if !config.wake_word_enabled {
            return self.rejected("wake_disabled", &config);
        }
        if config.activation_mode == VoiceActivationMode::PushToTalk {
            return self.rejected("push_to_talk_only", &config);
        }
        if confidence.is_some_and(|value| value < 0.25) {
            return self.rejected("not_matched", &config);
        }

        let matcher = WakeWordMatcher::from_config(&config);
        let Some(matched) = matcher.matches(transcript) else {
            return self.rejected("not_matched", &config);
        };

        {
            let mut runtime = self.write();
            let now = Instant::now();
            if runtime
                .last_activation_instant
                .is_some_and(|last| now.duration_since(last) < WAKE_COOLDOWN)
            {
                let listening = runtime.listening_until.is_some_and(|until| until > now);
                return WakeOutcome {
                    activated: false,
                    assistant_name: config.assistant_name.clone(),
                    listening,
                    trailing_query: matched.trailing_query.clone(),
                    reason: "cooldown",
                };
            }
            runtime.last_activation_instant = Some(now);
            runtime.last_activation = Some(Utc::now());
            runtime.activations = runtime.activations.saturating_add(1);
            runtime.listening_until =
                Some(now + Duration::from_secs(u64::from(config.listening_window_secs)));
        }

        self.kiosk.set_avatar_state(AvatarState::Listening);
        self.kiosk.set_agent_activity(AgentActivity::Listening);
        self.kiosk.bus().publish_kiosk(KioskEvent::Wake {
            assistant_name: config.assistant_name.clone(),
            trailing_query: matched.trailing_query.clone(),
        });
        tracing::info!(
            assistant = %config.assistant_name,
            score = matched.score,
            trailing = matched.trailing_query.is_some(),
            "wake word activated"
        );

        WakeOutcome {
            activated: true,
            assistant_name: config.assistant_name,
            listening: true,
            trailing_query: matched.trailing_query,
            reason: "activated",
        }
    }

    /// Close the listening window (a request has been submitted).
    pub fn begin_query(&self) {
        let mut runtime = self.write();
        runtime.listening_until = None;
    }

    /// Close the listening window without starting a query (timeout/dismiss).
    pub fn stop_listening(&self) {
        self.begin_query();
        self.kiosk.set_avatar_state(AvatarState::Idle);
        self.kiosk.set_agent_activity(AgentActivity::Idle);
    }

    fn rejected(&self, reason: &'static str, config: &VoiceConfig) -> WakeOutcome {
        WakeOutcome {
            activated: false,
            assistant_name: config.assistant_name.clone(),
            listening: self.is_listening(),
            trailing_query: None,
            reason,
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, VoiceRuntime> {
        self.runtime
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, VoiceRuntime> {
        self.runtime
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;

    fn manager(
        name: &str,
    ) -> (
        VoiceManager,
        Arc<KioskManager>,
        Arc<ConfigManager>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = Arc::new(ConfigManager::load(dir.path()).expect("config"));
        config
            .update(|config| {
                config.voice.assistant_name = name.to_owned();
                Ok(())
            })
            .expect("save voice config");
        let kiosk = Arc::new(KioskManager::new(EventBus::new(16), "Test"));
        (
            VoiceManager::new(Arc::clone(&config), Arc::clone(&kiosk)),
            kiosk,
            config,
            dir,
        )
    }

    #[test]
    fn matching_name_activates_and_opens_window() {
        let (voice, kiosk, _config, _dir) = manager("Aura");
        let outcome = voice.handle_transcript("oye aura enciende la luz", None);
        assert!(outcome.activated);
        assert!(outcome.listening);
        assert_eq!(outcome.trailing_query.as_deref(), Some("enciende la luz"));
        assert!(voice.is_listening());
        assert_eq!(kiosk.snapshot().avatar_state, AvatarState::Listening);
    }

    #[test]
    fn other_transcripts_do_not_activate() {
        let (voice, _kiosk, _config, _dir) = manager("Aura");
        let outcome = voice.handle_transcript("hola que tal", None);
        assert!(!outcome.activated);
        assert_eq!(outcome.reason, "not_matched");
        assert!(!voice.is_listening());
    }

    #[test]
    fn cooldown_prevents_double_activation() {
        let (voice, _kiosk, _config, _dir) = manager("Aura");
        assert!(voice.handle_transcript("oye aura", None).activated);
        let second = voice.handle_transcript("oye aura", None);
        assert!(!second.activated);
        assert_eq!(second.reason, "cooldown");
    }

    #[test]
    fn disabled_wake_word_is_ignored() {
        let (voice, _kiosk, config, _dir) = manager("Aura");
        config
            .update(|config| {
                config.voice.wake_word_enabled = false;
                Ok(())
            })
            .expect("disable wake word");
        let outcome = voice.handle_transcript("oye aura", None);
        assert!(!outcome.activated);
        assert_eq!(outcome.reason, "wake_disabled");
    }

    #[test]
    fn begin_query_closes_window() {
        let (voice, _kiosk, _config, _dir) = manager("Aura");
        assert!(voice.handle_transcript("oye aura", None).activated);
        voice.begin_query();
        assert!(!voice.is_listening());
        assert!(!voice.status().listening);
    }

    #[test]
    fn listener_heartbeat_is_reported_in_status() {
        let (manager, _kiosk, _config, _dir) = manager("Aura");
        assert!(!manager.listener_connected());
        assert!(!manager.status().listener_connected);
        manager.listener_heartbeat();
        assert!(manager.listener_connected());
        assert!(manager.status().listener_connected);
    }

    #[test]
    fn status_reports_name_and_activation_count() {
        let (voice, _kiosk, _config, _dir) = manager("Jarvis");
        assert_eq!(voice.status().assistant_name, "Jarvis");
        voice.handle_transcript("jarvis", None);
        let status = voice.status();
        assert_eq!(status.activations, 1);
        assert!(status.last_activation.is_some());
    }
}
