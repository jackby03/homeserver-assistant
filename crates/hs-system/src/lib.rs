//! Host hardware control and metrics for the HomeServer Assistant appliance.
//!
//! The crate is split between [`SystemService`], which orchestrates metrics and
//! policy operations, and the [`Hardware`] port, which abstracts the physical
//! machine. [`linux::LinuxHardware`] is the production implementation (sysfs,
//! `wpctl` and `systemctl`); [`fake::FakeHardware`] is a deterministic fake used
//! by the daemon development mode and by tests.
//!
//! Everything is read through sysfs or plain command invocations so the
//! appliance works on a minimal Debian/Ubuntu image without extra daemons. All
//! operations are no-ops or return `Unavailable`-style errors on non-Linux
//! development hosts; nothing in this crate ever panics on a missing device.
//!
//! # Example
//! ```
//! use std::sync::Arc;
//!
//! use hs_system::{FakeHardware, Hardware, SystemService};
//!
//! # tokio::runtime::Runtime::new().unwrap().block_on(async {
//! let hardware = Arc::new(FakeHardware::linux().with_temperature(41.5));
//! let service = SystemService::with_hardware(hardware.clone());
//! let overview = service.overview().await.expect("host metrics");
//! assert!(overview.cpu.cores >= 1);
//! assert_eq!(overview.temperature_c, Some(41.5));
//! assert!(hardware.is_linux());
//! # });
//! ```

#![deny(missing_docs)]

pub mod fake;
pub mod linux;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sysinfo::{Disks, Networks, System};

pub use fake::FakeHardware;
pub use linux::LinuxHardware;

/// Host services probed by [`SystemService::overview`].
pub const SERVICE_UNITS: [&str; 4] = ["hs-daemon", "hs-vision", "incus", "caddy"];

/// Valid range for the battery charge limit policy (percent).
pub const CHARGE_LIMIT_RANGE: std::ops::RangeInclusive<u8> = 60..=80;

/// Errors produced by the host system layer.
#[derive(Debug, thiserror::Error)]
pub enum SystemError {
    /// The requested hardware feature does not exist on this host.
    #[error("hardware feature unavailable: {0}")]
    Unavailable(String),

    /// An external command (`wpctl`, `systemctl`, ...) failed.
    #[error("command failed: {0}")]
    Command(String),

    /// Filesystem access failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A sysfs/procfs value could not be parsed.
    #[error("failed to parse host value: {0}")]
    Parse(String),

    /// A caller supplied value is invalid.
    #[error("invalid value for `{field}`: {message}")]
    Invalid {
        /// Offending field name.
        field: &'static str,
        /// Human readable explanation.
        message: String,
    },
}

impl SystemError {
    /// Build an [`SystemError::Unavailable`] error.
    #[must_use]
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable(message.into())
    }

    /// Build a [`SystemError::Command`] error.
    #[must_use]
    pub fn command(message: impl Into<String>) -> Self {
        Self::Command(message.into())
    }

    /// Build a [`SystemError::Parse`] error.
    #[must_use]
    pub fn parse(message: impl Into<String>) -> Self {
        Self::Parse(message.into())
    }
}

/// Convenient result alias for internal host-system helpers.
pub type SystemResult<T> = Result<T, SystemError>;

impl From<SystemError> for hs_core::CoreError {
    fn from(err: SystemError) -> Self {
        match err {
            SystemError::Invalid { field, message } => {
                hs_core::CoreError::validation(field, message)
            }
            SystemError::Unavailable(_) | SystemError::Command(_) | SystemError::Parse(_) => {
                hs_core::CoreError::Internal(err.to_string())
            }
            SystemError::Io(error) => hs_core::CoreError::Io(error),
        }
    }
}

// ---------------------------------------------------------------------------
// Metric types (serialized by the HTTP API, consumed by the UI)
// ---------------------------------------------------------------------------

