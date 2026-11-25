use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::Device;
use androidforge_core::toolchain::Toolchain;
use androidforge_core::util::ensure_dir;
use androidforge_process::{
    DetachedSpec, kill_tree, process_exists, process_image_name, spawn_detached,
};
use serde::{Deserialize, Serialize};

use crate::avd::list_avds;
use crate::port::{GRPC_PORT_OFFSET, allocate_port};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartedEmulator {
    pub pid: u32,
    pub serial: String,
    pub console_port: u16,
    pub avd: String,
    pub headless: bool,
    pub args: Vec<String>,
    pub log_path: String,
    pub device: Device,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmulatorStatus {
    pub serial: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub process_alive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adb_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_completed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<Device>,
    /// Console port chosen for this emulator (`emulator-<port>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console_port: Option<u16>,
    /// Console port + 1, used by ADB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adb_port: Option<u16>,
    /// Console port + 3000 when representable; `None` when no gRPC port exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grpc_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Conservative commit/RAM warning; absent when capacity is unknown or
    /// sufficient. Never contains a fabricated measurement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity_warning: Option<String>,
}

/// Read-only host capacity snapshot used by status warnings. Unknown values
/// stay `None`; callers (doctor --capacity) supply the measured values.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacitySnapshot {
    pub available_ram_bytes: Option<u64>,
    pub available_commit_bytes: Option<u64>,
}

impl CapacitySnapshot {
    pub fn is_unknown(&self) -> bool {
        self.available_ram_bytes.is_none() && self.available_commit_bytes.is_none()
    }
}

/// Optional context for an enriched status. Existing callers can keep using
/// `status(serial, pid)`; CLI wiring supplies owner/session/capacity later.
#[derive(Debug, Clone, Copy, Default)]
pub struct StatusContext<'a> {
    pub owner: Option<&'a str>,
    pub session_id: Option<&'a str>,
    /// Configured AVD RAM in MiB, when known.
    pub avd_ram_mb: Option<u32>,
    pub capacity: Option<&'a CapacitySnapshot>,
}

/// Purely derived status fields, testable without a device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusMetadata {
    pub console_port: Option<u16>,
    pub adb_port: Option<u16>,
    pub grpc_port: Option<u16>,
    pub owner: Option<String>,
    pub session_id: Option<String>,
    pub capacity_warning: Option<String>,
}

/// Extract the console port from an `emulator-<even port>` serial.
pub fn console_port_from_serial(serial: &str) -> Option<u16> {
    let port = serial.strip_prefix("emulator-")?;
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let port = port.parse::<u16>().ok()?;
    (port >= 1024 && port.is_multiple_of(2)).then_some(port)
}

/// Build a conservative commit warning only from known values. No warning is
/// produced when capacity or AVD RAM is unknown.
pub fn capacity_warning(
    avd_ram_mb: Option<u32>,
    capacity: Option<&CapacitySnapshot>,
) -> Option<String> {
    let ram_mb = avd_ram_mb?;
    let capacity = capacity?;
    let estimate = u64::from(ram_mb).saturating_mul(1024 * 1024);
    if capacity
        .available_commit_bytes
        .is_some_and(|available| estimate > available)
    {
        return Some(format!(
            "configured AVD RAM ({ram_mb} MiB) exceeds available commit; starting may fail"
        ));
    }
    if capacity
        .available_ram_bytes
        .is_some_and(|available| estimate > available)
    {
        return Some(format!(
            "configured AVD RAM ({ram_mb} MiB) exceeds available physical RAM; performance may degrade"
        ));
    }
    None
}

