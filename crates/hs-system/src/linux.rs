//! Linux hardware adapter: sysfs, PipeWire (`wpctl`) and systemd (`systemctl`).
//!
//! Everything here is defensive: missing files or commands degrade to `None` /
//! empty lists instead of panicking, and test-only constructors allow the sysfs
//! and device roots to be pointed at fixture directories.

use std::path::{Path, PathBuf};

use hs_core::CoreResult;

use crate::{
    AudioSink, BatteryStatus, Hardware, PowerPolicyReport, SystemError, SystemResult,
    CHARGE_LIMIT_RANGE,
};

/// Default sysfs mount point.
pub const DEFAULT_SYSFS_ROOT: &str = "/sys";
/// Default device node directory.
pub const DEFAULT_DEVICE_ROOT: &str = "/dev";
/// Logind drop-in written by the appliance to ignore the lid switch.
pub const DEFAULT_LOGIND_DROPIN: &str = "/etc/systemd/logind.conf.d/90-homeserver.conf";

/// Battery charge threshold attribute names, in preference order.
const CHARGE_THRESHOLD_FILES: [&str; 2] = ["charge_control_end_threshold", "charge_stop_threshold"];

/// Production hardware adapter for Debian/Ubuntu appliances.
#[derive(Debug, Clone)]
pub struct LinuxHardware {
    sysfs_root: PathBuf,
    device_root: PathBuf,
    logind_dropin: PathBuf,
}

impl Default for LinuxHardware {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxHardware {
    /// Create an adapter rooted at the production `/sys`, `/dev` and logind
    /// paths.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sysfs_root: PathBuf::from(DEFAULT_SYSFS_ROOT),
            device_root: PathBuf::from(DEFAULT_DEVICE_ROOT),
            logind_dropin: PathBuf::from(DEFAULT_LOGIND_DROPIN),
        }
    }

    /// Create an adapter reading from a fake sysfs tree (tests).
    #[must_use]
    pub fn with_sysfs_root(sysfs_root: PathBuf) -> Self {
        Self {
            sysfs_root,
            device_root: PathBuf::from(DEFAULT_DEVICE_ROOT),
            logind_dropin: PathBuf::from(DEFAULT_LOGIND_DROPIN),
        }
    }

    /// Override the device node directory (tests).
    #[must_use]
    pub fn with_device_root(mut self, device_root: PathBuf) -> Self {
        self.device_root = device_root;
        self
    }

    /// Override the logind drop-in path (tests).
    #[must_use]
    pub fn with_logind_dropin(mut self, logind_dropin: PathBuf) -> Self {
        self.logind_dropin = logind_dropin;
        self
    }

    /// List directories under `sysfs/class/power_supply` whose type is
    /// `Battery`, sorted for determinism.
    fn battery_directories(&self) -> Vec<PathBuf> {
        let supply_dir = self.sysfs_root.join("class/power_supply");
        let Ok(entries) = std::fs::read_dir(&supply_dir) else {
            return Vec::new();
        };
        let mut batteries: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .filter(|path| {
                read_trimmed(path.join("type"))
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("battery"))
            })
            .collect();
        batteries.sort();
        batteries
    }

    /// Whether a `sysfs/class/input/<node>/device/name` advertises touch input.
    fn input_is_touch(&self, node: &str) -> bool {
        let name_path = self
            .sysfs_root
            .join("class")
            .join("input")
            .join(node)
            .join("device")
            .join("name");
        read_trimmed(&name_path)
            .map(|name| name.to_ascii_lowercase().contains("touch"))
            .unwrap_or(false)
    }
}

impl Hardware for LinuxHardware {
    fn list_audio_sinks(&self) -> Vec<AudioSink> {
        if !self.is_linux() {
            return Vec::new();
        }
        match run_command("wpctl", &["status"]) {
            Ok(status) => parse_wpctl_sinks(&status),
            Err(err) => {
                tracing::debug!(error = %err, "cannot list PipeWire sinks");
                Vec::new()
            }
        }
    }

    fn default_audio_sink(&self) -> Option<String> {
        self.list_audio_sinks()
            .into_iter()
            .find(|sink| sink.is_default)
            .map(|sink| sink.id)
    }

    fn set_default_sink(&self, id: &str) -> CoreResult<()> {
        if !self.is_linux() {
            return Err(
                SystemError::unavailable("audio routing requires Linux with PipeWire").into(),
            );
        }
        if id.trim().is_empty() {
            return Err(hs_core::CoreError::validation(
                "sink",
                "sink id must not be empty",
            ));
        }
        run_command("wpctl", &["set-default", id])
            .map(|_| ())
            .map_err(Into::into)
    }

