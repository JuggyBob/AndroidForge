use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Device,
    Offline,
    Unauthorized,
    Bootloader,
    Recovery,
    Sideload,
    Connecting,
    Authorizing,
    Unknown,
}

impl DeviceState {
    pub fn from_adb(state: &str) -> Self {
        match state {
            "device" => Self::Device,
            "offline" => Self::Offline,
            "unauthorized" => Self::Unauthorized,
            "bootloader" => Self::Bootloader,
            "recovery" => Self::Recovery,
            "sideload" => Self::Sideload,
            "connecting" => Self::Connecting,
            "authorizing" => Self::Authorizing,
            _ => Self::Unknown,
        }
    }

    pub fn is_operational(&self) -> bool {
        matches!(self, Self::Device)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Offline => "offline",
            Self::Unauthorized => "unauthorized",
            Self::Bootloader => "bootloader",
            Self::Recovery => "recovery",
            Self::Sideload => "sideload",
            Self::Connecting => "connecting",
            Self::Authorizing => "authorizing",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Emulator,
    Physical,
    Unknown,
}

impl DeviceKind {
    pub fn from_adb(serial: &str, properties: &BTreeMap<String, String>) -> Self {
        if serial.starts_with("emulator-")
            || properties
                .get("ro.kernel.qemu")
                .map(|value| value == "1")
                .unwrap_or(false)
        {
            Self::Emulator
        } else if properties.contains_key("ro.product.model")
            || serial.chars().any(|c| c.is_ascii_alphanumeric())
        {
            Self::Physical
        } else {
            Self::Unknown
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub serial: String,
    pub state: DeviceState,
    pub kind: DeviceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_completed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_on: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
}

impl Device {
    pub fn basic(serial: impl Into<String>, state: DeviceState) -> Self {
        let serial = serial.into();
        Self {
            state,
            kind: if serial.starts_with("emulator-") {
                DeviceKind::Emulator
            } else {
                DeviceKind::Unknown
            },
            serial,
            model: None,
            manufacturer: None,
            android_version: None,
            api_level: None,
            architecture: None,
            resolution: None,
            density: None,
            product: None,
            transport_id: None,
            boot_completed: None,
            boot_state: None,
            screen_on: None,
            locked: None,
            properties: BTreeMap::new(),
        }
    }

    pub fn display_name(&self) -> String {
        let model = self.model.as_deref().unwrap_or("Android device");
        format!("{} ({})", model, self.serial)
    }

    pub fn short_summary(&self) -> String {
        let api = self
            .api_level
            .map(|level| format!("API {level}"))
            .unwrap_or_else(|| "API unknown".to_string());
        let boot = match self.boot_completed {
            Some(true) => "booted",
            Some(false) => "booting",
            None => "boot unknown",
        };
        format!(
            "{} [{}] {} {}",
            self.serial,
            self.state.as_str(),
            self.kind_label(),
            format_args!("{api}, {boot}")
        )
    }

    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            DeviceKind::Emulator => "emulator",
            DeviceKind::Physical => "physical",
            DeviceKind::Unknown => "unknown",
        }
    }

    /// Device summaries and run metadata deliberately omit the full getprop
    /// dump. `devices inspect` still exposes it on request.
    pub fn compact(&self) -> Self {
        let mut device = self.clone();
        device.properties.clear();
        device
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apk_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apk_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_apk_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_apk_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_signed: Option<bool>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl BuildIdentity {
    pub fn for_package(package: impl Into<String>) -> Self {
        Self {
            package: Some(package.into()),
            ..Self::default()
        }
    }

    pub fn summary(&self) -> String {
        let package = self.package.as_deref().unwrap_or("(unknown package)");
        let version = match (&self.version_name, self.version_code) {
            (Some(name), Some(code)) => format!("{name} ({code})"),
            (Some(name), None) => name.clone(),
            (None, Some(code)) => code.to_string(),
            (None, None) => "version unknown".to_string(),
        };
        format!("{package} {version}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub kind: String,
    /// Path relative to the run directory whenever possible.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl ArtifactRef {
    pub fn new(kind: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            path: path.into(),
            sha256: None,
            size_bytes: None,
            description: None,
        }
    }

    pub fn with_size(mut self, size: u64) -> Self {
        self.size_bytes = Some(size);
        self
    }

    pub fn with_sha256(mut self, sha256: impl Into<String>) -> Self {
        self.sha256 = Some(sha256.into());
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl ToolInfo {
    pub fn missing(name: impl Into<String>, note: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            path: None,
            version: None,
            available: false,
            note: Some(note.into()),
        }
    }

    pub fn available(
        name: impl Into<String>,
        path: impl Into<String>,
        version: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            path: Some(path.into()),
            version,
            available: true,
            note: None,
        }
    }
}