/// Snapshot of the appliance host: metrics, devices and service health.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Overview {
    /// Hostname of the appliance.
    pub hostname: String,
    /// Human readable operating system name (e.g. `Debian GNU/Linux 13`).
    pub os: String,
    /// Kernel version.
    pub kernel: String,
    /// Uptime in seconds.
    pub uptime_secs: u64,
    /// CPU usage and load averages.
    pub cpu: CpuStats,
    /// Physical memory and swap usage.
    pub memory: MemoryStats,
    /// Mounted filesystems with meaningful capacity.
    pub disks: Vec<DiskUsage>,
    /// SoC/CPU temperature in Celsius, when a sensor is present.
    pub temperature_c: Option<f32>,
    /// Battery status on laptops / UPS, when present.
    pub battery: Option<BatteryStatus>,
    /// Network interfaces with addresses and traffic counters.
    pub network: Vec<NetworkInterfaceInfo>,
    /// Health of the appliance systemd units.
    pub services: Vec<ServiceHealth>,
    /// Timestamp when the snapshot was produced.
    pub generated_at: DateTime<Utc>,
}

/// CPU statistics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CpuStats {
    /// Global CPU usage percentage (0-100).
    pub usage_percent: f32,
    /// Number of logical cores.
    pub cores: usize,
    /// 1 minute load average.
    pub load_1: f32,
    /// 5 minute load average.
    pub load_5: f32,
    /// 15 minute load average.
    pub load_15: f32,
}

/// Memory statistics in mebibytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryStats {
    /// Total physical memory in MiB.
    pub total_mb: u64,
    /// Used physical memory in MiB.
    pub used_mb: u64,
    /// Available physical memory in MiB.
    pub available_mb: u64,
    /// Total swap in MiB.
    pub swap_total_mb: u64,
    /// Used swap in MiB.
    pub swap_used_mb: u64,
}

/// Filesystem type detected for a mounted volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskKind {
    /// ZFS dataset or pool.
    Zfs,
    /// Btrfs filesystem.
    Btrfs,
    /// ext2/ext3/ext4 family.
    Ext4,
    /// XFS filesystem.
    Xfs,
    /// Any other filesystem (vfat, ntfs, tmpfs, ...).
    Other,
}

impl DiskKind {
    /// Detect the filesystem kind from the sysinfo filesystem label.
    #[must_use]
    pub fn detect(filesystem: &str) -> Self {
        let filesystem = filesystem.trim().to_ascii_lowercase();
        if filesystem.contains("zfs") {
            Self::Zfs
        } else if filesystem.contains("btrfs") {
            Self::Btrfs
        } else if filesystem.starts_with("ext") {
            Self::Ext4
        } else if filesystem.contains("xfs") {
            Self::Xfs
        } else {
            Self::Other
        }
    }

    /// Stable identifier used by the API and UI.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Zfs => "zfs",
            Self::Btrfs => "btrfs",
            Self::Ext4 => "ext4",
            Self::Xfs => "xfs",
            Self::Other => "other",
        }
    }
}

/// Usage of a mounted filesystem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiskUsage {
    /// Mount point (`/`, `/var/lib/...`).
    pub mount: String,
    /// Filesystem label reported by the kernel (`ext4`, `zfs`, ...).
    pub filesystem: String,
    /// Total capacity in gibibytes.
    pub total_gb: f64,
    /// Used capacity in gibibytes.
    pub used_gb: f64,
    /// Available capacity in gibibytes.
    pub available_gb: f64,
    /// Used percentage (0-100).
    pub usage_percent: f32,
    /// Detected filesystem kind.
    pub kind: DiskKind,
}

/// Battery status of the appliance (laptop or UPS).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatteryStatus {
    /// Whether a battery is present.
    pub present: bool,
    /// Charge percentage (0-100).
    pub percent: u8,
    /// Whether the battery is currently charging.
    pub charging: bool,
    /// Estimated minutes remaining while discharging.
    pub time_remaining_min: Option<u32>,
}

/// Network interface information.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkInterfaceInfo {
    /// Interface name (`eth0`, `wlan0`, ...).
    pub name: String,
    /// Global IPv4 addresses.
    pub ipv4: Vec<String>,
    /// MAC address in colon notation, when the interface has one.
    pub mac: Option<String>,
    /// Total bytes transmitted since boot.
    pub bytes_sent: u64,
    /// Total bytes received since boot.
    pub bytes_received: u64,
}

/// Health of a host service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceHealth {
    /// Systemd unit name (`hs-daemon`).
    pub name: String,
    /// Whether the unit is currently active.
    pub active: bool,
    /// Extra context (`active`, `inactive`, `systemctl unavailable`, ...).
    pub detail: Option<String>,
}