    fn set_volume(&self, percent: u8) -> CoreResult<()> {
        if percent > 100 {
            return Err(hs_core::CoreError::validation(
                "volume",
                "must be between 0 and 100",
            ));
        }
        if !self.is_linux() {
            return Err(
                SystemError::unavailable("audio routing requires Linux with PipeWire").into(),
            );
        }
        let volume = format!("{percent}%");
        run_command("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &volume])
            .map(|_| ())
            .map_err(Into::into)
    }

    fn read_battery(&self) -> Option<BatteryStatus> {
        self.battery_directories()
            .iter()
            .find_map(|dir| parse_battery_directory(dir))
    }

    fn read_temperature(&self) -> Option<f32> {
        thermal_zone_temperature(&self.sysfs_root).or_else(|| hwmon_temperature(&self.sysfs_root))
    }

    fn camera_devices(&self) -> Vec<String> {
        scan_device_nodes(&self.device_root, "video")
    }

    fn touch_devices(&self) -> Vec<String> {
        let input_dir = self.device_root.join("input");
        scan_device_nodes(&input_dir, "event")
            .into_iter()
            .filter(|node| self.input_is_touch(node))
            .collect()
    }

    fn apply_lid_switch_ignore(&self) -> CoreResult<()> {
        if let Some(parent) = self.logind_dropin.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(&self.logind_dropin, LID_SWITCH_DROPIN)?;
        tracing::info!(
            path = %self.logind_dropin.display(),
            "wrote systemd-logind lid switch policy"
        );
        Ok(())
    }

    fn apply_charge_limit(&self, percent: u8) -> CoreResult<()> {
        if !CHARGE_LIMIT_RANGE.contains(&percent) {
            return Err(hs_core::CoreError::validation(
                "charge_limit",
                "must be between 60 and 80 percent",
            ));
        }
        if !self.is_linux() {
            return Err(
                SystemError::unavailable("battery charge limits require a Linux kernel").into(),
            );
        }
        let batteries = self.battery_directories();
        if batteries.is_empty() {
            return Err(SystemError::unavailable("no battery was detected").into());
        }
        for battery in &batteries {
            if write_charge_limit(battery, percent)? {
                return Ok(());
            }
        }
        Err(SystemError::unavailable("no battery exposes a charge threshold attribute").into())
    }

    fn is_linux(&self) -> bool {
        cfg!(target_os = "linux")
    }

    fn power_policy(&self) -> PowerPolicyReport {
        let lid_switch_ignored = std::fs::read_to_string(&self.logind_dropin)
            .map(|contents| {
                contents
                    .lines()
                    .any(|line| line.trim().eq_ignore_ascii_case("HandleLidSwitch=ignore"))
            })
            .unwrap_or(false);
        let charge_limit = self
            .battery_directories()
            .into_iter()
            .find_map(|battery| read_u8(battery.join("charge_control_end_threshold")))
            .or_else(|| {
                self.battery_directories()
                    .into_iter()
                    .find_map(|battery| read_u8(battery.join("charge_stop_threshold")))
            });
        PowerPolicyReport {
            lid_switch_ignored,
            charge_limit,
            warnings: Vec::new(),
        }
    }
}

/// Content of the systemd-logind drop-in that ignores the lid switch.
const LID_SWITCH_DROPIN: &str = "\
# Managed by HomeServer Assistant. Do not edit manually.
[Login]
HandleLidSwitch=ignore
HandleLidSwitchExternalPower=ignore
HandleLidSwitchDocked=ignore
";

/// Read a small text file and trim surrounding whitespace.
pub(crate) fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|contents| contents.trim().to_owned())
}

/// Read an unsigned integer from a sysfs attribute.
pub(crate) fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

/// Read an unsigned 8 bit integer from a sysfs attribute.
fn read_u8(path: impl AsRef<Path>) -> Option<u8> {
    read_u64(path).and_then(|value| u8::try_from(value).ok())
}

/// Parse a `power_supply` directory into a [`BatteryStatus`].
///
/// `capacity` is preferred; when it is missing the percentage is derived from
/// the charge counters. Remaining time is only estimated while discharging.
pub(crate) fn parse_battery_directory(dir: &Path) -> Option<BatteryStatus> {
    let status = read_trimmed(dir.join("status")).unwrap_or_default();
    let percent = read_u64(dir.join("capacity"))
        .map(|value| value.min(100) as u8)
        .or_else(|| charge_percent(dir))?;
    let charging = status.eq_ignore_ascii_case("charging");
    let time_remaining_min = if status.eq_ignore_ascii_case("discharging") {
        remaining_minutes(dir)
    } else {
        None
    };
    Some(BatteryStatus {
        present: true,
        percent,
        charging,
        time_remaining_min,
    })
}

