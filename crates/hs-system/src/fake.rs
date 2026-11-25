//! Deterministic [`FakeHardware`] used by daemon development mode and tests.
//!
//! The fake exposes public fields for the sensor readings and records every
//! mutating hardware call so tests can assert on them without root, sysfs,
//! PipeWire or systemd.

use std::sync::Mutex;

use hs_core::CoreResult;

use crate::{AudioSink, BatteryStatus, Hardware, PowerPolicyReport, CHARGE_LIMIT_RANGE};

/// Configurable in-memory hardware implementation.
#[derive(Debug)]
pub struct FakeHardware {
    /// Whether the fake reports itself as a Linux host.
    pub linux: bool,
    /// Audio sinks returned by [`Hardware::list_audio_sinks`].
    pub audio_sinks: Vec<AudioSink>,
    /// Default audio sink id, when explicitly set.
    pub default_sink: Option<String>,
    /// Battery status returned by [`Hardware::read_battery`].
    pub battery: Option<BatteryStatus>,
    /// Temperature returned by [`Hardware::read_temperature`].
    pub temperature_c: Option<f32>,
    /// Camera device nodes returned by [`Hardware::camera_devices`].
    pub camera_nodes: Vec<String>,
    /// Touch device nodes returned by [`Hardware::touch_devices`].
    pub touch_nodes: Vec<String>,
    /// Value reported by [`Hardware::power_policy`] for the lid switch.
    pub lid_switch_ignored: bool,
    /// Value reported by [`Hardware::power_policy`] for the charge limit.
    pub charge_limit: Option<u8>,
    recorded: Mutex<RecordedCalls>,
}

#[derive(Default)]
struct RecordedCalls {
    default_sinks: Vec<String>,
    volumes: Vec<u8>,
    lid_switch_applications: usize,
    charge_limits: Vec<u8>,
}

impl std::fmt::Debug for RecordedCalls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordedCalls")
            .field("default_sinks", &self.default_sinks)
            .field("volumes", &self.volumes)
            .field("lid_switch_applications", &self.lid_switch_applications)
            .field("charge_limits", &self.charge_limits)
            .finish()
    }
}

impl Default for FakeHardware {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeHardware {
    /// Create an empty fake. `linux` matches the build host.
    #[must_use]
    pub fn new() -> Self {
        Self {
            linux: cfg!(target_os = "linux"),
            audio_sinks: Vec::new(),
            default_sink: None,
            battery: None,
            temperature_c: None,
            camera_nodes: Vec::new(),
            touch_nodes: Vec::new(),
            lid_switch_ignored: false,
            charge_limit: None,
            recorded: Mutex::new(RecordedCalls::default()),
        }
    }

    /// Create a fake that reports itself as a Linux appliance.
    #[must_use]
    pub fn linux() -> Self {
        Self {
            linux: true,
            ..Self::new()
        }
    }

    /// Install a battery reading.
    #[must_use]
    pub fn with_battery(mut self, battery: BatteryStatus) -> Self {
        self.battery = Some(battery);
        self
    }

    /// Install a temperature reading.
    #[must_use]
    pub fn with_temperature(mut self, celsius: f32) -> Self {
        self.temperature_c = Some(celsius);
        self
    }

    /// Install the audio sink list.
    #[must_use]
    pub fn with_audio(mut self, sinks: Vec<AudioSink>) -> Self {
        self.audio_sinks = sinks;
        self
    }

    /// Install camera device nodes.
    #[must_use]
    pub fn with_cameras(mut self, nodes: Vec<String>) -> Self {
        self.camera_nodes = nodes;
        self
    }

    /// Install touch device nodes.
    #[must_use]
    pub fn with_touch_devices(mut self, nodes: Vec<String>) -> Self {
        self.touch_nodes = nodes;
        self
    }

    /// Install the state reported by [`Hardware::power_policy`].
    #[must_use]
    pub fn with_power_policy(mut self, lid_switch_ignored: bool, charge_limit: Option<u8>) -> Self {
        self.lid_switch_ignored = lid_switch_ignored;
        self.charge_limit = charge_limit;
        self
    }

    /// Sink ids passed to [`Hardware::set_default_sink`], in order.
    #[must_use]
    pub fn recorded_default_sinks(&self) -> Vec<String> {
        self.recorded
            .lock()
            .map(|calls| calls.default_sinks.clone())
            .unwrap_or_default()
    }

    /// Volumes passed to [`Hardware::set_volume`], in order.
    #[must_use]
    pub fn recorded_volumes(&self) -> Vec<u8> {
        self.recorded
            .lock()
            .map(|calls| calls.volumes.clone())
            .unwrap_or_default()
    }

    /// Number of [`Hardware::apply_lid_switch_ignore`] calls.
    #[must_use]
    pub fn lid_switch_applications(&self) -> usize {
        self.recorded
            .lock()
            .map(|calls| calls.lid_switch_applications)
            .unwrap_or_default()
    }