/// A PipeWire audio sink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSink {
    /// PipeWire node id (`41`).
    pub id: String,
    /// Node name shown by `wpctl`.
    pub name: String,
    /// Human readable description.
    pub description: String,
    /// Whether this is the current default sink.
    pub is_default: bool,
}

/// Detected cameras and touch devices.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PeripheralStatus {
    /// Video device nodes (`video0`, `video1`, ...).
    pub camera_devices: Vec<String>,
    /// Input event nodes that belong to touch devices.
    pub touch_devices: Vec<String>,
}

/// Result of applying the power policy.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PowerPolicyReport {
    /// Whether `HandleLidSwitch=ignore` was written successfully.
    pub lid_switch_ignored: bool,
    /// Charge limit applied, when the battery supports it.
    pub charge_limit: Option<u8>,
    /// Non-fatal problems collected while applying the policy.
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Hardware port
// ---------------------------------------------------------------------------

/// Abstraction over the physical appliance hardware.
///
/// Implementations must be `Send + Sync` because the daemon shares one instance
/// between HTTP handlers. All methods are cheap synchronous probes except the
/// audio setters, which may spawn `wpctl`.
pub trait Hardware: Send + Sync {
    /// List PipeWire audio sinks.
    fn list_audio_sinks(&self) -> Vec<AudioSink>;

    /// Id of the current default audio sink, when known.
    fn default_audio_sink(&self) -> Option<String>;

    /// Change the default audio sink.
    ///
    /// # Errors
    /// Returns an error when the sink id is invalid or `wpctl` fails.
    fn set_default_sink(&self, id: &str) -> hs_core::CoreResult<()>;

    /// Set the volume of the default sink (0-100).
    ///
    /// # Errors
    /// Returns a validation error when `percent > 100` or the command fails.
    fn set_volume(&self, percent: u8) -> hs_core::CoreResult<()>;

    /// Read battery status, when a battery is present.
    fn read_battery(&self) -> Option<BatteryStatus>;

    /// Read the CPU/SoC temperature in Celsius, when a sensor is present.
    fn read_temperature(&self) -> Option<f32>;

    /// List camera device nodes.
    fn camera_devices(&self) -> Vec<String>;

    /// List touch input device nodes.
    fn touch_devices(&self) -> Vec<String>;

    /// Make logind ignore the laptop lid switch.
    ///
    /// # Errors
    /// Returns an I/O error when the logind drop-in cannot be written.
    fn apply_lid_switch_ignore(&self) -> hs_core::CoreResult<()>;

    /// Apply a battery charge limit (see [`CHARGE_LIMIT_RANGE`]).
    ///
    /// # Errors
    /// Returns a validation error for out-of-range values and an internal error
    /// when the battery does not expose the required sysfs attribute.
    fn apply_charge_limit(&self, percent: u8) -> hs_core::CoreResult<()>;

    /// Whether the host is Linux (the only supported appliance platform).
    fn is_linux(&self) -> bool;

    /// Current power policy as observed on the host.
    ///
    /// The default implementation reports "nothing applied"; implementations
    /// that can read state (sysfs/logind) override it.
    fn power_policy(&self) -> PowerPolicyReport {
        PowerPolicyReport::default()
    }
}

// ---------------------------------------------------------------------------
// System service
// ---------------------------------------------------------------------------

/// Orchestration service for host metrics, power policy and audio routing.
///
/// The daemon owns one instance created with [`SystemService::new`] and exposes
/// it read-mostly through the HTTP API.
pub struct SystemService {
    hardware: Arc<dyn Hardware>,
}

impl std::fmt::Debug for SystemService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemService").finish_non_exhaustive()
    }
}