/// Compute the charge percentage from `charge_now`/`charge_full`.
fn charge_percent(dir: &Path) -> Option<u8> {
    let now = read_u64(dir.join("charge_now"))?;
    let full =
        read_u64(dir.join("charge_full")).or_else(|| read_u64(dir.join("charge_full_design")))?;
    if full == 0 {
        return None;
    }
    Some(((now.saturating_mul(100) / full).min(100)) as u8)
}

/// Estimate remaining discharge time in minutes.
fn remaining_minutes(dir: &Path) -> Option<u32> {
    let energy_now = read_u64(dir.join("energy_now"));
    let power_now = read_u64(dir.join("power_now"));
    if let (Some(energy_now), Some(power_now)) = (energy_now, power_now) {
        let minutes = energy_now.saturating_mul(60).checked_div(power_now)?;
        return Some(minutes.min(u64::from(u32::MAX)) as u32);
    }
    let charge_now = read_u64(dir.join("charge_now"))?;
    let current_now = read_u64(dir.join("current_now"))?;
    let minutes = charge_now.saturating_mul(60).checked_div(current_now)?;
    Some(minutes.min(u64::from(u32::MAX)) as u32)
}

/// Read the temperature from the first thermal zone, preferring CPU sensors.
pub(crate) fn thermal_zone_temperature(sysfs_root: &Path) -> Option<f32> {
    let thermal_root = sysfs_root.join("class").join("thermal");
    let Ok(entries) = std::fs::read_dir(&thermal_root) else {
        return None;
    };
    let mut zones: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("thermal_zone"))
        })
        .collect();
    zones.sort();
    let mut fallback = None;
    for zone in zones {
        let Some(millidegrees) = read_u64(zone.join("temp")) else {
            continue;
        };
        if millidegrees == 0 {
            continue;
        }
        let celsius = millidegrees as f32 / 1000.0;
        let zone_type = read_trimmed(zone.join("type")).unwrap_or_default();
        let zone_type = zone_type.to_ascii_lowercase();
        if zone_type.contains("x86_pkg_temp")
            || zone_type.contains("cpu")
            || zone_type.contains("soc")
        {
            return Some(celsius);
        }
        fallback.get_or_insert(celsius);
    }
    fallback
}

/// Fallback temperature source: `hwmon` `temp1_input` attributes.
pub(crate) fn hwmon_temperature(sysfs_root: &Path) -> Option<f32> {
    let hwmon_root = sysfs_root.join("class").join("hwmon");
    let Ok(entries) = std::fs::read_dir(&hwmon_root) else {
        return None;
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs.iter().find_map(|dir| {
        let millidegrees = read_u64(dir.join("temp1_input"))?;
        if millidegrees == 0 {
            None
        } else {
            Some(millidegrees as f32 / 1000.0)
        }
    })
}

/// List device nodes in `dir` whose name is `prefix` followed by digits.
pub(crate) fn scan_device_nodes(dir: &Path, prefix: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut nodes: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            name.strip_prefix(prefix).is_some_and(|suffix| {
                !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
            })
        })
        .collect();
    nodes.sort();
    nodes
}

/// Parse `wpctl status` output and extract the audio sink list.
///
/// The parser only looks at the `Sinks:` block and tolerates the Unicode tree
/// glyphs printed by `wpctl`.
pub(crate) fn parse_wpctl_sinks(status: &str) -> Vec<AudioSink> {
    let mut sinks = Vec::new();
    let mut in_sinks = false;
    for line in status.lines() {
        let trimmed = line.trim();
        if trimmed.ends_with("Sinks:") {
            in_sinks = true;
            continue;
        }
        if !in_sinks {
            continue;
        }
        // A new "Sources:"/"Filters:"/"Streams:" block ends the sink list.
        if trimmed.ends_with(':') && !trimmed.contains(". ") {
            break;
        }
        let is_default = line.contains('*');
        let cleaned: String = line
            .chars()
            .filter(|character| !matches!(character, '│' | '├' | '└' | '─' | '*'))
            .collect();
        let cleaned = cleaned.trim();
        let Some((id, rest)) = cleaned.split_once(". ") else {
            continue;
        };
        let id = id.trim();
        if id.is_empty() || !id.chars().all(|character| character.is_ascii_digit()) {
            continue;
        }
        let name = rest.split('[').next().unwrap_or(rest).trim().to_owned();
        if name.is_empty() {
            continue;
        }
        sinks.push(AudioSink {
            id: id.to_owned(),
            description: name.clone(),
            name,
            is_default,
        });
    }
    sinks
}

