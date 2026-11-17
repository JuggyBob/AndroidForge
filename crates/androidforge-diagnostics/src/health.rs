//! Structured, read-only device health.
//!
//! Android and vendor ROMs differ. Every collector records an explicit reason
//! when a value is unavailable; no metric is guessed.

use std::collections::BTreeMap;

use androidforge_adb::AdbClient;
use androidforge_core::config::HealthSection;
use androidforge_core::error::Result;
use androidforge_core::model::Device;
use androidforge_core::util::now_rfc3339;
use serde::{Deserialize, Serialize};

pub const HEALTH_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthEvidence {
    pub command: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BatteryHealth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_percent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_c: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown_reasons: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThermalHealth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<i32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown_reasons: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageHealth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown_reasons: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryHealth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown_reasons: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkHealth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub airplane_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wifi_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_interface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown_reasons: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScreenHealth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interactive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown_reasons: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceHealth {
    pub schema_version: u32,
    pub collected_at: String,
    pub serial: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_level: Option<u32>,
    pub adb_state: String,
    pub battery: BatteryHealth,
    pub thermal: ThermalHealth,
    pub storage: StorageHealth,
    pub memory: MemoryHealth,
    pub network: NetworkHealth,
    pub screen: ScreenHealth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uptime_seconds: Option<u64>,
    pub thresholds: HealthThresholds,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<HealthEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthThresholds {
    pub battery_warn_percent: u32,
    pub storage_warn_bytes: u64,
    pub thermal_warn_code: i32,
}

impl From<&HealthSection> for HealthThresholds {
    fn from(section: &HealthSection) -> Self {
        Self {
            battery_warn_percent: section.battery_warn_percent,
            storage_warn_bytes: section.storage_warn_bytes,
            thermal_warn_code: section.thermal_warn_code,
        }
    }
}

fn excerpt(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        trimmed.to_string()
    } else {
        let mut value = trimmed.chars().take(max).collect::<String>();
        value.push_str("\n[truncated]");
        value
    }
}

fn command_text(values: &[&str]) -> String {
    values.join(" ")
}

async fn shell(
    adb: &AdbClient,
    serial: &str,
    args: &[&str],
    evidence: &mut Vec<HealthEvidence>,
) -> Result<String> {
    let argv = args
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    let command = command_text(args);
    let result = adb
        .shell_text(serial, &argv, Some(std::time::Duration::from_secs(10)))
        .await;
    match result {
        Ok(text) => {
            evidence.push(HealthEvidence {
                command,
                status: "ok".to_string(),
                excerpt: Some(excerpt(&text, 1200)),
                reason: None,
            });
            Ok(text)
        }
        Err(error) => {
            evidence.push(HealthEvidence {
                command,
                status: "unavailable".to_string(),
                excerpt: None,
                reason: Some(error.message().to_string()),
            });
            Err(error)
        }
    }
}

/// Collect read-only health state. No device setting is changed.
pub async fn collect_health(
    adb: &AdbClient,
    device: &Device,
    thresholds: &HealthThresholds,
) -> DeviceHealth {
    let serial = device.serial.clone();
    let mut evidence = Vec::new();
    let mut battery = BatteryHealth::default();
    let mut thermal = ThermalHealth::default();
    let mut storage = StorageHealth::default();
    let mut memory = MemoryHealth::default();
    let mut network = NetworkHealth::default();
    let mut screen = ScreenHealth::default();
    let mut uptime_seconds = None;
    let mut warnings = Vec::new();

    let battery_raw = shell(adb, &serial, &["dumpsys", "battery"], &mut evidence).await;
    match battery_raw {
        Ok(text) => parse_battery(&text, &mut battery),
        Err(error) => {
            battery
                .unknown_reasons
                .insert("battery".to_string(), error.message().to_string());
        }
    }

    let thermal_raw = shell(adb, &serial, &["dumpsys", "thermalservice"], &mut evidence).await;
    match thermal_raw {
        Ok(text) => parse_thermal(&text, &mut thermal),
        Err(error) => {
            thermal
                .unknown_reasons
                .insert("thermal".to_string(), error.message().to_string());
        }
    }

    let storage_raw = shell(adb, &serial, &["df", "-k", "/data"], &mut evidence).await;
    match storage_raw {
        Ok(text) => parse_storage(&text, &mut storage),
        Err(error) => {
            storage
                .unknown_reasons
                .insert("storage".to_string(), error.message().to_string());
        }
    }

    let memory_raw = shell(adb, &serial, &["cat", "/proc/meminfo"], &mut evidence).await;
    match memory_raw {
        Ok(text) => parse_memory(&text, &mut memory),
        Err(error) => {
            memory
                .unknown_reasons
                .insert("memory".to_string(), error.message().to_string());
        }
    }

    let airplane_raw = shell(
        adb,
        &serial,
        &["settings", "get", "global", "airplane_mode_on"],
        &mut evidence,
    )
    .await;
    match airplane_raw {
        Ok(text) => network.airplane_mode = parse_bool_int(&text),
        Err(error) => {
            network
                .unknown_reasons
                .insert("airplane_mode".to_string(), error.message().to_string());
        }
    }

    let wifi_raw = shell(
        adb,
        &serial,
        &["settings", "get", "global", "wifi_on"],
        &mut evidence,
    )
    .await;
    match wifi_raw {
        Ok(text) => network.wifi_enabled = parse_bool_int(&text),
        Err(error) => {
            network
                .unknown_reasons
                .insert("wifi_enabled".to_string(), error.message().to_string());
        }
    }

    let route_raw = shell(adb, &serial, &["ip", "route"], &mut evidence).await;
    match route_raw {
        Ok(text) => {
            if let Some(interface) = parse_active_interface(&text) {
                network.active_interface = Some(interface);
            } else {
                network.unknown_reasons.insert(
                    "active_interface".to_string(),
                    "no default route was reported".to_string(),
                );
            }
            network.summary = Some(excerpt(&text, 400));
        }
        Err(error) => {
            network
                .unknown_reasons
                .insert("active_interface".to_string(), error.message().to_string());
        }
    }

    let power_raw = shell(adb, &serial, &["dumpsys", "power"], &mut evidence).await;
    match power_raw {
        Ok(text) => {
            screen.interactive = parse_wakefulness(&text);
            if screen.interactive.is_none() {
                screen.unknown_reasons.insert(
                    "interactive".to_string(),
                    "mWakefulness was not reported".to_string(),
                );
            }
        }
        Err(error) => {
            screen
                .unknown_reasons
                .insert("interactive".to_string(), error.message().to_string());
        }
    }

    let window_raw = shell(adb, &serial, &["dumpsys", "window"], &mut evidence).await;
    match window_raw {
        Ok(text) => {
            screen.locked = parse_locked(&text);
            if screen.locked.is_none() {
                screen.unknown_reasons.insert(
                    "locked".to_string(),
                    "no keyguard/dreaming flag was reported".to_string(),
                );
            }
        }
        Err(error) => {
            screen
                .unknown_reasons
                .insert("locked".to_string(), error.message().to_string());
        }
    }

    let uptime_raw = shell(adb, &serial, &["cat", "/proc/uptime"], &mut evidence).await;
    match uptime_raw {
        Ok(text) => uptime_seconds = parse_uptime(&text),
        Err(error) => warnings.push(format!("Uptime is unavailable: {}", error.message())),
    }

    let adb_state = adb
        .run_raw(
            Some(&serial),
            &["get-state".to_string()],
            Some(std::time::Duration::from_secs(5)),
            None,
        )
        .await
        .map(|output| output.stdout_text().trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    if let Some(level) = battery.level_percent
        && level < thresholds.battery_warn_percent
        && battery.charging != Some(true)
    {
        warnings.push(format!(
            "Battery is {}%, below the {}% warning threshold and not charging.",
            level, thresholds.battery_warn_percent
        ));
    }
    if let Some(available) = storage.available_bytes
        && available < thresholds.storage_warn_bytes
    {
        warnings.push(format!(
            "Data storage has {} MiB free, below the {} MiB warning threshold.",
            available / (1024 * 1024),
            thresholds.storage_warn_bytes / (1024 * 1024)
        ));
    }
    if let Some(code) = thermal.status_code
        && code >= thresholds.thermal_warn_code
    {
        warnings.push(format!(
            "Thermal status {} meets or exceeds the configured warning level {}.",
            code, thresholds.thermal_warn_code
        ));
    }

    DeviceHealth {
        schema_version: HEALTH_SCHEMA_VERSION,
        collected_at: now_rfc3339(),
        serial,
        model: device.model.clone(),
        manufacturer: device.manufacturer.clone(),
        android_version: device.android_version.clone(),
        api_level: device.api_level,
        adb_state,
        battery,
        thermal,
        storage,
        memory,
        network,
        screen,
        uptime_seconds,
        thresholds: thresholds.clone(),
        warnings,
        evidence,
    }
}

fn parse_bool_int(text: &str) -> Option<bool> {
    match text.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

fn parse_key_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let value = line.strip_prefix(key)?;
        Some(value.trim())
    })
}