impl Default for SystemService {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemService {
    /// Create a service backed by the production Linux hardware adapter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            hardware: Arc::new(LinuxHardware::new()),
        }
    }

    /// Create a service with an injected hardware implementation.
    #[must_use]
    pub fn with_hardware(hardware: Arc<dyn Hardware>) -> Self {
        Self { hardware }
    }

    /// Access the underlying hardware adapter (mostly useful for tests).
    #[must_use]
    pub fn hardware(&self) -> &Arc<dyn Hardware> {
        &self.hardware
    }

    /// Collect a full host overview.
    ///
    /// CPU usage needs two samples separated by [`sysinfo::MINIMUM_CPU_UPDATE_INTERVAL`],
    /// so the blocking work runs on the Tokio blocking pool.
    ///
    /// # Errors
    /// Returns an error when the blocking task cannot be joined; individual
    /// sensors that are missing only degrade the corresponding field.
    pub async fn overview(&self) -> hs_core::CoreResult<Overview> {
        let hardware = Arc::clone(&self.hardware);
        tokio::task::spawn_blocking(move || collect_overview(hardware.as_ref()))
            .await
            .map_err(|err| {
                hs_core::CoreError::Internal(format!("host metrics task failed: {err}"))
            })?
    }

    /// Current power policy as observed on the host.
    #[must_use]
    pub fn power_policy(&self) -> PowerPolicyReport {
        self.hardware.power_policy()
    }

    /// Apply the appliance power policy: ignore the lid switch and cap the
    /// battery charge.
    ///
    /// # Errors
    /// Returns a validation error when `limit_percent` is outside
    /// [`CHARGE_LIMIT_RANGE`]. Partial failures are reported in the returned
    /// [`PowerPolicyReport`] warnings instead of aborting.
    pub fn apply_power_policy(&self, limit_percent: u8) -> hs_core::CoreResult<PowerPolicyReport> {
        if !CHARGE_LIMIT_RANGE.contains(&limit_percent) {
            return Err(hs_core::CoreError::validation(
                "charge_limit",
                "must be between 60 and 80 percent",
            ));
        }
        let mut warnings = Vec::new();
        let lid_switch_ignored = match self.hardware.apply_lid_switch_ignore() {
            Ok(()) => true,
            Err(err) => {
                warnings.push(format!("lid switch policy: {err}"));
                false
            }
        };
        let charge_limit = match self.hardware.apply_charge_limit(limit_percent) {
            Ok(()) => Some(limit_percent),
            Err(err) => {
                warnings.push(format!("charge limit: {err}"));
                None
            }
        };
        tracing::info!(
            lid_switch_ignored,
            charge_limit = ?charge_limit,
            "power policy applied"
        );
        Ok(PowerPolicyReport {
            lid_switch_ignored,
            charge_limit,
            warnings,
        })
    }

    /// List available audio sinks.
    #[must_use]
    pub fn audio_sinks(&self) -> Vec<AudioSink> {
        self.hardware.list_audio_sinks()
    }

    /// Select the default audio sink.
    ///
    /// # Errors
    /// Returns a validation error for an empty id or a command error when
    /// `wpctl` fails.
    pub fn set_audio_sink(&self, id: &str) -> hs_core::CoreResult<()> {
        if id.trim().is_empty() {
            return Err(hs_core::CoreError::validation(
                "sink",
                "sink id must not be empty",
            ));
        }
        self.hardware.set_default_sink(id)
    }

    /// Set the volume of the default audio sink (0-100).
    ///
    /// # Errors
    /// Returns a validation error when `percent > 100`.
    pub fn set_volume(&self, percent: u8) -> hs_core::CoreResult<()> {
        if percent > 100 {
            return Err(hs_core::CoreError::validation(
                "volume",
                "must be between 0 and 100",
            ));
        }
        self.hardware.set_volume(percent)
    }

    /// Detect cameras and touch devices.
    #[must_use]
    pub fn peripherals(&self) -> PeripheralStatus {
        PeripheralStatus {
            camera_devices: self.hardware.camera_devices(),
            touch_devices: self.hardware.touch_devices(),
        }
    }
}