/// Write the charge limit to a battery directory.
///
/// Returns `Ok(true)` when an attribute was written and `Ok(false)` when the
/// battery exposes no threshold attribute.
pub(crate) fn write_charge_limit(battery_dir: &Path, percent: u8) -> SystemResult<bool> {
    for attribute in CHARGE_THRESHOLD_FILES {
        let path = battery_dir.join(attribute);
        if path.is_file() {
            std::fs::write(&path, format!("{percent}\n"))?;
            tracing::info!(
                attribute = %path.display(),
                percent,
                "applied battery charge limit"
            );
            return Ok(true);
        }
    }
    Ok(false)
}

/// Run a command and capture its stdout, mapping failures to [`SystemError`].
fn run_command(program: &str, args: &[&str]) -> SystemResult<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|err| {
            SystemError::unavailable(format!("cannot run `{program}` on this host: {err}"))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(SystemError::command(format!(
            "`{program} {}` failed: {stderr}",
            args.join(" ")
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{BatteryStatus, Hardware};

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture directory");
        }
        std::fs::write(path, contents).expect("write fixture file");
    }

    fn battery_fixture(root: &Path, name: &str) -> PathBuf {
        let dir = root.join("class").join("power_supply").join(name);
        write_file(&dir.join("type"), "Battery\n");
        dir
    }

    #[test]
    fn battery_discharging_fixture() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let battery = battery_fixture(root, "BAT0");
        write_file(&battery.join("status"), "Discharging\n");
        write_file(&battery.join("capacity"), "72\n");
        write_file(&battery.join("energy_now"), "48000000\n");
        write_file(&battery.join("power_now"), "12000000\n");

        let hardware = LinuxHardware::with_sysfs_root(root.to_path_buf());
        let status = hardware.read_battery().expect("battery present");
        assert_eq!(
            status,
            BatteryStatus {
                present: true,
                percent: 72,
                charging: false,
                time_remaining_min: Some(240),
            }
        );
    }

    #[test]
    fn battery_charging_fixture_uses_charge_counters() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let battery = battery_fixture(root, "BAT1");
        write_file(&battery.join("status"), "Charging\n");
        write_file(&battery.join("charge_now"), "2500000\n");
        write_file(&battery.join("charge_full"), "5000000\n");

        let hardware = LinuxHardware::with_sysfs_root(root.to_path_buf());
        let status = hardware.read_battery().expect("battery present");
        assert_eq!(status.percent, 50);
        assert!(status.charging);
        assert_eq!(status.time_remaining_min, None);
    }

    #[test]
    fn no_battery_returns_none() {
        let temp = tempfile::tempdir().expect("tempdir");
        let hardware = LinuxHardware::with_sysfs_root(temp.path().to_path_buf());
        assert_eq!(hardware.read_battery(), None);
        assert_eq!(hardware.read_temperature(), None);
    }

    #[test]
    fn non_battery_power_supply_is_ignored() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let supply = root.join("class/power_supply/AC");
        write_file(&supply.join("type"), "Mains\n");
        write_file(&supply.join("online"), "1\n");
        let hardware = LinuxHardware::with_sysfs_root(root.to_path_buf());
        assert_eq!(hardware.read_battery(), None);
    }

    #[test]
    fn thermal_zone_fixture_is_parsed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_file(&root.join("class/thermal/thermal_zone0/type"), "acpitz\n");
        write_file(&root.join("class/thermal/thermal_zone0/temp"), "45000\n");
        let hardware = LinuxHardware::with_sysfs_root(root.to_path_buf());
        assert_eq!(hardware.read_temperature(), Some(45.0));
    }

    #[test]
    fn thermal_zone_prefers_cpu_sensor() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_file(&root.join("class/thermal/thermal_zone0/type"), "acpitz\n");
        write_file(&root.join("class/thermal/thermal_zone0/temp"), "32000\n");
        write_file(
            &root.join("class/thermal/thermal_zone1/type"),
            "x86_pkg_temp\n",
        );
        write_file(&root.join("class/thermal/thermal_zone1/temp"), "52000\n");
        assert_eq!(thermal_zone_temperature(root), Some(52.0));
    }

    #[test]
    fn hwmon_temperature_fallback() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_file(&root.join("class/hwmon/hwmon0/temp1_input"), "41500\n");
        assert_eq!(hwmon_temperature(root), Some(41.5));
    }

    #[test]
    fn charge_limit_write_uses_threshold_attribute() {
        let temp = tempfile::tempdir().expect("tempdir");
        let battery = temp.path().join("BAT0");
        write_file(&battery.join("charge_control_end_threshold"), "100\n");
        assert!(write_charge_limit(&battery, 80).expect("write threshold"));
        let written = std::fs::read_to_string(battery.join("charge_control_end_threshold"))
            .expect("read threshold");
        assert_eq!(written.trim(), "80");

        let empty = temp.path().join("BAT1");
        write_file(&empty.join("status"), "Charging\n");
        assert!(!write_charge_limit(&empty, 70).expect("no threshold"));
    }

    #[test]
    fn apply_charge_limit_validates_range() {
        let hardware = LinuxHardware::new();
        for invalid in [0u8, 20, 59, 81, 100] {
            assert!(matches!(
                hardware.apply_charge_limit(invalid),
                Err(hs_core::CoreError::Validation { .. })
            ));
        }
    }

    #[test]
    fn lid_switch_dropin_is_written() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dropin = temp.path().join("logind.conf.d/90-homeserver.conf");
        let hardware = LinuxHardware::new().with_logind_dropin(dropin.clone());
        hardware.apply_lid_switch_ignore().expect("write drop-in");
        let contents = std::fs::read_to_string(&dropin).expect("read drop-in");
        assert!(contents.contains("[Login]"));
        assert!(contents.contains("HandleLidSwitch=ignore"));
        assert!(hardware.power_policy().lid_switch_ignored);
    }

    #[test]
    fn power_policy_reads_threshold_and_dropin() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("sys");
        let battery = battery_fixture(&root, "BAT0");
        write_file(&battery.join("status"), "Charging\n");
        write_file(&battery.join("capacity"), "60\n");
        write_file(&battery.join("charge_control_end_threshold"), "75\n");
        let dropin = temp.path().join("90-homeserver.conf");
        write_file(&dropin, "[Login]\nHandleLidSwitch=ignore\n");

        let hardware = LinuxHardware::with_sysfs_root(root).with_logind_dropin(dropin);
        let policy = hardware.power_policy();
        assert!(policy.lid_switch_ignored);
        assert_eq!(policy.charge_limit, Some(75));
    }

    #[test]
    fn wpctl_output_parsing() {
        let status = "\
Audio
 ├─ Devices:
 │      40. Built-in Audio                      [vol: 0.80]
 ├─ Sinks:
 │  *   41. Built-in Audio Analog Stereo        [vol: 1.00]
 │      45. HDMI Audio Output                   [vol: 0.60]
 ├─ Sources:
 │      52. Built-in Audio Analog Stereo        [vol: 1.00]
";
        let sinks = parse_wpctl_sinks(status);
        assert_eq!(sinks.len(), 2);
        assert_eq!(sinks[0].id, "41");
        assert_eq!(sinks[0].name, "Built-in Audio Analog Stereo");
        assert!(sinks[0].is_default);
        assert_eq!(sinks[1].id, "45");
        assert!(!sinks[1].is_default);
    }

    #[test]
    fn wpctl_output_without_sinks_is_empty() {
        assert!(parse_wpctl_sinks("Audio\n ├─ Sinks:\n ├─ Sources:\n").is_empty());
        assert!(parse_wpctl_sinks("").is_empty());
    }

    #[test]
    fn device_node_scanning_and_touch_detection() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("sys");
        let devices = temp.path().join("dev");
        write_file(&devices.join("video0"), "");
        write_file(&devices.join("video1"), "");
        write_file(&devices.join("video-not-a-node"), "");
        write_file(&devices.join("input/event0"), "");
        write_file(&devices.join("input/event1"), "");
        write_file(
            &root.join("class/input/event0/device/name"),
            "ELAN Touchscreen\n",
        );
        write_file(
            &root.join("class/input/event1/device/name"),
            "AT Translated Set 2 keyboard\n",
        );

        let hardware = LinuxHardware::with_sysfs_root(root).with_device_root(devices);
        assert_eq!(
            hardware.camera_devices(),
            vec!["video0".to_owned(), "video1".to_owned()]
        );
        assert_eq!(hardware.touch_devices(), vec!["event0".to_owned()]);
    }

    #[test]
    fn default_audio_sink_requires_linux() {
        let hardware = LinuxHardware::new();
        if !hardware.is_linux() {
            assert!(hardware.list_audio_sinks().is_empty());
            assert_eq!(hardware.default_audio_sink(), None);
            assert!(hardware.set_volume(50).is_err());
            assert!(hardware.set_default_sink("41").is_err());
        }
        // Range validation happens before the platform check.
        assert!(matches!(
            hardware.set_volume(150),
            Err(hs_core::CoreError::Validation { .. })
        ));
    }
}
