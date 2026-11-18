use std::time::Duration;

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DozeState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_on: Option<bool>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandbyState {
    pub package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub present: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ac_powered: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usb_powered: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wireless_powered: Option<bool>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub airplane_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wifi_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mobile_data_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_network: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    pub raw_connectivity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeState {
    pub device_time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_time: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_time_zone: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DeviceStateManager {
    pub adb: AdbClient,
}

impl DeviceStateManager {
    pub fn new(adb: AdbClient) -> Self {
        Self { adb }
    }

    pub async fn doze(&self, serial: &str) -> Result<DozeState> {
        let raw = self
            .adb
            .shell_args(
                serial,
                &["dumpsys".to_string(), "deviceidle".to_string()],
                Some(Duration::from_secs(20)),
                Some(8 * 1024 * 1024),
            )
            .await?
            .stdout_text();
        Ok(parse_doze(&raw))
    }

    pub async fn force_doze(&self, serial: &str, confirmed: bool) -> Result<DozeState> {
        if !confirmed {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                "forcing Doze changes device power state; pass --yes to confirm",
            )
            .with_suggestion("Use `androidforge doze status` to inspect state first.")
            .device(serial.to_string()));
        }
        let before = self.doze(serial).await?;
        if before.screen_on == Some(true) {
            let _ = self.adb.keyevent(serial, "KEYCODE_SLEEP", None).await;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let battery = self.battery(serial).await?;
        if battery.ac_powered == Some(true) || battery.usb_powered == Some(true) {
            let _ = self
                .adb
                .shell_args(
                    serial,
                    &[
                        "dumpsys".to_string(),
                        "battery".to_string(),
                        "unplug".to_string(),
                    ],
                    Some(Duration::from_secs(10)),
                    None,
                )
                .await;
        }
        let output = self
            .adb
            .shell_args(
                serial,
                &[
                    "cmd".to_string(),
                    "deviceidle".to_string(),
                    "force-idle".to_string(),
                ],
                Some(Duration::from_secs(20)),
                Some(2 * 1024 * 1024),
            )
            .await;
        let after = self.doze(serial).await?;
        if after.state.as_deref() == Some("IDLE") {
            return Ok(after);
        }
        let output_text = output
            .map(|output| output.combined_text())
            .unwrap_or_default();
        Err(ForgeError::new(
            ErrorCode::UnsupportedOperation,
            "the device did not enter Doze after force-idle",
        )
        .with_context_string("output", truncate(output_text.trim(), 3000))
        .with_context(
            "before",
            serde_json::to_value(&before).unwrap_or(serde_json::Value::Null),
        )
        .with_context(
            "after",
            serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
        )
        .with_suggestion(
            "Doze requires the screen off, battery unplugged and a supported Android build.",
        )
        .with_suggestion(
            "Use `androidforge adb shell dumpsys deviceidle` to inspect the raw state.",
        )
        .device(serial.to_string()))
    }

    pub async fn unforce_doze(&self, serial: &str) -> Result<DozeState> {
        let output = self
            .adb
            .shell_args(
                serial,
                &[
                    "cmd".to_string(),
                    "deviceidle".to_string(),
                    "unforce".to_string(),
                ],
                Some(Duration::from_secs(20)),
                Some(2 * 1024 * 1024),
            )
            .await;
        let _ = self
            .adb
            .shell_args(
                serial,
                &[
                    "dumpsys".to_string(),
                    "battery".to_string(),
                    "reset".to_string(),
                ],
                Some(Duration::from_secs(10)),
                None,
            )
            .await;
        let after = self.doze(serial).await?;
        if matches!(after.state.as_deref(), Some("ACTIVE")) {
            return Ok(after);
        }
        let message = output
            .map(|output| output.combined_text())
            .unwrap_or_default();
        Err(ForgeError::new(
            ErrorCode::CommandFailed,
            "the device did not leave Doze after unforce",
        )
        .with_context_string("output", truncate(message.trim(), 3000))
        .with_context(
            "state",
            serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
        )
        .device(serial.to_string()))
    }

    pub async fn standby(&self, serial: &str, package: &str) -> Result<StandbyState> {
        let output = self
            .adb
            .shell_args(
                serial,
                &[
                    "am".to_string(),
                    "get-standby-bucket".to_string(),
                    package.to_string(),
                ],
                Some(Duration::from_secs(15)),
                None,
            )
            .await?;
        let raw = output.stdout_text();
        let bucket = raw
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(normalize_bucket);
        Ok(StandbyState {
            package: package.to_string(),
            bucket,
            raw,
        })
    }

    pub async fn set_standby(
        &self,
        serial: &str,
        package: &str,
        bucket: &str,
    ) -> Result<StandbyState> {
        if !matches!(
            bucket,
            "active" | "working_set" | "frequent" | "rare" | "restricted" | "never"
        ) {
            return Err(ForgeError::invalid_argument(format!(
                "invalid standby bucket '{bucket}'"
            )));
        }
        let output = self
            .adb
            .shell_args(
                serial,
                &[
                    "am".to_string(),
                    "set-standby-bucket".to_string(),
                    package.to_string(),
                    bucket.to_string(),
                ],
                Some(Duration::from_secs(15)),
                Some(1024 * 1024),
            )
            .await;
        let after = self.standby(serial, package).await?;
        if after.bucket.as_deref().map(normalize_bucket_name) != Some(bucket) {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                "the device did not accept the requested standby bucket",
            )
            .with_context_string(
                "output",
                output
                    .map(|output| truncate(output.combined_text().trim(), 2000))
                    .unwrap_or_default(),
            )
            .with_context(
                "state",
                serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
            )
            .device(serial.to_string()));
        }
        Ok(after)
    }

    pub async fn battery(&self, serial: &str) -> Result<BatteryState> {
        let raw = self
            .adb
            .shell_args(
                serial,
                &["dumpsys".to_string(), "battery".to_string()],
                Some(Duration::from_secs(20)),
                Some(2 * 1024 * 1024),
            )
            .await?
            .stdout_text();
        Ok(parse_battery(&raw))
    }

    pub async fn battery_unplug(&self, serial: &str) -> Result<BatteryState> {
        self.adb
            .shell_args(
                serial,
                &[
                    "dumpsys".to_string(),
                    "battery".to_string(),
                    "unplug".to_string(),
                ],
                Some(Duration::from_secs(15)),
                None,
            )
            .await?;
        self.battery(serial).await
    }

    pub async fn battery_reset(&self, serial: &str) -> Result<BatteryState> {
        self.adb
            .shell_args(
                serial,
                &[
                    "dumpsys".to_string(),
                    "battery".to_string(),
                    "reset".to_string(),
                ],
                Some(Duration::from_secs(15)),
                None,
            )
            .await?;
        self.battery(serial).await
    }

    pub async fn proxy(&self, serial: &str) -> Result<ProxyState> {
        let raw = self
            .adb
            .settings_get(serial, "global", "http_proxy")
            .await
            .ok()
            .map(|value| value.trim().to_string());
        let proxy = raw
            .as_deref()
            .filter(|value| !value.is_empty() && *value != ":0")
            .map(str::to_string);
        Ok(ProxyState { proxy, raw })
    }

    pub async fn set_proxy(&self, serial: &str, proxy: Option<&str>) -> Result<ProxyState> {
        self.adb
            .settings_put(
                serial,
                "global",
                "http_proxy",
                proxy
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(":0"),
            )
            .await?;
        self.proxy(serial).await
    }

    pub async fn network(&self, serial: &str) -> Result<NetworkState> {
        let connectivity = self
            .adb
            .shell_args(
                serial,
                &["dumpsys".to_string(), "connectivity".to_string()],
                Some(Duration::from_secs(30)),
                Some(8 * 1024 * 1024),
            )
            .await
            .map(|output| output.stdout_text())
            .unwrap_or_default();
        let airplane = self
            .adb
            .settings_get(serial, "global", "airplane_mode_on")
            .await
            .ok()
            .map(|value| value.trim() == "1");
        let wifi = self
            .adb
            .settings_get(serial, "global", "wifi_on")
            .await
            .ok()
            .map(|value| value.trim() == "1");
        let mobile = self
            .adb
            .settings_get(serial, "global", "mobile_data")
            .await
            .ok()
            .map(|value| value.trim() == "1");
        let active = connectivity
            .lines()
            .find(|line| line.contains("Active default network:"))
            .map(|line| line.trim().to_string());
        let mut notes = Vec::new();
        if wifi.is_none() {
            notes.push("wifi_on setting could not be read on this Android build".to_string());
        }
        Ok(NetworkState {
            airplane_mode: airplane,
            wifi_enabled: wifi,
            mobile_data_enabled: mobile,
            active_network: active,
            notes,
            raw_connectivity: connectivity,
        })
    }

    pub async fn set_airplane(
        &self,
        serial: &str,
        enabled: bool,
        confirmed: bool,
    ) -> Result<NetworkState> {
        if !confirmed {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                "changing airplane mode is disruptive; pass --yes to confirm",
            )
            .device(serial.to_string()));
        }
        let mode = if enabled { "enable" } else { "disable" };
        let command = self
            .adb
            .shell_args(
                serial,
                &[
                    "cmd".to_string(),
                    "connectivity".to_string(),
                    "airplane-mode".to_string(),
                    mode.to_string(),
                ],
                Some(Duration::from_secs(15)),
                Some(1024 * 1024),
            )
            .await;
        if command.is_err() {
            let value = if enabled { "1" } else { "0" };
            self.adb
                .settings_put(serial, "global", "airplane_mode_on", value)
                .await?;
            let _ = self
                .adb
                .shell_args(
                    serial,
                    &[
                        "am".to_string(),
                        "broadcast".to_string(),
                        "-a".to_string(),
                        "android.intent.action.AIRPLANE_MODE".to_string(),
                        "--ez".to_string(),
                        "state".to_string(),
                        enabled.to_string(),
                    ],
                    Some(Duration::from_secs(15)),
                    Some(1024 * 1024),
                )
                .await;
        }
        let after = self.network(serial).await?;
        if after.airplane_mode != Some(enabled) {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                "the device did not accept the requested airplane-mode change",
            )
            .with_context(
                "state",
                serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
            )
            .with_suggestion("Some builds restrict this shell command; use a physical device setting or AppOps-aware hook.")
            .device(serial.to_string()));
        }
        Ok(after)
    }

    pub async fn set_wifi(&self, serial: &str, enabled: bool) -> Result<NetworkState> {
        self.adb
            .shell_args(
                serial,
                &[
                    "svc".to_string(),
                    "wifi".to_string(),
                    if enabled { "enable" } else { "disable" }.to_string(),
                ],
                Some(Duration::from_secs(20)),
                Some(1024 * 1024),
            )
            .await?;
        let after = self.network(serial).await?;
        if after.wifi_enabled != Some(enabled) {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                "the device did not confirm the requested Wi-Fi state",
            )
            .with_context(
                "state",
                serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
            )
            .device(serial.to_string()));
        }
        Ok(after)
    }

    pub async fn set_mobile_data(&self, serial: &str, enabled: bool) -> Result<NetworkState> {
        self.adb
            .shell_args(
                serial,
                &[
                    "svc".to_string(),
                    "data".to_string(),
                    if enabled { "enable" } else { "disable" }.to_string(),
                ],
                Some(Duration::from_secs(20)),
                Some(1024 * 1024),
            )
            .await?;
        let after = self.network(serial).await?;
        if after.mobile_data_enabled != Some(enabled) {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                "the device did not confirm the requested mobile-data state",
            )
            .with_context(
                "state",
                serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
            )
            .device(serial.to_string()));
        }
        Ok(after)
    }

    pub async fn time(&self, serial: &str) -> Result<TimeState> {
        let date = self
            .adb
            .shell_args(
                serial,
                &["date".to_string(), "+%Y-%m-%dT%H:%M:%S%z".to_string()],
                Some(Duration::from_secs(10)),
                None,
            )
            .await
            .map(|output| output.stdout_text().trim().to_string())
            .unwrap_or_default();
        let epoch = self
            .adb
            .shell_args(
                serial,
                &["date".to_string(), "+%s".to_string()],
                Some(Duration::from_secs(10)),
                None,
            )
            .await
            .map(|output| output.stdout_text().trim().parse::<i64>().ok())
            .unwrap_or(None);
        let auto_time = self
            .adb
            .settings_get(serial, "global", "auto_time")
            .await
            .ok()
            .map(|value| value.trim() == "1");
        let auto_time_zone = self
            .adb
            .settings_get(serial, "global", "auto_time_zone")
            .await
            .ok()
            .map(|value| value.trim() == "1");
        let timezone = self.adb.getprop(serial, "persist.sys.timezone").await.ok();
        Ok(TimeState {
            device_time: date,
            epoch_seconds: epoch,
            auto_time,
            auto_time_zone,
            timezone,
            changed: None,
            notes: Vec::new(),
        })
    }

    pub async fn set_time(
        &self,
        serial: &str,
        epoch_seconds: i64,
        confirmed: bool,
    ) -> Result<TimeState> {
        if !confirmed {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                "changing device time requires explicit confirmation; pass --yes",
            )
            .with_suggestion("Record the current state with `androidforge time get` first.")
            .with_suggestion("Use `androidforge time restore` afterwards.")
            .device(serial.to_string()));
        }
        let before = self.time(serial).await?;
        let output = self
            .adb
            .shell_args(
                serial,
                &["date".to_string(), format!("@{epoch_seconds}")],
                Some(Duration::from_secs(20)),
                Some(1024 * 1024),
            )
            .await;
        let mut after = self.time(serial).await?;
        let matched = after
            .epoch_seconds
            .map(|actual| (actual - epoch_seconds).abs() <= 5)
            .unwrap_or(false);
        if !matched {
            let message = output
                .map(|output| output.combined_text())
                .unwrap_or_default();
            return Err(ForgeError::new(
                ErrorCode::PermissionDenied,
                "the device rejected the time change",
            )
            .with_context_string("output", truncate(message.trim(), 2000))
            .with_context(
                "before",
                serde_json::to_value(&before).unwrap_or(serde_json::Value::Null),
            )
            .with_context(
                "after",
                serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
            )
            .with_suggestion("Android normally requires root for `date` changes; AndroidForge will not root a device automatically.")
            .with_suggestion("Use a userdebug emulator with `adb root`, or use an app hook for scheduled-work tests.")
            .device(serial.to_string()));
        }
        after.changed = Some(true);
        Ok(after)
    }

    pub async fn set_auto_time(
        &self,
        serial: &str,
        auto_time: bool,
        auto_time_zone: bool,
    ) -> Result<TimeState> {
        self.adb
            .settings_put(
                serial,
                "global",
                "auto_time",
                if auto_time { "1" } else { "0" },
            )
            .await?;
        self.adb
            .settings_put(
                serial,
                "global",
                "auto_time_zone",
                if auto_time_zone { "1" } else { "0" },
            )
            .await?;
        let after = self.time(serial).await?;
        if after.auto_time != Some(auto_time) || after.auto_time_zone != Some(auto_time_zone) {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                "the device did not confirm the automatic-time settings",
            )
            .with_context(
                "state",
                serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
            )
            .device(serial.to_string()));
        }
        Ok(after)
    }
}