pub fn parse_battery(text: &str, battery: &mut BatteryHealth) {
    if let Some(level) = parse_key_value(text, "level:").and_then(|value| value.parse::<u32>().ok())
    {
        battery.level_percent = Some(level);
    } else {
        battery.unknown_reasons.insert(
            "level_percent".to_string(),
            "level was not reported".to_string(),
        );
    }
    battery.status = parse_key_value(text, "status:").map(|value| match value {
        "1" => "unknown".to_string(),
        "2" => "charging".to_string(),
        "3" => "discharging".to_string(),
        "4" => "not_charging".to_string(),
        "5" => "full".to_string(),
        other => format!("status_{other}"),
    });
    battery.charging = battery
        .status
        .as_deref()
        .map(|status| matches!(status, "charging" | "full"));
    battery.temperature_c = parse_key_value(text, "temperature:")
        .and_then(|value| value.parse::<f64>().ok())
        .map(|tenths| tenths / 10.0);
    battery.health = parse_key_value(text, "health:").map(|value| match value {
        "1" => "unknown".to_string(),
        "2" => "good".to_string(),
        "3" => "overheat".to_string(),
        "4" => "dead".to_string(),
        "5" => "over_voltage".to_string(),
        "6" => "failure".to_string(),
        "7" => "cold".to_string(),
        other => format!("health_{other}"),
    });
}

