//! First-boot onboarding coordinator.
//!
//! The wizard has six steps. Each step is persisted as soon as it is completed
//! so a browser refresh or a daemon restart does not lose progress. The final
//! step runs the deployment job and streams progress over `/ws/setup`.

use std::collections::BTreeSet;
use std::sync::Mutex;

use hs_core::config::SystemConfig;

use crate::events::{EventBus, ServerEvent};

/// Number of onboarding steps.
pub const SETUP_STEPS: u8 = 6;

/// Labels shown in the setup progress UI (English technical labels; the UI
/// localizes them by step number).
pub const STEP_LABELS: [&str; 6] = [
    "Device and administrator account",
    "Network, encrypted DNS and DMZ",
    "Audio output and peripherals",
    "Local IoT discovery",
    "AI provider and household profiles",
    "Deploying home server services",
];

/// Public setup status returned by `GET /api/v1/setup/status`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SetupStatus {
    /// Whether the appliance finished onboarding.
    pub initialized: bool,
    /// Device name configured so far (or the default).
    pub device_name: String,
    /// UI language configured so far.
    pub language: String,
    /// Completed step numbers (1..=6).
    pub completed_steps: Vec<u8>,
    /// Whether the deployment job is currently running.
    pub running: bool,
    /// Last setup error, if any.
    pub error: Option<String>,
}

#[derive(Debug, Default)]
struct SetupState {
    completed: BTreeSet<u8>,
    running: bool,
    error: Option<String>,
}

/// Setup coordinator shared by the wizard routes.
#[derive(Debug)]
pub struct SetupCoordinator {
    state: Mutex<SetupState>,
    bus: EventBus,
}

impl SetupCoordinator {
    /// Create a coordinator with no completed steps.
    #[must_use]
    pub fn new(bus: EventBus) -> Self {
        Self {
            state: Mutex::new(SetupState::default()),
            bus,
        }
    }

    /// Build the status payload for the current configuration.
    #[must_use]
    pub fn status(&self, config: &SystemConfig) -> SetupStatus {
        let state = self.lock();
        SetupStatus {
            initialized: config.initialized,
            device_name: config.device.name.clone(),
            language: config.device.language.clone(),
            completed_steps: state.completed.iter().copied().collect(),
            running: state.running,
            error: state.error.clone(),
        }
    }

    /// Whether a step has been completed.
    #[must_use]
    pub fn is_completed(&self, step: u8) -> bool {
        self.lock().completed.contains(&step)
    }

    /// Mark a step as completed and notify the wizard.
    pub fn complete_step(&self, step: u8) {
        {
            let mut state = self.lock();
            state.completed.insert(step);
            state.error = None;
        }
        let label = STEP_LABELS
            .get(usize::from(step.saturating_sub(1)))
            .copied()
            .unwrap_or("Setup step");
        self.bus.publish_server(ServerEvent::SetupProgress {
            step,
            label: label.to_owned(),
            percent: 100,
            done: true,
        });
    }

    /// Publish arbitrary progress for the deployment job.
    pub fn publish_progress(&self, step: u8, label: impl Into<String>, percent: u8, done: bool) {
        self.bus.publish_server(ServerEvent::SetupProgress {
            step,
            label: label.into(),
            percent: percent.min(100),
            done,
        });
    }

    /// Try to start the deployment job. Returns `false` if it is already
    /// running, which makes the endpoint idempotent.
    pub fn try_begin_deploy(&self) -> bool {
        let mut state = self.lock();
        if state.running {
            return false;
        }
        state.running = true;
        state.error = None;
        true
    }

    /// Finish the deployment job.
    pub fn finish_deploy(&self, error: Option<String>) {
        {
            let mut state = self.lock();
            state.running = false;
            state.error = error;
        }
        if self.lock().error.is_none() {
            self.bus.publish_server(ServerEvent::SetupFinished);
        }
    }

    /// Record a setup error and stop the deployment job.
    pub fn fail(&self, message: impl Into<String>) {
        let message = message.into();
        {
            let mut state = self.lock();
            state.running = false;
            state.error = Some(message.clone());
        }
        self.bus.publish_server(ServerEvent::Notification {
            level: "error".to_owned(),
            message,
        });
    }

    /// Reset all progress (used by tests and by a factory reset).
    pub fn reset(&self) {
        let mut state = self.lock();
        *state = SetupState::default();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SetupState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_tracked_and_published() {
        let bus = EventBus::new(16);
        let mut receiver = bus.subscribe_server();
        let coordinator = SetupCoordinator::new(bus);
        assert!(!coordinator.is_completed(1));
        coordinator.complete_step(1);
        assert!(coordinator.is_completed(1));
        let event = receiver.try_recv().unwrap();
        assert!(matches!(
            event,
            ServerEvent::SetupProgress {
                step: 1,
                done: true,
                ..
            }
        ));
    }

    #[test]
    fn deploy_job_is_idempotent() {
        let coordinator = SetupCoordinator::new(EventBus::new(8));
        assert!(coordinator.try_begin_deploy());
        assert!(!coordinator.try_begin_deploy());
        coordinator.finish_deploy(None);
        assert!(coordinator.try_begin_deploy());
    }

    #[test]
    fn status_reflects_configuration() {
        let coordinator = SetupCoordinator::new(EventBus::new(8));
        coordinator.complete_step(2);
        let mut config = SystemConfig::bootstrap();
        config.device.name = "Salón".to_owned();
        let status = coordinator.status(&config);
        assert_eq!(status.device_name, "Salón");
        assert_eq!(status.completed_steps, vec![2]);
        assert!(!status.initialized);
    }
}
