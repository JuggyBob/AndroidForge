use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRef {
    pub serial: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Check {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default)]
    pub impact: Option<String>,
    #[serde(default)]
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceList {
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    #[serde(default)]
    pub project_root: Option<String>,
    #[serde(default)]
    pub lease_expires_at: Option<String>,
    #[serde(default)]
    pub lease_ttl_seconds: Option<u64>,
    #[serde(default)]
    pub lease_expired: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandLine {
    #[serde(default)]
    pub argv: Vec<String>,
}

impl CommandLine {
    pub fn display(&self) -> String {
        self.argv
            .iter()
            .map(|arg| {
                if arg.contains(' ') || arg.contains('\t') {
                    format!("'{}'", arg.replace('\'', "''"))
                } else {
                    arg.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NextAction {
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub priority: String,
    #[serde(default)]
    pub command: CommandLine,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default)]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Diagnosis {
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub confidence: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub failed_operation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvidenceRef {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub artifact_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiagnosisExplanation {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub project_root: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub diagnosis: Diagnosis,
    #[serde(default)]
    pub alternatives: Vec<Diagnosis>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
    #[serde(default)]
    pub next_actions: Vec<NextAction>,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub uncertainty: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepairAction {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub requires_confirmation: bool,
    #[serde(default)]
    pub requires_admin: bool,
    #[serde(default)]
    pub requires_network: bool,
    #[serde(default)]
    pub modifies_environment: bool,
}

impl RepairAction {
    pub fn command_display(&self) -> Option<String> {
        self.command.as_ref().map(|argv| {
            argv.iter()
                .map(|arg| {
                    if arg.contains(' ') {
                        format!("'{}'", arg.replace('\'', "''"))
                    } else {
                        arg.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepairIssue {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub detected: String,
    #[serde(default)]
    pub component: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub repair: RepairAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepairSummary {
    #[serde(default)]
    pub blocking: usize,
    #[serde(default)]
    pub recommended: usize,
    #[serde(default)]
    pub optional: usize,
    #[serde(default)]
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepairPlan {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub summary: RepairSummary,
    #[serde(default)]
    pub issues: Vec<RepairIssue>,
    #[serde(default)]
    pub next_actions: Vec<NextAction>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionList {
    #[serde(default)]
    pub active_only: bool,
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunList {
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub runs: Vec<RunSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FixtureList {
    #[serde(default)]
    pub fixtures: Vec<FixtureInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookList {
    #[serde(default)]
    pub hooks: Vec<HookInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FixturesSection {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ConformanceSection {
    #[serde(default)]
    pub flow: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallResult {
    #[serde(default)]
    pub build: BuildIdentity,
    #[serde(default)]
    pub fresh: bool,
    #[serde(default)]
    pub debounced: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManifestEntry {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FieldChange {
    #[serde(default)]
    pub field: String,
    #[serde(default)]
    pub current: String,
    #[serde(default)]
    pub baseline: String,
    #[serde(default)]
    pub changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StepComparison {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub current_status: Option<String>,
    #[serde(default)]
    pub baseline_status: Option<String>,
    #[serde(default)]
    pub current_duration_ms: Option<u64>,
    #[serde(default)]
    pub baseline_duration_ms: Option<u64>,
    #[serde(default)]
    pub duration_delta_ms: Option<i64>,
    #[serde(default)]
    pub current_failure: Option<String>,
    #[serde(default)]
    pub baseline_failure: Option<String>,
    #[serde(default)]
    pub change: String,
    #[serde(default)]
    pub current_artifacts: usize,
    #[serde(default)]
    pub baseline_artifacts: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunComparison {
    #[serde(default)]
    pub current_run_id: String,
    #[serde(default)]
    pub baseline_run_id: String,
    #[serde(default)]
    pub comparable: bool,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub build_changes: Vec<FieldChange>,
    #[serde(default)]
    pub environment_changes: Vec<FieldChange>,
    #[serde(default)]
    pub step_changes: Vec<StepComparison>,
    #[serde(default)]
    pub new_failures: Vec<String>,
    #[serde(default)]
    pub resolved_failures: Vec<String>,
    #[serde(default)]
    pub artifacts_added: Vec<String>,
    #[serde(default)]
    pub artifacts_removed: Vec<String>,
    #[serde(default)]
    pub current_status: String,
    #[serde(default)]
    pub baseline_status: String,
    #[serde(default)]
    pub current_duration_ms: u64,
    #[serde(default)]
    pub baseline_duration_ms: u64,
    #[serde(default)]
    pub duration_delta_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlakyObservation {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub run_status: String,
    #[serde(default)]
    pub step_status: String,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub failure_code: Option<String>,
    #[serde(default)]
    pub failure_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlakyStep {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub step_name: String,
    #[serde(default)]
    pub executions: usize,
    #[serde(default)]
    pub successes: usize,
    #[serde(default)]
    pub failures: usize,
    #[serde(default)]
    pub skipped: usize,
    #[serde(default)]
    pub sufficient_evidence: bool,
    #[serde(default)]
    pub classification: String,
    #[serde(default)]
    pub heuristic: String,
    #[serde(default)]
    pub observations: Vec<FlakyObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlakyReport {
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub runs_analyzed: usize,
    #[serde(default)]
    pub min_runs: usize,
    #[serde(default)]
    pub heuristic: String,
    #[serde(default)]
    pub steps: Vec<FlakyStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FrameResult {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub captured_at: String,
    #[serde(default)]
    pub display_size: Option<(u32, u32)>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LogcatPeek {
    #[serde(default)]
    pub lines: Vec<String>,
    #[serde(default)]
    pub line_count: usize,
    #[serde(default)]
    pub raw_bytes: u64,
    #[serde(default)]
    pub pid_filter: Vec<u32>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub captured_at: String,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Rect {
    #[serde(default)]
    pub left: i32,
    #[serde(default)]
    pub top: i32,
    #[serde(default)]
    pub right: i32,
    #[serde(default)]
    pub bottom: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiNode {
    #[serde(default)]
    pub id: usize,
    #[serde(default)]
    pub parent: Option<usize>,
    #[serde(default)]
    pub children: Vec<usize>,
    #[serde(default)]
    pub depth: usize,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub content_desc: Option<String>,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub resource_id: Option<String>,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub bounds: Rect,
    #[serde(default)]
    pub bounds_raw: String,
    #[serde(default)]
    pub clickable: bool,
    #[serde(default)]
    pub long_clickable: bool,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub selected: bool,
    #[serde(default)]
    pub scrollable: bool,
    #[serde(default)]
    pub checked: bool,
    #[serde(default)]
    pub password: bool,
    #[serde(default)]
    pub displayed: Option<bool>,
    #[serde(default)]
    pub index_attr: Option<String>,
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiHierarchy {
    #[serde(default)]
    pub xml: String,
    #[serde(default)]
    pub rotation: Option<i32>,
    #[serde(default)]
    pub nodes: Vec<UiNode>,
    #[serde(default)]
    pub root: Option<usize>,
    #[serde(default)]
    pub package_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiInspectResult {
    #[serde(default)]
    pub hierarchy: UiHierarchy,
    #[serde(default)]
    pub output_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReconcileEntry {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub previous_status: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReconcileResult {
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub reconciled: Vec<ReconcileEntry>,
    #[serde(default)]
    pub scanned: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunStorage {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionStorage {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub serial: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MaintenanceSummary {
    #[serde(default)]
    pub state_dir: String,
    #[serde(default)]
    pub artifacts_dir: String,
    #[serde(default)]
    pub state_bytes: u64,
    #[serde(default)]
    pub artifacts_bytes: u64,
    #[serde(default)]
    pub total_bytes: u64,
    #[serde(default)]
    pub session_count: usize,
    #[serde(default)]
    pub active_session_count: usize,
    #[serde(default)]
    pub terminal_session_count: usize,
    #[serde(default)]
    pub stale_session_count: usize,
    #[serde(default)]
    pub session_bytes: u64,
    #[serde(default)]
    pub run_count: usize,
    #[serde(default)]
    pub running_run_count: usize,
    #[serde(default)]
    pub terminal_run_count: usize,
    #[serde(default)]
    pub run_bytes: u64,
    #[serde(default)]
    pub oldest_run_started_at: Option<String>,
    #[serde(default)]
    pub largest_runs: Vec<RunStorage>,
    #[serde(default)]
    pub terminal_sessions: Vec<SessionStorage>,
}