pub fn parse_thermal(text: &str, thermal: &mut ThermalHealth) {
    let code = text.lines().find_map(|line| {
        let line = line.trim();
        let value = line
            .strip_prefix("Thermal Status:")
            .or_else(|| line.strip_prefix("mStatus="))
            .or_else(|| line.strip_prefix("Status:"))?;
        value.trim().parse::<i32>().ok()
    });
    thermal.status_code = code;
    thermal.status = code.map(|code| {
        match code {
            0 => "none",
            1 => "light",
            2 => "moderate",
            3 => "severe",
            4 => "critical",
            5 => "emergency",
            6 => "shutdown",
            _ => "unknown",
        }
        .to_string()
    });
    if code.is_none() {
        thermal.unknown_reasons.insert(
            "thermal".to_string(),
            "thermal status was not reported".to_string(),
        );
    }
}

pub fn parse_storage(text: &str, storage: &mut StorageHealth) {
    let Some(numbers) = text.lines().rev().find_map(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 4 {
            return None;
        }
        let total = fields.get(1)?.parse::<u64>().ok()?;
        let used = fields.get(2)?.parse::<u64>().ok()?;
        let available = fields.get(3)?.parse::<u64>().ok()?;
        Some((total, used, available))
    }) else {
        storage.unknown_reasons.insert(
            "storage".to_string(),
            "df did not return numeric data blocks".to_string(),
        );
        return;
    };
    let (total, used, available) = numbers;
    storage.total_bytes = Some(total.saturating_mul(1024));
    storage.used_bytes = Some(used.saturating_mul(1024));
    storage.available_bytes = Some(available.saturating_mul(1024));
    storage.used_percent = (total > 0).then(|| used as f64 / total as f64 * 100.0);
}

pub fn parse_memory(text: &str, memory: &mut MemoryHealth) {
    let value_kb = |key: &str| {
        text.lines().find_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix(key)?;
            rest.split_whitespace()
                .next()
                .and_then(|value| value.parse::<u64>().ok())
        })
    };
    memory.total_bytes = value_kb("MemTotal:").map(|kb| kb.saturating_mul(1024));
    memory.available_bytes = value_kb("MemAvailable:").map(|kb| kb.saturating_mul(1024));
    if memory.total_bytes.is_none() {
        memory.unknown_reasons.insert(
            "total_bytes".to_string(),
            "MemTotal was not reported".to_string(),
        );
    }
    if memory.available_bytes.is_none() {
        memory.unknown_reasons.insert(
            "available_bytes".to_string(),
            "MemAvailable was not reported".to_string(),
        );
    }
}