pub fn status_metadata(serial: &str, context: &StatusContext<'_>) -> StatusMetadata {
    let console_port = console_port_from_serial(serial);
    StatusMetadata {
        console_port,
        adb_port: console_port.and_then(|port| port.checked_add(1)),
        grpc_port: console_port.and_then(|port| port.checked_add(GRPC_PORT_OFFSET)),
        owner: context.owner.map(str::to_string),
        session_id: context.session_id.map(str::to_string),
        capacity_warning: capacity_warning(context.avd_ram_mb, context.capacity),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopResult {
    pub serial: String,
    pub method: String,
    pub process_exited: bool,
    pub adb_disconnected: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfo {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct EmulatorManager {
    pub toolchain: Toolchain,
    pub adb: AdbClient,
    pub trace: bool,
    pub boot_timeout: Duration,
    pub stop_timeout: Duration,
}

impl EmulatorManager {
    pub fn new(
        toolchain: Toolchain,
        adb: AdbClient,
        trace: bool,
        boot_timeout: Duration,
        stop_timeout: Duration,
    ) -> Self {
        Self {
            toolchain,
            adb,
            trace,
            boot_timeout,
            stop_timeout,
        }
    }

    pub async fn list_avds(&self) -> Result<Vec<crate::avd::AvdInfo>> {
        list_avds(&self.toolchain, self.trace).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        &self,
        avd: &str,
        requested_port: Option<u16>,
        headless: bool,
        memory_mb: Option<u32>,
        cores: Option<u32>,
        cold_boot: bool,
        snapshot_save: bool,
        extra_args: &[String],
        log_dir: &Path,
    ) -> Result<StartedEmulator> {
        let emulator = self.toolchain.require_emulator()?;
        let avds = self.list_avds().await?;
        let Some(avd_info) = avds.iter().find(|candidate| candidate.name == avd) else {
            let available = avds
                .iter()
                .map(|candidate| candidate.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ForgeError::new(
                ErrorCode::EmulatorNotFound,
                format!("AVD '{avd}' was not found"),
            )
            .with_context_string("available_avds", available)
            .with_suggestion("Run `androidforge avd list` to see installed AVDs.")
            .with_suggestion("Create an AVD with Android Studio or `avdmanager`."));
        };
        ensure_dir(log_dir)?;
        let reservation = allocate_port(requested_port)?;
        let port = reservation.console_port;
        let serial = reservation.serial();
        let log_path = log_dir.join(format!("emulator-{port}.log"));
        let err_path = log_dir.join(format!("emulator-{port}.stderr.log"));
        let mut args = vec![
            "-avd".to_string(),
            avd.to_string(),
            "-port".to_string(),
            port.to_string(),
            "-no-audio".to_string(),
            "-no-boot-anim".to_string(),
        ];
        if !snapshot_save {
            args.push("-no-snapshot-save".to_string());
        }
        if headless {
            args.push("-no-window".to_string());
        }
        if cold_boot {
            args.push("-no-snapshot-load".to_string());
        }
        if let Some(memory_mb) = memory_mb {
            if memory_mb < 512 {
                return Err(ForgeError::invalid_argument(
                    "--memory must be at least 512 MiB",
                ));
            }
            args.push("-memory".to_string());
            args.push(memory_mb.to_string());
        }
        if let Some(cores) = cores {
            if cores == 0 || cores > 64 {
                return Err(ForgeError::invalid_argument(
                    "--cores must be between 1 and 64",
                ));
            }
            args.push("-cores".to_string());
            args.push(cores.to_string());
        }
        args.extend(extra_args.iter().cloned());
        reservation.release();
        let spec = DetachedSpec::new(emulator.to_string_lossy().into_owned())
            .args(args.clone())
            .stdout_path(log_path.clone())
            .stderr_path(err_path.clone())
            .hide_window(headless)
            .trace(self.trace);
        let child = spawn_detached(&spec)?;
        let started = Instant::now();
        let mut last_device = None;
        let mut last_error = None;
        loop {
            if !process_exists(child.pid) {
                let log = read_log_tail(&log_path, 80);
                let err = read_log_tail(&err_path, 40);
                return Err(ForgeError::new(
                    ErrorCode::EmulatorBootTimeout,
                    format!("emulator process for {avd} exited before boot completed"),
                )
                .with_context_string("avd", avd.to_string())
                .with_context_string("port", port.to_string())
                .with_context_string("emulator_log", truncate(log.trim(), 6000))
                .with_context_string("emulator_stderr", truncate(err.trim(), 3000))
                .with_suggestion(
                    "Inspect the emulator log for a lock, GPU, or AVD configuration error.",
                )
                .with_suggestion("Run `androidforge avd list` and confirm the AVD is valid.")
                .with_suggestion(
                    "Try a different --port if the console port was claimed between checks.",
                ));
            }
            match self.adb.devices_raw().await {
                Ok(devices) => {
                    if let Some(device) = devices.into_iter().find(|device| device.serial == serial)
                    {
                        last_device = Some(device.clone());
                        if device.state.is_operational() && self.adb.is_boot_complete(&serial).await
                        {
                            let device = self.adb.inspect(&serial).await?;
                            return Ok(StartedEmulator {
                                pid: child.pid,
                                serial,
                                console_port: port,
                                avd: avd_info.name.clone(),
                                headless,
                                args,
                                log_path: log_path.to_string_lossy().into_owned(),
                                device,
                            });
                        }
                    }
                }
                Err(error) => {
                    last_error = Some(error.message().to_string());
                }
            }
            if started.elapsed() >= self.boot_timeout {
                kill_tree(child.pid);
                let log = read_log_tail(&log_path, 80);
                let state = last_device
                    .as_ref()
                    .map(|device| device.state.as_str())
                    .unwrap_or("not-visible-to-adb");
                let mut error = ForgeError::new(
                    ErrorCode::EmulatorBootTimeout,
                    format!(
                        "emulator {serial} did not finish booting within {} seconds",
                        self.boot_timeout.as_secs()
                    ),
                )
                .with_context_string("avd", avd.to_string())
                .with_context_string("adb_state", state)
                .with_context_string("emulator_log", truncate(log.trim(), 6000))
                .with_suggestion("Inspect the emulator log and `androidforge devices list`.")
                .with_suggestion("Increase [timeouts].boot_secs if the host is slow.")
                .with_suggestion("Use `--cold-boot` if the AVD's snapshot is corrupt.");
                if let Some(last_error) = last_error {
                    error = error.with_context_string("last_adb_error", last_error);
                }
                return Err(error);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    pub async fn stop(&self, serial: &str, pid: Option<u32>) -> Result<StopResult> {
        let mut notes = Vec::new();
        let expected_image = self.toolchain.require_emulator().ok().and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
        let emu_result = self
            .adb
            .emu(serial, &["kill".to_string()], Some(Duration::from_secs(15)))
            .await;
        let mut method = "adb_emu_kill".to_string();
        if let Err(error) = &emu_result {
            notes.push(format!("adb emu kill failed: {}", error.message()));
            match pid {
                Some(pid) => {
                    if kill_verified_emulator(pid, expected_image.as_deref(), &mut notes) {
                        method = "taskkill_tree".to_string();
                    }
                }
                None => {
                    return Err(ForgeError::new(
                        ErrorCode::CommandFailed,
                        format!("could not stop emulator {serial}"),
                    )
                    .with_cause(error.message())
                    .with_suggestion("The emulator may not be owned by this AndroidForge session.")
                    .with_suggestion("Run `androidforge session list` before trying to stop it.")
                    .device(serial.to_string()));
                }
            }
        }
        let started = Instant::now();
        let mut process_exited = pid.map(|pid| !process_exists(pid)).unwrap_or(true);
        let mut adb_disconnected = false;
        while started.elapsed() < self.stop_timeout {
            process_exited = pid.map(|pid| !process_exists(pid)).unwrap_or(true);
            adb_disconnected = match self.adb.devices_raw().await {
                Ok(devices) => !devices.iter().any(|device| device.serial == serial),
                Err(_) => true,
            };
            if process_exited && adb_disconnected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if !process_exited
            && let Some(pid) = pid
            && kill_verified_emulator(pid, expected_image.as_deref(), &mut notes)
        {
            method = "taskkill_tree".to_string();
            tokio::time::sleep(Duration::from_millis(400)).await;
            process_exited = !process_exists(pid);
        }
        if !process_exited || !adb_disconnected {
            notes.push(
                "emulator did not disappear from both the process table and adb before the stop timeout"
                    .to_string(),
            );
        }
        Ok(StopResult {
            serial: serial.to_string(),
            method,
            process_exited,
            adb_disconnected,
            notes,
        })
    }

    pub async fn status(&self, serial: &str, pid: Option<u32>) -> Result<EmulatorStatus> {
        self.status_with_context(serial, pid, StatusContext::default())
            .await
    }

    /// Enriched status with caller-supplied session ownership and host
    /// capacity. This is core data only; no CLI flags are wired here.
    pub async fn status_with_context(
        &self,
        serial: &str,
        pid: Option<u32>,
        context: StatusContext<'_>,
    ) -> Result<EmulatorStatus> {
        let metadata = status_metadata(serial, &context);
        let process_alive = pid.map(process_exists).unwrap_or(false);
        let device = match self.adb.inspect(serial).await {
            Ok(device) => Some(device),
            Err(_) => {
                let devices = self.adb.devices_raw().await?;
                devices.into_iter().find(|device| device.serial == serial)
            }
        };
        let avd = self
            .adb
            .emu(
                serial,
                &["avd".to_string(), "name".to_string()],
                Some(Duration::from_secs(8)),
            )
            .await
            .ok()
            .and_then(|output| {
                output
                    .stdout_text()
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty() && *line != "OK" && *line != "KO")
                    .map(str::to_string)
            });
        Ok(EmulatorStatus {
            serial: serial.to_string(),
            avd,
            pid,
            process_alive,
            adb_state: device
                .as_ref()
                .map(|device| device.state.as_str().to_string()),
            boot_completed: device.as_ref().and_then(|device| device.boot_completed),
            device: device.map(|device| device.compact()),
            console_port: metadata.console_port,
            adb_port: metadata.adb_port,
            grpc_port: metadata.grpc_port,
            owner: metadata.owner,
            session_id: metadata.session_id,
            capacity_warning: metadata.capacity_warning,
        })
    }

    pub async fn snapshot_list(&self, serial: &str) -> Result<Vec<SnapshotInfo>> {
        let output = self
            .adb
            .emu(
                serial,
                &[
                    "avd".to_string(),
                    "snapshot".to_string(),
                    "list".to_string(),
                ],
                Some(Duration::from_secs(20)),
            )
            .await?;
        let text = output.combined_text();
        Ok(parse_snapshot_list(&text))
    }

    pub async fn snapshot_save(&self, serial: &str, name: &str) -> Result<SnapshotInfo> {
        validate_snapshot_name(name)?;
        let output = self
            .adb
            .emu(
                serial,
                &[
                    "avd".to_string(),
                    "snapshot".to_string(),
                    "save".to_string(),
                    name.to_string(),
                ],
                Some(Duration::from_secs(120)),
            )
            .await?;
        let text = output.combined_text();
        if !text.to_ascii_uppercase().contains("OK") {
            return Err(ForgeError::new(
                ErrorCode::UnsupportedOperation,
                format!("emulator did not confirm snapshot save '{name}'"),
            )
            .with_context_string("output", truncate(text.trim(), 3000))
            .with_suggestion("Verify that the emulator supports snapshots and is not read-only.")
            .device(serial.to_string()));
        }
        // The emulator can acknowledge the save before the snapshot is fully
        // written and visible through the console. Poll briefly instead of
        // reporting a false failure on a slow disk.
        for _ in 0..60 {
            let snapshots = self.snapshot_list(serial).await?;
            if snapshots.iter().any(|snapshot| snapshot.name == name) {
                return Ok(SnapshotInfo {
                    name: name.to_string(),
                });
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Err(ForgeError::new(
            ErrorCode::UnsupportedOperation,
            format!("snapshot '{name}' did not appear in the emulator snapshot list"),
        )
        .with_context_string("output", truncate(text.trim(), 3000))
        .with_suggestion("Snapshot support varies with emulator version and AVD configuration.")
        .device(serial.to_string()))
    }

    pub async fn snapshot_load(
        &self,
        serial: &str,
        name: &str,
        confirmed: bool,
    ) -> Result<SnapshotInfo> {
        validate_snapshot_name(name)?;
        if !confirmed {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                "loading a snapshot overwrites the emulator's current user data; pass --yes to confirm",
            )
            .with_suggestion("Save important emulator state first or use a disposable AVD.")
            .device(serial.to_string()));
        }
        let snapshots = self.snapshot_list(serial).await?;
        if !snapshots.iter().any(|snapshot| snapshot.name == name) {
            return Err(ForgeError::new(
                ErrorCode::SnapshotNotFound,
                format!("snapshot '{name}' was not found"),
            )
            .with_context(
                "available",
                serde_json::to_value(&snapshots).unwrap_or(serde_json::Value::Null),
            )
            .device(serial.to_string()));
        }
        let output = self
            .adb
            .emu(
                serial,
                &[
                    "avd".to_string(),
                    "snapshot".to_string(),
                    "load".to_string(),
                    name.to_string(),
                ],
                Some(Duration::from_secs(180)),
            )
            .await?;
        let text = output.combined_text();
        if !text.to_ascii_uppercase().contains("OK") {
            return Err(ForgeError::new(
                ErrorCode::UnsupportedOperation,
                format!("emulator did not confirm snapshot load '{name}'"),
            )
            .with_context_string("output", truncate(text.trim(), 3000))
            .device(serial.to_string()));
        }
        // Snapshot load can reset the ADB connection; wait for the device and boot.
        let device = self
            .adb
            .wait_for_device(Some(serial), true, self.boot_timeout)
            .await?;
        let _ = device;
        Ok(SnapshotInfo {
            name: name.to_string(),
        })
    }

    pub async fn snapshot_delete(&self, serial: &str, name: &str) -> Result<SnapshotInfo> {
        validate_snapshot_name(name)?;
        let output = self
            .adb
            .emu(
                serial,
                &[
                    "avd".to_string(),
                    "snapshot".to_string(),
                    "delete".to_string(),
                    name.to_string(),
                ],
                Some(Duration::from_secs(60)),
            )
            .await?;
        let text = output.combined_text();
        if text.to_ascii_uppercase().contains("KO") {
            return Err(ForgeError::new(
                ErrorCode::SnapshotNotFound,
                format!("snapshot '{name}' could not be deleted"),
            )
            .with_context_string("output", truncate(text.trim(), 2000))
            .device(serial.to_string()));
        }
        Ok(SnapshotInfo {
            name: name.to_string(),
        })
    }

    pub fn log_path_for(&self, log_dir: &Path, port: u16) -> PathBuf {
        log_dir.join(format!("emulator-{port}.log"))
    }
}

fn validate_snapshot_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(ForgeError::invalid_argument(
            "snapshot names may contain only letters, numbers, '.', '_' and '-'",
        ));
    }
    Ok(())
}

fn parse_snapshot_list(output: &str) -> Vec<SnapshotInfo> {
    let mut snapshots = Vec::new();
    for line in output.lines() {
        let line = line.trim().trim_matches('"').trim();
        if line.is_empty()
            || line.eq_ignore_ascii_case("OK")
            || line.eq_ignore_ascii_case("KO")
            || line.to_ascii_lowercase().contains("list of snapshots")
            || line.to_ascii_lowercase().starts_with("no snapshot")
            || line.to_ascii_lowercase().contains("error")
        {
            continue;
        }
        let name = if let Some(name) = line.strip_prefix("snapshot:") {
            name.trim().to_string()
        } else {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            match fields.as_slice() {
                // `adb emu avd snapshot list` is tabular:
                // ID  TAG  VM SIZE  DATE  VM CLOCK
                // --  clean-install  68M  2026-...  00:00:20
                [id, name, ..] if *id == "--" => (*name).to_string(),
                _ => line.to_string(),
            }
        };
        if !name.is_empty()
            && !name.eq_ignore_ascii_case("TAG")
            && name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        {
            snapshots.push(SnapshotInfo { name });
        }
    }
    snapshots
}

fn read_log_tail(path: &Path, lines: usize) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    androidforge_core::util::tail_lines(&text, lines)
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect::<String>() + "\n[truncated]"
    }
}

fn kill_verified_emulator(pid: u32, expected_image: Option<&str>, notes: &mut Vec<String>) -> bool {
    if !process_exists(pid) {
        return false;
    }
    let Some(observed) = process_image_name(pid) else {
        notes.push(format!(
            "refusing to kill PID {pid}: could not read its process identity"
        ));
        return false;
    };
    let Some(expected) = expected_image else {
        notes.push(format!(
            "refusing to kill PID {pid}: emulator executable is unknown"
        ));
        return false;
    };
    if !observed.eq_ignore_ascii_case(expected) {
        notes.push(format!(
            "refusing to kill PID {pid}: process is '{observed}', not '{expected}'"
        ));
        return false;
    }
    kill_tree(pid);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_list_ignores_protocol_words() {
        let snapshots = parse_snapshot_list("OK\nclean-install\nwork-state\nKO\n");
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].name, "clean-install");
    }

    #[test]
    fn snapshot_list_parses_the_emulator_table() {
        let snapshots = parse_snapshot_list(
            "List of snapshots present on all disks:\n\
             ID        TAG                 VM SIZE                DATE       VM CLOCK\n\
             --        default_boot            68M 2026-11-18 07:02:20   00:00:25.205\n\
             --        af_snapshot_probe       68M 2026-11-18 07:03:23   00:00:23.973\n\
             OK\n",
        );
        assert_eq!(
            snapshots
                .iter()
                .map(|snapshot| snapshot.name.as_str())
                .collect::<Vec<_>>(),
            vec!["default_boot", "af_snapshot_probe"]
        );
    }

    #[test]
    fn snapshot_names_are_restricted() {
        assert!(validate_snapshot_name("clean-install_1").is_ok());
        assert!(validate_snapshot_name("../bad").is_err());
    }

    #[test]
    fn status_ports_and_owner_come_from_serial_and_context() {
        let capacity = CapacitySnapshot {
            available_ram_bytes: Some(32 * 1024 * 1024 * 1024),
            available_commit_bytes: Some(64 * 1024 * 1024 * 1024),
        };
        let context = StatusContext {
            owner: Some("agent-a"),
            session_id: Some("ses-1"),
            avd_ram_mb: Some(2048),
            capacity: Some(&capacity),
        };
        let metadata = status_metadata("emulator-5556", &context);
        assert_eq!(metadata.console_port, Some(5556));
        assert_eq!(metadata.adb_port, Some(5557));
        assert_eq!(metadata.grpc_port, Some(8556));
        assert_eq!(metadata.owner.as_deref(), Some("agent-a"));
        assert_eq!(metadata.session_id.as_deref(), Some("ses-1"));
        assert_eq!(metadata.capacity_warning, None);
    }

    #[test]
    fn console_port_parsing_rejects_non_emulator_serials() {
        assert_eq!(console_port_from_serial("emulator-5554"), Some(5554));
        assert_eq!(console_port_from_serial("emulator-5555"), None);
        assert_eq!(console_port_from_serial("emulator-1024"), Some(1024));
        assert_eq!(console_port_from_serial("emulator-1000"), None);
        assert_eq!(console_port_from_serial("0123456789012345"), None);
        assert_eq!(console_port_from_serial("emulator-abc"), None);
        assert_eq!(console_port_from_serial("usb-device"), None);
    }

    #[test]
    fn capacity_warning_is_conservative_when_unavailable() {
        let unknown = CapacitySnapshot::default();
        assert!(unknown.is_unknown());
        assert_eq!(capacity_warning(Some(8192), Some(&unknown)), None);
        assert_eq!(capacity_warning(None, None), None);
        let tight = CapacitySnapshot {
            available_ram_bytes: Some(1024 * 1024 * 1024),
            available_commit_bytes: Some(1024 * 1024 * 1024),
        };
        let warning = capacity_warning(Some(4096), Some(&tight)).unwrap();
        assert!(warning.contains("exceeds available commit"));
    }

    #[test]
    fn status_serialization_keeps_new_fields_optional() {
        let status = EmulatorStatus {
            serial: "emulator-5554".to_string(),
            avd: None,
            pid: None,
            process_alive: false,
            adb_state: None,
            boot_completed: None,
            device: None,
            console_port: Some(5554),
            adb_port: Some(5555),
            grpc_port: Some(8554),
            owner: Some("agent-a".to_string()),
            session_id: None,
            capacity_warning: None,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["console_port"], 5554);
        assert_eq!(json["owner"], "agent-a");
        assert!(json.get("session_id").is_none());
        assert!(json.get("capacity_warning").is_none());
    }

    #[test]
    fn emulator_kill_refuses_an_unverified_process() {
        let pid = std::process::id();
        let mut notes = Vec::new();
        assert!(!kill_verified_emulator(
            pid,
            Some("definitely-not-emulator.exe"),
            &mut notes
        ));
        assert!(notes[0].contains("refusing to kill"));
        let mut notes = Vec::new();
        assert!(!kill_verified_emulator(pid, None, &mut notes));
        assert!(notes[0].contains("executable is unknown"));
    }
}