    /// Charge limits passed to [`Hardware::apply_charge_limit`], in order.
    #[must_use]
    pub fn recorded_charge_limits(&self) -> Vec<u8> {
        self.recorded
            .lock()
            .map(|calls| calls.charge_limits.clone())
            .unwrap_or_default()
    }
}

impl Hardware for FakeHardware {
    fn list_audio_sinks(&self) -> Vec<AudioSink> {
        self.audio_sinks.clone()
    }

    fn default_audio_sink(&self) -> Option<String> {
        self.default_sink.clone().or_else(|| {
            self.audio_sinks
                .iter()
                .find(|sink| sink.is_default)
                .map(|sink| sink.id.clone())
        })
    }

    fn set_default_sink(&self, id: &str) -> CoreResult<()> {
        if id.trim().is_empty() {
            return Err(hs_core::CoreError::validation(
                "sink",
                "sink id must not be empty",
            ));
        }
        if let Ok(mut calls) = self.recorded.lock() {
            calls.default_sinks.push(id.to_owned());
        }
        Ok(())
    }

    fn set_volume(&self, percent: u8) -> CoreResult<()> {
        if percent > 100 {
            return Err(hs_core::CoreError::validation(
                "volume",
                "must be between 0 and 100",
            ));
        }
        if let Ok(mut calls) = self.recorded.lock() {
            calls.volumes.push(percent);
        }
        Ok(())
    }

    fn read_battery(&self) -> Option<BatteryStatus> {
        self.battery.clone()
    }

    fn read_temperature(&self) -> Option<f32> {
        self.temperature_c
    }

    fn camera_devices(&self) -> Vec<String> {
        self.camera_nodes.clone()
    }

    fn touch_devices(&self) -> Vec<String> {
        self.touch_nodes.clone()
    }

    fn apply_lid_switch_ignore(&self) -> CoreResult<()> {
        if let Ok(mut calls) = self.recorded.lock() {
            calls.lid_switch_applications += 1;
        }
        Ok(())
    }

    fn apply_charge_limit(&self, percent: u8) -> CoreResult<()> {
        if !CHARGE_LIMIT_RANGE.contains(&percent) {
            return Err(hs_core::CoreError::validation(
                "charge_limit",
                "must be between 60 and 80 percent",
            ));
        }
        if let Ok(mut calls) = self.recorded.lock() {
            calls.charge_limits.push(percent);
        }
        Ok(())
    }

    fn is_linux(&self) -> bool {
        self.linux
    }

    fn power_policy(&self) -> PowerPolicyReport {
        PowerPolicyReport {
            lid_switch_ignored: self.lid_switch_ignored,
            charge_limit: self.charge_limit,
            warnings: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink(id: &str, is_default: bool) -> AudioSink {
        AudioSink {
            id: id.to_owned(),
            name: format!("sink-{id}"),
            description: format!("Sink {id}"),
            is_default,
        }
    }

    #[test]
    fn builders_populate_state() {
        let hardware = FakeHardware::linux()
            .with_audio(vec![sink("41", true), sink("45", false)])
            .with_temperature(43.0)
            .with_cameras(vec!["video0".to_owned()])
            .with_touch_devices(vec!["event3".to_owned()])
            .with_power_policy(true, Some(70));
        assert!(hardware.is_linux());
        assert_eq!(hardware.list_audio_sinks().len(), 2);
        assert_eq!(hardware.default_audio_sink(), Some("41".to_owned()));
        assert_eq!(hardware.read_temperature(), Some(43.0));
        assert_eq!(hardware.camera_devices(), vec!["video0".to_owned()]);
        assert_eq!(hardware.touch_devices(), vec!["event3".to_owned()]);
        assert_eq!(
            hardware.power_policy(),
            PowerPolicyReport {
                lid_switch_ignored: true,
                charge_limit: Some(70),
                warnings: Vec::new(),
            }
        );
    }

    #[test]
    fn mutating_calls_are_recorded() {
        let hardware = FakeHardware::linux();
        hardware.set_default_sink("41").expect("set default sink");
        hardware.set_volume(60).expect("set volume");
        hardware.apply_lid_switch_ignore().expect("ignore lid");
        hardware.apply_charge_limit(80).expect("charge limit");
        assert_eq!(hardware.recorded_default_sinks(), vec!["41".to_owned()]);
        assert_eq!(hardware.recorded_volumes(), vec![60]);
        assert_eq!(hardware.lid_switch_applications(), 1);
        assert_eq!(hardware.recorded_charge_limits(), vec![80]);
    }

    #[test]
    fn invalid_values_are_rejected_and_not_recorded() {
        let hardware = FakeHardware::new();
        assert!(hardware.set_default_sink("  ").is_err());
        assert!(hardware.set_volume(101).is_err());
        assert!(hardware.apply_charge_limit(59).is_err());
        assert!(hardware.recorded_default_sinks().is_empty());
        assert!(hardware.recorded_volumes().is_empty());
        assert!(hardware.recorded_charge_limits().is_empty());
    }
}