pub fn parse_active_interface(text: &str) -> Option<String> {
    let interface = |line: &str| {
        line.split_whitespace()
            .skip_while(|field| *field != "dev")
            .nth(1)
            .map(str::to_string)
    };
    text.lines()
        .find(|line| {
            line.trim_start().starts_with("default") || line.trim_start().starts_with("0.0.0.0")
        })
        .and_then(interface)
        .or_else(|| {
            text.lines()
                .find(|line| line.contains(" dev "))
                .and_then(interface)
        })
}

pub fn parse_wakefulness(text: &str) -> Option<bool> {
    let value = parse_key_value(text, "mWakefulness=")?;
    Some(matches!(value, "Awake" | "Dreaming"))
}

pub fn parse_locked(text: &str) -> Option<bool> {
    for key in [
        "isKeyguardShowing=",
        "mDreamingLockscreen=",
        "mShowingLockscreen=",
        "mKeyguardShowing=",
        "mKeyguardShown=",
    ] {
        if let Some(value) = parse_key_value(text, key) {
            return Some(value.eq_ignore_ascii_case("true"));
        }
    }
    let mut in_keyguard = false;
    for line in text.lines() {
        if line.contains("KeyguardServiceDelegate") {
            in_keyguard = true;
            continue;
        }
        if in_keyguard {
            if let Some(value) = parse_key_value(line, "showing=") {
                return Some(value.eq_ignore_ascii_case("true"));
            }
            if !line.trim().is_empty() && !line.starts_with(' ') {
                in_keyguard = false;
            }
        }
    }
    None
}

pub fn parse_uptime(text: &str) -> Option<u64> {
    text.split_whitespace()
        .next()
        .and_then(|value| value.parse::<f64>().ok())
        .map(|seconds| seconds.max(0.0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_battery_variants() {
        let mut battery = BatteryHealth::default();
        parse_battery(
            "Current Battery Service state:\n  level: 14\n  status: 3\n  health: 2\n  temperature: 315\n",
            &mut battery,
        );
        assert_eq!(battery.level_percent, Some(14));
        assert_eq!(battery.status.as_deref(), Some("discharging"));
        assert_eq!(battery.charging, Some(false));
        assert_eq!(battery.temperature_c, Some(31.5));
        assert_eq!(battery.health.as_deref(), Some("good"));
    }

    #[test]
    fn parses_thermal_and_unknown() {
        let mut thermal = ThermalHealth::default();
        parse_thermal("Thermal Status: 3\n", &mut thermal);
        assert_eq!(thermal.status_code, Some(3));
        assert_eq!(thermal.status.as_deref(), Some("severe"));
        let mut missing = ThermalHealth::default();
        parse_thermal("no thermal data", &mut missing);
        assert!(missing.status_code.is_none());
        assert!(!missing.unknown_reasons.is_empty());
    }

    #[test]
    fn parses_storage_in_kibibytes() {
        let mut storage = StorageHealth::default();
        parse_storage(
            "Filesystem 1K-blocks Used Available Use% Mounted on\n/dev/block/dm-50 100000 40000 60000 40% /data\n",
            &mut storage,
        );
        assert_eq!(storage.total_bytes, Some(100_000 * 1024));
        assert_eq!(storage.used_bytes, Some(40_000 * 1024));
        assert_eq!(storage.available_bytes, Some(60_000 * 1024));
        assert_eq!(storage.used_percent, Some(40.0));
    }

    #[test]
    fn parses_memory() {
        let mut memory = MemoryHealth::default();
        parse_memory(
            "MemTotal:        8000000 kB\nMemFree: 1000 kB\nMemAvailable:    3000000 kB\n",
            &mut memory,
        );
        assert_eq!(memory.total_bytes, Some(8_000_000 * 1024));
        assert_eq!(memory.available_bytes, Some(3_000_000 * 1024));
    }

    #[test]
    fn parses_network_and_screen() {
        assert_eq!(
            parse_active_interface("default via 192.168.1.1 dev wlan0 proto dhcp\n"),
            Some("wlan0".to_string())
        );
        assert_eq!(
            parse_wakefulness("mWakefulness=Awake\nmWakefulnessChanging=false\n"),
            Some(true)
        );
        assert_eq!(parse_locked("mDreamingLockscreen=true\n"), Some(true));
        assert_eq!(parse_locked("nothing"), None);
    }
}