/// Blocking part of [`SystemService::overview`].
fn collect_overview(hardware: &dyn Hardware) -> hs_core::CoreResult<Overview> {
    let mut system = System::new();
    system.refresh_cpu_all();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    system.refresh_cpu_all();
    system.refresh_memory();
    let load = System::load_average();

    let networks = Networks::new_with_refreshed_list();
    let mut network: Vec<NetworkInterfaceInfo> = networks
        .list()
        .iter()
        .map(|(name, data)| {
            let mut ipv4: Vec<String> = data
                .ip_networks()
                .iter()
                .filter_map(|network| match network.addr {
                    std::net::IpAddr::V4(address) => Some(address.to_string()),
                    std::net::IpAddr::V6(_) => None,
                })
                .collect();
            ipv4.sort();
            ipv4.dedup();
            let mac = data.mac_address();
            NetworkInterfaceInfo {
                name: name.clone(),
                ipv4,
                mac: if mac.is_unspecified() {
                    None
                } else {
                    Some(mac.to_string())
                },
                bytes_sent: data.total_transmitted(),
                bytes_received: data.total_received(),
            }
        })
        .collect();
    network.sort_by(|left, right| left.name.cmp(&right.name));

    let disks = Disks::new_with_refreshed_list();
    let mut disk_usage: Vec<DiskUsage> = disks
        .list()
        .iter()
        .filter_map(|disk| {
            let total = disk.total_space();
            if total == 0 {
                return None;
            }
            let available = disk.available_space();
            let used = total.saturating_sub(available);
            let gib = 1024.0 * 1024.0 * 1024.0;
            let filesystem = disk.file_system().to_string_lossy().into_owned();
            Some(DiskUsage {
                mount: disk.mount_point().to_string_lossy().into_owned(),
                kind: DiskKind::detect(&filesystem),
                filesystem,
                total_gb: total as f64 / gib,
                used_gb: used as f64 / gib,
                available_gb: available as f64 / gib,
                usage_percent: (used as f64 / total as f64 * 100.0) as f32,
            })
        })
        .collect();
    disk_usage.sort_by(|left, right| left.mount.cmp(&right.mount));

    Ok(Overview {
        hostname: System::host_name().unwrap_or_else(|| "homeserver".to_owned()),
        os: os_description(),
        kernel: System::kernel_version().unwrap_or_else(|| "unknown".to_owned()),
        uptime_secs: System::uptime(),
        cpu: CpuStats {
            usage_percent: system.global_cpu_usage(),
            cores: system.cpus().len(),
            load_1: load.one as f32,
            load_5: load.five as f32,
            load_15: load.fifteen as f32,
        },
        memory: MemoryStats {
            total_mb: system.total_memory() / MIB,
            used_mb: system.used_memory() / MIB,
            available_mb: system.available_memory() / MIB,
            swap_total_mb: system.total_swap() / MIB,
            swap_used_mb: system.used_swap() / MIB,
        },
        disks: disk_usage,
        temperature_c: hardware.read_temperature(),
        battery: hardware.read_battery(),
        network,
        services: service_health(),
        generated_at: Utc::now(),
    })
}

/// One mebibyte in bytes.
const MIB: u64 = 1024 * 1024;

/// Best-effort operating system description.
fn os_description() -> String {
    #[cfg(target_os = "linux")]
    if let Ok(contents) = std::fs::read_to_string("/etc/os-release") {
        if let Some(line) = contents
            .lines()
            .find(|line| line.starts_with("PRETTY_NAME="))
        {
            return line
                .trim_start_matches("PRETTY_NAME=")
                .trim_matches('"')
                .to_owned();
        }
    }
    let name = System::name().unwrap_or_else(|| "Linux".to_owned());
    match System::os_version() {
        Some(version) if !version.is_empty() => format!("{name} {version}"),
        _ => name,
    }
}

/// Probe the appliance systemd units (best effort, never fails).
fn service_health() -> Vec<ServiceHealth> {
    SERVICE_UNITS
        .iter()
        .map(|unit| probe_service(unit))
        .collect()
}