fn parse_doze(raw: &str) -> DozeState {
    let mut state = None;
    let mut light_state = None;
    let mut charging = None;
    let mut screen_on = None;
    for token in raw.split_whitespace() {
        if let Some(value) = token.strip_prefix("mState=") {
            state = Some(
                value
                    .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                    .to_string(),
            );
        } else if let Some(value) = token.strip_prefix("mLightState=") {
            light_state = Some(
                value
                    .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                    .to_string(),
            );
        } else if let Some(value) = token.strip_prefix("mCharging=") {
            charging = value
                .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .parse::<bool>()
                .ok();
        } else if let Some(value) = token.strip_prefix("mScreenOn=") {
            screen_on = value
                .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .parse::<bool>()
                .ok();
        }
    }
    DozeState {
        state,
        light_state,
        charging,
        screen_on,
        raw: raw.to_string(),
    }
}

fn parse_battery(raw: &str) -> BatteryState {
    let mut state = BatteryState {
        level: None,
        status: None,
        health: None,
        present: None,
        ac_powered: None,
        usb_powered: None,
        wireless_powered: None,
        raw: raw.to_string(),
    };
    for line in raw.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("level:") {
            state.level = value.trim().parse::<u32>().ok();
        } else if let Some(value) = line.strip_prefix("status:") {
            state.status = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("health:") {
            state.health = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("present:") {
            state.present = value.trim().parse::<bool>().ok();
        } else if let Some(value) = line.strip_prefix("AC powered:") {
            state.ac_powered = value.trim().parse::<bool>().ok();
        } else if let Some(value) = line.strip_prefix("USB powered:") {
            state.usb_powered = value.trim().parse::<bool>().ok();
        } else if let Some(value) = line.strip_prefix("Wireless powered:") {
            state.wireless_powered = value.trim().parse::<bool>().ok();
        }
    }
    state
}

fn normalize_bucket(value: &str) -> String {
    match value.trim() {
        "10" => "active".to_string(),
        "20" => "working_set".to_string(),
        "30" => "frequent".to_string(),
        "40" => "rare".to_string(),
        "45" => "restricted".to_string(),
        "50" => "never".to_string(),
        other => other.to_string(),
    }
}

fn normalize_bucket_name(value: &str) -> &str {
    match value {
        "10" => "active",
        "20" => "working_set",
        "30" => "frequent",
        "40" => "rare",
        "45" => "restricted",
        "50" => "never",
        other => other,
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect::<String>() + "\n[truncated]"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_doze_state() {
        let state = parse_doze("mState=IDLE\nmLightState=ACTIVE\nmCharging=false\n");
        assert_eq!(state.state.as_deref(), Some("IDLE"));
        assert_eq!(state.light_state.as_deref(), Some("ACTIVE"));
        assert_eq!(state.charging, Some(false));
    }

    #[test]
    fn parses_battery_state() {
        let state = parse_battery(
            "AC powered: false\nUSB powered: true\nWireless powered: false\nlevel: 80\nstatus: 2\npresent: true\n",
        );
        assert_eq!(state.level, Some(80));
        assert_eq!(state.ac_powered, Some(false));
        assert_eq!(state.usb_powered, Some(true));
        assert_eq!(state.present, Some(true));
    }

    #[test]
    fn maps_standby_bucket_numbers() {
        assert_eq!(normalize_bucket("10"), "active");
        assert_eq!(normalize_bucket("45"), "restricted");
        assert_eq!(normalize_bucket_name("working_set"), "working_set");
    }
}
