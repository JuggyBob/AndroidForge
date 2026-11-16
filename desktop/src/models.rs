use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct Envelope {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub exit_code: i32,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub device: Option<DeviceRef>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<ErrorBody>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceRef {
    pub serial: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ErrorBody {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub recoverable: bool,
    #[serde(default)]
    pub suggestions: Vec<String>,
    #[serde(default)]
    pub context: BTreeMap<String, Value>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub device_serial: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub cause: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Check {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Tool {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DoctorResult {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub avds: Vec<String>,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub project_config: Option<String>,
    #[serde(default)]
    pub project_root: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DeviceOwner {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Device {
    #[serde(default)]
    pub serial: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub android_version: Option<String>,
    #[serde(default)]
    pub api_level: Option<u32>,
    #[serde(default)]
    pub architecture: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub boot_state: Option<String>,
    #[serde(default)]
    pub boot_completed: Option<bool>,
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub density: Option<u32>,
    #[serde(default)]
    pub screen_on: Option<bool>,
    #[serde(default)]
    pub transport_id: Option<String>,
    #[serde(default)]
    pub owner: Option<DeviceOwner>,
    #[serde(default)]
    pub avd: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub battery_level: Option<u32>,
    #[serde(default)]
    pub battery: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DeviceList {
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub serial: Option<String>,
    #[serde(default)]
    pub avd: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub emulator_pid: Option<u32>,
    #[serde(default)]
    pub emulator_process_alive: Option<bool>,
    #[serde(default)]
    pub headless: Option<bool>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub heartbeat_at: Option<String>,
    #[serde(default)]
    pub age_seconds: u64,
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub operation_active: bool,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub device_settings: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub snapshots: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionList {
    #[serde(default)]
    pub active_only: bool,
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ArtifactRef {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BuildIdentity {
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub version_name: Option<String>,
    #[serde(default)]
    pub version_code: Option<i64>,
    #[serde(default)]
    pub apk_path: Option<String>,
    #[serde(default)]
    pub apk_sha256: Option<String>,
    #[serde(default)]
    pub installed_apk_path: Option<String>,
    #[serde(default)]
    pub installed_apk_sha256: Option<String>,
    #[serde(default)]
    pub cert_sha256: Option<String>,
    #[serde(default)]
    pub signer_dn: Option<String>,
    #[serde(default)]
    pub debug_signed: Option<bool>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FailureInfo {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub failed_operation: Option<String>,
    #[serde(default)]
    pub recoverable: bool,
    #[serde(default)]
    pub suggestions: Vec<String>,
    #[serde(default)]
    pub context: BTreeMap<String, Value>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Step {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub ended_at: String,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub details: Value,
    #[serde(default)]
    pub failure: Option<FailureInfo>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub retries: u32,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RunReport {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub project_root: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub exit_code: i32,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub ended_at: String,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub device_serial: Option<String>,
    #[serde(default)]
    pub device: Option<Value>,
    #[serde(default)]
    pub build: BuildIdentity,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub failure: Option<FailureInfo>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RunSummary {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub exit_code: i32,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub steps: usize,
    #[serde(default)]
    pub failed_steps: usize,
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RunList {
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub runs: Vec<RunSummary>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ReportPath {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub report: String,
    #[serde(default)]
    pub manifest: String,
    #[serde(default)]
    pub metadata: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FixtureInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub valid: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FixtureList {
    #[serde(default)]
    pub fixtures: Vec<FixtureInfo>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct HookInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub idempotent: bool,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct HookList {
    #[serde(default)]
    pub hooks: Vec<HookInfo>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ProjectConfig {
    #[serde(default)]
    pub adapter_version: u32,
    #[serde(default)]
    pub project: ProjectSection,
    #[serde(default)]
    pub fixtures: FixturesSection,
    #[serde(default)]
    pub conformance: ConformanceSection,
    #[serde(default)]
    pub min_androidforge_version: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ProjectSection {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub debug_package: Option<String>,
    #[serde(default)]
    pub expected_cert_sha256: Vec<String>,
    #[serde(default)]
    pub default_avd: Option<String>,
    #[serde(default)]
    pub default_device: Option<String>,
    #[serde(default)]
    pub artifact_dir: Option<String>,
    #[serde(default)]
    pub verify_installed_apk: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct FixturesSection {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ConformanceSection {
    #[serde(default)]
    pub flow: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SignatureInfo {
    #[serde(default)]
    pub cert_sha256: Option<String>,
    #[serde(default)]
    pub signer_dn: Option<String>,
    #[serde(default)]
    pub signed: Option<bool>,
    #[serde(default)]
    pub verified: Option<bool>,
    #[serde(default)]
    pub v1: Option<bool>,
    #[serde(default)]
    pub v2: Option<bool>,
    #[serde(default)]
    pub v3: Option<bool>,
    #[serde(default)]
    pub debug_signed: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ApkInfo {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub version_name: Option<String>,
    #[serde(default)]
    pub version_code: Option<i64>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub min_sdk: Option<u32>,
    #[serde(default)]
    pub target_sdk: Option<u32>,
    #[serde(default)]
    pub launchable_activity: Option<String>,
    #[serde(default)]
    pub architectures: Vec<String>,
    #[serde(default)]
    pub signature: SignatureInfo,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct InstallResult {
    #[serde(default)]
    pub build: BuildIdentity,
    #[serde(default)]
    pub fresh: bool,
    #[serde(default)]
    pub debounced: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ManifestEntry {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Manifest {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub files: Vec<ManifestEntry>,
}