/// Probe one systemd unit using `systemctl is-active` on Linux.
fn probe_service(unit: &str) -> ServiceHealth {
    if !cfg!(target_os = "linux") {
        return ServiceHealth {
            name: (*unit).to_owned(),
            active: false,
            detail: Some("service manager not available on this platform".to_owned()),
        };
    }
    match std::process::Command::new("systemctl")
        .arg("is-active")
        .arg(unit)
        .output()
    {
        Ok(output) => {
            let detail = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            ServiceHealth {
                name: (*unit).to_owned(),
                active: output.status.success() && detail == "active",
                detail: Some(if detail.is_empty() {
                    "unknown".to_owned()
                } else {
                    detail
                }),
            }
        }
        Err(err) => ServiceHealth {
            name: (*unit).to_owned(),
            active: false,
            detail: Some(format!("systemctl unavailable: {err}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{AudioSink, BatteryStatus, FakeHardware};

    #[tokio::test]
    async fn fake_hardware_overview_exposes_injected_sensors() {
        let hardware = Arc::new(
            FakeHardware::linux()
                .with_battery(BatteryStatus {
                    present: true,
                    percent: 77,
                    charging: true,
                    time_remaining_min: None,
                })
                .with_temperature(46.5)
                .with_audio(vec![AudioSink {
                    id: "41".to_owned(),
                    name: "analog-stereo".to_owned(),
                    description: "Built-in Audio".to_owned(),
                    is_default: true,
                }]),
        );
        let service = SystemService::with_hardware(hardware);
        let overview = service.overview().await.expect("host overview");
        assert!(!overview.hostname.is_empty());
        assert!(overview.cpu.cores >= 1);
        assert!(overview.cpu.usage_percent >= 0.0);
        assert!(overview.memory.total_mb > 0);
        assert_eq!(overview.temperature_c, Some(46.5));
        assert_eq!(
            overview.battery.as_ref().map(|battery| battery.percent),
            Some(77)
        );
        assert_eq!(overview.services.len(), SERVICE_UNITS.len());
        let names: Vec<_> = overview
            .services
            .iter()
            .map(|service| service.name.as_str())
            .collect();
        assert_eq!(names, SERVICE_UNITS.to_vec());
        assert!(overview.generated_at <= Utc::now());
    }

    #[test]
    fn volume_and_charge_limit_bounds_are_validated() {
        let service = SystemService::with_hardware(Arc::new(FakeHardware::linux()));
        assert!(service.set_volume(0).is_ok());
        assert!(service.set_volume(100).is_ok());
        match service.set_volume(101) {
            Err(hs_core::CoreError::Validation { field, .. }) => assert_eq!(field, "volume"),
            other => panic!("expected validation error, got {other:?}"),
        }
        assert!(service.apply_power_policy(60).is_ok());
        assert!(service.apply_power_policy(80).is_ok());
        for invalid in [0u8, 59, 81, 255] {
            match service.apply_power_policy(invalid) {
                Err(hs_core::CoreError::Validation { field, .. }) => {
                    assert_eq!(field, "charge_limit");
                }
                other => panic!("expected validation error, got {other:?}"),
            }
        }
        let report = service.apply_power_policy(70).expect("power policy");
        assert!(report.lid_switch_ignored);
        assert_eq!(report.charge_limit, Some(70));
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn audio_and_peripheral_wiring() {
        let hardware = FakeHardware::linux()
            .with_audio(vec![
                AudioSink {
                    id: "41".to_owned(),
                    name: "analog".to_owned(),
                    description: "Analog".to_owned(),
                    is_default: true,
                },
                AudioSink {
                    id: "45".to_owned(),
                    name: "hdmi".to_owned(),
                    description: "HDMI".to_owned(),
                    is_default: false,
                },
            ])
            .with_cameras(vec!["video0".to_owned()])
            .with_touch_devices(vec!["event3".to_owned()]);
        let service = SystemService::with_hardware(Arc::new(hardware));
        assert_eq!(service.audio_sinks().len(), 2);
        assert!(service.set_audio_sink("41").is_ok());
        assert!(matches!(
            service.set_audio_sink("   "),
            Err(hs_core::CoreError::Validation { .. })
        ));
        let peripherals = service.peripherals();
        assert_eq!(peripherals.camera_devices, vec!["video0".to_owned()]);
        assert_eq!(peripherals.touch_devices, vec!["event3".to_owned()]);
    }

    #[test]
    fn disk_kind_detection() {
        assert_eq!(DiskKind::detect("ext4"), DiskKind::Ext4);
        assert_eq!(DiskKind::detect("ext2"), DiskKind::Ext4);
        assert_eq!(DiskKind::detect("btrfs"), DiskKind::Btrfs);
        assert_eq!(DiskKind::detect("zfs"), DiskKind::Zfs);
        assert_eq!(DiskKind::detect("zfs_member"), DiskKind::Zfs);
        assert_eq!(DiskKind::detect("xfs"), DiskKind::Xfs);
        assert_eq!(DiskKind::detect("vfat"), DiskKind::Other);
        assert_eq!(DiskKind::detect(""), DiskKind::Other);
        assert_eq!(DiskKind::Ext4.id(), "ext4");
    }

    #[test]
    fn power_policy_reads_hardware_state() {
        let hardware = FakeHardware::linux().with_power_policy(true, Some(75));
        let service = SystemService::with_hardware(Arc::new(hardware));
        let policy = service.power_policy();
        assert!(policy.lid_switch_ignored);
        assert_eq!(policy.charge_limit, Some(75));
    }
}
