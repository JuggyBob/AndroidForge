//! Deterministic failure classification and repair suggestions for agent-facing
//! reports. This module never calls an LLM and never invents an outcome: every
//! suggestion is derived from the report's structured failure data.

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::ErrorCode;
use crate::model::ArtifactRef;
use crate::run::{FailureInfo, RunReport, RunStatus, RunStep};
use crate::util::{now_rfc3339, sha256_hex};

pub const DIAGNOSIS_SCHEMA_VERSION: u32 = 1;
pub const FINGERPRINT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisCategory {
    Environment,
    Device,
    Emulator,
    Install,
    Signature,
    Application,
    Process,
    Task,
    Anr,
    Selector,
    SelectorAmbiguous,
    UiAutomation,
    Timeout,
    Assertion,
    Permission,
    Configuration,
    Cancelled,
    Interrupted,
    Unknown,
}

impl DiagnosisCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::Device => "device",
            Self::Emulator => "emulator",
            Self::Install => "install",
            Self::Signature => "signature",
            Self::Application => "application",
            Self::Process => "process",
            Self::Task => "task",
            Self::Anr => "anr",
            Self::Selector => "selector",
            Self::SelectorAmbiguous => "selector_ambiguous",
            Self::UiAutomation => "ui_automation",
            Self::Timeout => "timeout",
            Self::Assertion => "assertion",
            Self::Permission => "permission",
            Self::Configuration => "configuration",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
    InsufficientEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandLine {
    pub argv: Vec<String>,
}

impl CommandLine {
    pub fn new(args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            argv: args.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextAction {
    pub reason: String,
    pub priority: String,
    pub command: CommandLine,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub source: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnosis {
    pub category: DiagnosisCategory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub confidence: Confidence,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_operation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticExplanation {
    pub schema_version: u32,
    pub generated_at: String,
    pub run_id: String,
    pub project: String,
    pub project_root: String,
    pub scenario: String,
    pub status: RunStatus,
    pub diagnosis: Diagnosis,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Diagnosis>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceRef>,
    pub next_actions: Vec<NextAction>,
    pub fingerprint: String,
    pub fingerprint_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncertainty: Vec<String>,
}

impl DiagnosticExplanation {
    pub fn primary_code(&self) -> Option<&str> {
        self.diagnosis.code.as_deref()
    }
}

fn is_aggregate_code(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::CommandFailed | ErrorCode::Internal | ErrorCode::Interrupted
    )
}

fn failure_for(report: &RunReport) -> Option<(&FailureInfo, Option<&RunStep>)> {
    let step_failures = report
        .steps
        .iter()
        .filter_map(|step| step.failure.as_ref().map(|failure| (failure, step)))
        .collect::<Vec<_>>();
    if let Some(failure) = &report.failure {
        // Some run-level failures aggregate an external command exit code while
        // the meaningful failure is a specific step failure (for example a UI
        // assertion inside a conformance script).
        if is_aggregate_code(failure.code)
            && let Some((step_failure, step)) = step_failures
                .iter()
                .find(|(candidate, _)| !is_aggregate_code(candidate.code))
        {
            return Some((*step_failure, Some(*step)));
        }
        // Prefer the step object when it carries the same stable code; it has
        // the operation-specific context and artifacts.
        if let Some((step_failure, step)) = step_failures
            .iter()
            .find(|(candidate, _)| candidate.code == failure.code)
        {
            return Some((*step_failure, Some(*step)));
        }
        return Some((failure, None));
    }
    step_failures
        .first()
        .map(|(failure, step)| (*failure, Some(*step)))
}

fn package_from(report: &RunReport) -> Option<String> {
    report.build.package.clone().or_else(|| {
        failure_for(report).and_then(|(failure, _)| {
            failure
                .context
                .get("package")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
    })
}

fn selector_text(failure: &FailureInfo) -> Option<String> {
    let selector = failure.context.get("selector")?;
    for key in ["text", "text_contains", "content_desc", "resource_id"] {
        if let Some(value) = selector.get(key).and_then(|value| value.as_str())
            && !value.is_empty()
        {
            return Some(value.to_string());
        }
    }
    None
}

fn selector_index(failure: &FailureInfo) -> Option<u64> {
    failure
        .context
        .get("selector")
        .and_then(|selector| selector.get("index"))
        .and_then(|value| value.as_u64())
}

fn category_for_code(code: ErrorCode) -> Option<DiagnosisCategory> {
    use DiagnosisCategory as C;
    match code {
        ErrorCode::SdkNotFound
        | ErrorCode::AdbNotFound
        | ErrorCode::ToolNotFound
        | ErrorCode::MaestroNotFound => Some(C::Environment),
        ErrorCode::DeviceNotFound
        | ErrorCode::DeviceAmbiguous
        | ErrorCode::DeviceOffline
        | ErrorCode::DeviceUnauthorized
        | ErrorCode::DeviceBusy
        | ErrorCode::DeviceDisconnected
        | ErrorCode::SessionExpired
        | ErrorCode::SessionOwnershipMismatch
        | ErrorCode::SessionNotFound
        | ErrorCode::LeaseExpired => Some(C::Device),
        ErrorCode::EmulatorNotFound | ErrorCode::EmulatorBootTimeout => Some(C::Emulator),
        ErrorCode::InstallFailed
        | ErrorCode::InstallVerificationFailed
        | ErrorCode::PackageNotFound => Some(C::Install),
        ErrorCode::InstallSignatureMismatch | ErrorCode::SignatureVerificationFailed => {
            Some(C::Signature)
        }
        ErrorCode::SelectorNotFound => Some(C::Selector),
        ErrorCode::SelectorAmbiguous => Some(C::SelectorAmbiguous),
        ErrorCode::UiAssertionFailed | ErrorCode::TextVerificationFailed => Some(C::Assertion),
        ErrorCode::UiHierarchyInvalid => Some(C::Application),
        ErrorCode::UiAutomatorUnavailable => Some(C::UiAutomation),
        ErrorCode::CommandTimeout => Some(C::Timeout),
        ErrorCode::PermissionDenied => Some(C::Permission),
        ErrorCode::ConfigInvalid
        | ErrorCode::AdapterInvalid
        | ErrorCode::FixtureInvalid
        | ErrorCode::JunitInvalid
        | ErrorCode::InvalidArgument
        | ErrorCode::InvalidCommand => Some(C::Configuration),
        ErrorCode::Interrupted => Some(C::Interrupted),
        ErrorCode::PostconditionTimeout => Some(C::Assertion),
        ErrorCode::ObservationNotFound
        | ErrorCode::ObservationExpired
        | ErrorCode::ObservationSessionMismatch => Some(C::Configuration),
        _ => None,
    }
}

fn category_for_failure(failure: &FailureInfo) -> (DiagnosisCategory, Confidence) {
    let message = failure.message.to_ascii_lowercase();
    // Install errors are sometimes reported with the aggregate install code
    // even when the device rejected the signing identity.
    if message.contains("signatures do not match")
        || message.contains("signature mismatch")
        || message.contains("install_failed_update_incompatible")
    {
        return (DiagnosisCategory::Signature, Confidence::High);
    }
    if message.contains("uiautomator") && !message.contains("not found") {
        return (DiagnosisCategory::UiAutomation, Confidence::High);
    }
    if message.contains("adb command failed with exit code 137") {
        return (DiagnosisCategory::UiAutomation, Confidence::Medium);
    }
    if let Some(category) = category_for_code(failure.code) {
        return (category, Confidence::High);
    }
    if message.contains("anr") || message.contains("not responding") {
        return (DiagnosisCategory::Anr, Confidence::High);
    }
    if message.contains("fatal exception")
        || message.contains("has died")
        || message.contains("process crashed")
        || message.contains("native crash")
    {
        return (DiagnosisCategory::Application, Confidence::High);
    }
    if message.contains("task") && (message.contains("missing") || message.contains("disappeared"))
    {
        return (DiagnosisCategory::Task, Confidence::Medium);
    }
    if message.contains("device")
        && (message.contains("disconnected") || message.contains("offline"))
    {
        return (DiagnosisCategory::Device, Confidence::Medium);
    }
    if message.contains("timed out") || message.contains("timeout") {
        return (DiagnosisCategory::Timeout, Confidence::Medium);
    }
    (DiagnosisCategory::Unknown, Confidence::Low)
}

fn normalize_message(message: &str) -> String {
    let mut text = message.to_ascii_lowercase();
    let temporary_path = Regex::new(r"[a-z]:\\[^\s]*").expect("valid path regex");
    text = temporary_path.replace_all(&text, "<path>").into_owned();
    let memory = Regex::new(r"0x[0-9a-f]+").expect("valid hex regex");
    text = memory.replace_all(&text, "<address>").into_owned();
    let number = Regex::new(r"\b\d{2,}\b").expect("valid number regex");
    text = number.replace_all(&text, "<n>").into_owned();
    let whitespace = Regex::new(r"\s+").expect("valid whitespace regex");
    whitespace.replace_all(&text, " ").trim().to_string()
}

fn fingerprint(
    category: DiagnosisCategory,
    failure: Option<&FailureInfo>,
    report: &RunReport,
    package: Option<&str>,
) -> String {
    let code = failure.map(|failure| failure.code.as_str()).unwrap_or("-");
    let operation = failure
        .map(|failure| failure.failed_operation.as_str())
        .unwrap_or("-");
    let message = failure
        .map(|failure| normalize_message(&failure.message))
        .unwrap_or_default();
    let selector = failure.and_then(selector_text).unwrap_or_default();
    let step = failure
        .and_then(|failure| {
            report.steps.iter().find(|step| {
                step.failure.as_ref().is_some_and(|candidate| {
                    candidate.code == failure.code
                        && candidate.failed_operation == failure.failed_operation
                        && candidate.message == failure.message
                })
            })
        })
        .map(|step| step.name.clone())
        .unwrap_or_default();
    let material = format!(
        "v{FINGERPRINT_VERSION}|{}|{code}|{operation}|{}|{selector}|{step}|{message}",
        category.as_str(),
        package.unwrap_or("-")
    );
    format!("sha256:{}", sha256_hex(material.as_bytes()))
}

fn argv(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn action(
    reason: impl Into<String>,
    priority: &str,
    args: impl IntoIterator<Item = impl Into<String>>,
) -> NextAction {
    NextAction {
        reason: reason.into(),
        priority: priority.to_string(),
        command: CommandLine::new(args),
        destructive: false,
        warning: None,
    }
}

fn session_arg(report: &RunReport) -> Vec<String> {
    report
        .session_id
        .as_ref()
        .map(|session| vec!["--session".to_string(), session.clone()])
        .unwrap_or_default()
}

fn next_actions(
    report: &RunReport,
    category: DiagnosisCategory,
    failure: Option<&FailureInfo>,
) -> Vec<NextAction> {
    let session = session_arg(report);
    let package = package_from(report);
    let operation = failure.map(|failure| failure.failed_operation.as_str());
    let project = report.project_root.as_str();
    let selector = failure.and_then(selector_text);
    let apk_path = failure
        .and_then(|failure| failure.context.get("apk_path"))
        .and_then(|value| value.as_str())
        .or(report.build.apk_path.as_deref());
    let mut actions = Vec::new();
    match category {
        DiagnosisCategory::Selector => {
            let mut observe = argv(&["androidforge", "ui", "observe"]);
            observe.extend(session.clone());
            observe.push("--json".to_string());
            actions.push(action(
                "Re-observe the screen before choosing a different semantic selector.",
                "high",
                observe,
            ));
            if let Some(text) = &selector {
                let mut find = argv(&["androidforge", "ui", "find", "--contains", text.as_str()]);
                find.extend(session.clone());
                find.push("--json".to_string());
                actions.push(action(
                    "Search the hierarchy for a partial or updated label.",
                    "normal",
                    find,
                ));
            }
            let mut dump = argv(&["androidforge", "ui", "dump"]);
            dump.extend(session.clone());
            dump.push("--json".to_string());
            actions.push(action(
                "Dump the full hierarchy when the compact observation is insufficient.",
                "normal",
                dump,
            ));
        }
        DiagnosisCategory::SelectorAmbiguous => {
            let mut observe = argv(&["androidforge", "ui", "observe"]);
            observe.extend(session.clone());
            observe.push("--json".to_string());
            actions.push(action(
                "Inspect the compact observation and choose a more specific selector.",
                "high",
                observe,
            ));
            if let Some(text) = &selector {
                let mut find = argv(&["androidforge", "ui", "find"]);
                find.extend(session.clone());
                find.extend(["--text".to_string(), text.clone(), "--json".to_string()]);
                actions.push(action(
                    "List matching candidates and use --index deliberately.",
                    "normal",
                    find,
                ));
            }
            if let Some(index) = failure.and_then(selector_index)
                && let Some(text) = &selector
            {
                let mut tap = argv(&["androidforge", "ui", "tap"]);
                tap.extend(session.clone());
                tap.extend([
                    "--text".to_string(),
                    text.clone(),
                    "--index".to_string(),
                    index.to_string(),
                    "--json".to_string(),
                ]);
                actions.push(action(
                    "Retry with the candidate index only after inspecting the matches.",
                    "normal",
                    tap,
                ));
            }
        }
        DiagnosisCategory::Device | DiagnosisCategory::Emulator => {
            actions.push(action(
                "Inspect connected devices and the current AndroidForge sessions.",
                "high",
                ["androidforge", "devices", "list", "--json"],
            ));
            if let Some(session) = &report.session_id {
                actions.push(action(
                    "Inspect the session before attempting recovery.",
                    "high",
                    [
                        "androidforge",
                        "session",
                        "inspect",
                        "--session",
                        session.as_str(),
                        "--json",
                    ],
                ));
                actions.push(action(
                    "Restart the owned session if the device is gone and the owner is known.",
                    "normal",
                    [
                        "androidforge",
                        "session",
                        "restart",
                        "--session",
                        session.as_str(),
                        "--json",
                    ],
                ));
            }
            if category == DiagnosisCategory::Emulator {
                if let Some(session) = &report.session_id {
                    actions.push(action(
                        "Inspect the emulator log path recorded by the session.",
                        "normal",
                        [
                            "androidforge",
                            "session",
                            "inspect",
                            "--session",
                            session.as_str(),
                            "--json",
                        ],
                    ));
                } else {
                    actions.push(action(
                        "List sessions to find the emulator log path.",
                        "normal",
                        ["androidforge", "session", "list", "--json"],
                    ));
                }
            }
        }
        DiagnosisCategory::UiAutomation => {
            let mut observe = argv(&["androidforge", "ui", "observe"]);
            observe.extend(session.clone());
            observe.push("--json".to_string());
            actions.push(action(
                "Retry the observation; AndroidForge now attempts to clear a stuck UIAutomator helper automatically.",
                "high",
                observe,
            ));
            let mut diagnostics = argv(&["androidforge", "diagnostics", "collect", "--json"]);
            diagnostics.extend(session.clone());
            actions.push(action(
                "Collect bounded device diagnostics if the helper failure repeats.",
                "normal",
                diagnostics,
            ));
            if let Some(session_id) = &report.session_id {
                actions.push(action(
                    "Inspect the session and device state.",
                    "normal",
                    [
                        "androidforge",
                        "session",
                        "inspect",
                        "--session",
                        session_id.as_str(),
                        "--json",
                    ],
                ));
            }
        }
        DiagnosisCategory::Environment => {
            actions.push(action(
                "Re-run the actionable environment check.",
                "high",
                ["androidforge", "doctor", "--json"],
            ));
            actions.push(action(
                "Get a read-only repair plan with exact SDK/tool commands.",
                "high",
                ["androidforge", "doctor", "fix", "--plan", "--json"],
            ));
        }
        DiagnosisCategory::Install => {
            if let Some(apk) = apk_path {
                actions.push(action(
                    "Inspect the APK before reinstalling.",
                    "high",
                    ["androidforge", "apk", "inspect", apk, "--json"],
                ));
                let mut install = vec![
                    "androidforge".to_string(),
                    "app".to_string(),
                    "install".to_string(),
                    apk.to_string(),
                ];
                install.extend(session.clone());
                install.push("--json".to_string());
                actions.push(action(
                    "Install the recorded APK on the recorded session.",
                    "normal",
                    install,
                ));
                let message = failure
                    .map(|failure| failure.message.to_ascii_lowercase())
                    .unwrap_or_default();
                if message.contains("downgrade") {
                    let mut downgrade = vec![
                        "androidforge".to_string(),
                        "app".to_string(),
                        "install".to_string(),
                        apk.to_string(),
                        "--downgrade".to_string(),
                    ];
                    downgrade.extend(session.clone());
                    downgrade.push("--json".to_string());
                    actions.push(action(
                        "The device rejected this APK as a version downgrade; allow it explicitly only if intended.",
                        "normal",
                        downgrade,
                    ));
                }
            } else {
                actions.push(action(
                    "Inspect the stored report to identify the APK that failed to install.",
                    "high",
                    [
                        "androidforge",
                        "report",
                        "show",
                        report.run_id.as_str(),
                        "--json",
                    ],
                ));
            }
            if let Some(package) = &package {
                let mut status = argv(&["androidforge", "app", "status", package]);
                status.extend(session.clone());
                status.push("--json".to_string());
                actions.push(action(
                    "Check the installed package and process state.",
                    "normal",
                    status,
                ));
            }
        }
        DiagnosisCategory::Signature => {
            if let Some(apk) = apk_path {
                let mut verify = argv(&["androidforge", "apk", "verify", apk, "--json"]);
                if let Some(package) = &package {
                    verify.push("--expected-package".to_string());
                    verify.push(package.clone());
                }
                actions.push(action(
                    "Verify the APK signature and package identity locally.",
                    "high",
                    verify,
                ));
                let mut install = vec![
                    "androidforge".to_string(),
                    "app".to_string(),
                    "install".to_string(),
                    apk.to_string(),
                    "--fresh".to_string(),
                    "--yes".to_string(),
                ];
                install.extend(session.clone());
                install.push("--json".to_string());
                actions.push(NextAction {
                    reason: "Only if replacing the app and deleting its data is intended."
                        .to_string(),
                    priority: "normal".to_string(),
                    command: CommandLine::new(install),
                    destructive: true,
                    warning: Some(
                        "A fresh install deletes application data. Confirm that this is intended."
                            .to_string(),
                    ),
                });
            } else {
                actions.push(action(
                    "Inspect the stored report to identify the signed APK.",
                    "high",
                    [
                        "androidforge",
                        "report",
                        "show",
                        report.run_id.as_str(),
                        "--json",
                    ],
                ));
            }
        }
        DiagnosisCategory::Application | DiagnosisCategory::Anr | DiagnosisCategory::Process => {
            if let Some(package) = &package {
                let mut crashes = argv(&[
                    "androidforge",
                    "diagnostics",
                    "crashes",
                    package.as_str(),
                    "--json",
                ]);
                crashes.extend(session.clone());
                actions.push(action(
                    "Inspect crash and ANR evidence for the package.",
                    "high",
                    crashes,
                ));
                let mut logcat = argv(&[
                    "androidforge",
                    "diagnostics",
                    "logcat",
                    "peek",
                    "--package",
                    package.as_str(),
                    "--level",
                    "warn",
                    "--json",
                ]);
                logcat.extend(session.clone());
                actions.push(action(
                    "Read a bounded warning/error log window around the failure.",
                    "high",
                    logcat,
                ));
            }
            let mut observe = argv(&["androidforge", "ui", "observe"]);
            observe.extend(session.clone());
            observe.push("--json".to_string());
            actions.push(action(
                "Observe the current screen after the crash or ANR.",
                "normal",
                observe,
            ));
        }
        DiagnosisCategory::Task => {
            let mut observe = argv(&["androidforge", "ui", "observe"]);
            observe.extend(session.clone());
            observe.push("--json".to_string());
            actions.push(action(
                "Observe the current foreground task and activity.",
                "high",
                observe,
            ));
            if let Some(package) = &package {
                let mut status = argv(&["androidforge", "app", "status", package]);
                status.extend(session.clone());
                status.push("--json".to_string());
                actions.push(action(
                    "Inspect task, activity and foreground state.",
                    "normal",
                    status,
                ));
            }
        }
        DiagnosisCategory::Timeout => {
            if let Some(session) = &report.session_id {
                actions.push(action(
                    "Inspect the session and device state before retrying.",
                    "high",
                    [
                        "androidforge",
                        "session",
                        "status",
                        "--session",
                        session.as_str(),
                        "--json",
                    ],
                ));
            }
            actions.push(action(
                "Inspect connected devices and boot state.",
                "normal",
                ["androidforge", "devices", "list", "--json"],
            ));
        }
        DiagnosisCategory::Assertion => {
            let mut observe = argv(&["androidforge", "ui", "observe"]);
            observe.extend(session.clone());
            observe.push("--json".to_string());
            actions.push(action(
                "Observe the actual screen state instead of assuming the assertion target exists.",
                "high",
                observe,
            ));
            let mut screenshot = argv(&["androidforge", "capture", "screenshot"]);
            screenshot.extend(session.clone());
            screenshot.push("--json".to_string());
            actions.push(action(
                "Capture a screenshot for visual context.",
                "normal",
                screenshot,
            ));
            if let Some(operation) = operation {
                actions.push(action(
                    format!(
                        "Retry the failed step only after inspecting the evidence: {operation}"
                    ),
                    "normal",
                    [
                        "androidforge",
                        "report",
                        "show",
                        report.run_id.as_str(),
                        "--json",
                    ],
                ));
            }
        }
        DiagnosisCategory::Permission => {
            if let Some(package) = &package {
                let permission = failure
                    .and_then(|failure| failure.context.get("permission"))
                    .and_then(|value| value.as_str());
                if let Some(permission) = permission {
                    let mut check = argv(&[
                        "androidforge",
                        "permissions",
                        "check",
                        permission,
                        "--package",
                        package,
                        "--json",
                    ]);
                    check.extend(session.clone());
                    actions.push(action(
                        "Inspect this Android permission for the package.",
                        "high",
                        check,
                    ));
                } else {
                    let mut check = argv(&[
                        "androidforge",
                        "diagnostics",
                        "app",
                        package.as_str(),
                        "--json",
                    ]);
                    check.extend(session.clone());
                    actions.push(action(
                        "Inspect the application's permission and package state.",
                        "high",
                        check,
                    ));
                }
            }
            actions.push(action(
                "Inspect AppOps and permission state in full diagnostics.",
                "normal",
                ["androidforge", "diagnostics", "collect", "--json"],
            ));
        }
        DiagnosisCategory::Configuration => {
            actions.push(action(
                "Validate the project adapter and merged configuration.",
                "high",
                [
                    "androidforge",
                    "config",
                    "validate",
                    "--project",
                    project,
                    "--json",
                ],
            ));
            actions.push(action(
                "Inspect the actionable environment and project checks.",
                "normal",
                ["androidforge", "doctor", "--json"],
            ));
        }
        DiagnosisCategory::Cancelled
        | DiagnosisCategory::Interrupted
        | DiagnosisCategory::Unknown => {
            actions.push(action(
                "Inspect the stored report and its artifact paths.",
                "high",
                [
                    "androidforge",
                    "report",
                    "show",
                    report.run_id.as_str(),
                    "--json",
                ],
            ));
            actions.push(action(
                "Use the current context to decide whether the scenario can be retried safely.",
                "normal",
                ["androidforge", "context", "--json"],
            ));
        }
    }
    actions
}

fn evidence_priority(artifact: &ArtifactRef) -> Option<u8> {
    let kind = artifact.kind.as_str();
    match kind {
        "command_output" => Some(0),
        "ui_hierarchy" => Some(1),
        "crash_report" | "anr" | "crash_buffer" | "tombstone" => Some(2),
        "app_status" => Some(3),
        "screenshot" | "screen_recording" => Some(4),
        "logcat_filtered" => Some(5),
        "build_identity" | "apk_metadata" => Some(6),
        "logcat_raw" | "dumpsys" => None,
        _ => Some(7),
    }
}

fn relevant_evidence(report: &RunReport, failure: Option<&FailureInfo>) -> Vec<EvidenceRef> {
    let mut candidates: Vec<(&ArtifactRef, u8)> = Vec::new();
    for artifact in report.artifacts.iter().chain(
        failure
            .map(|failure| failure.artifacts.iter())
            .into_iter()
            .flatten(),
    ) {
        if let Some(priority) = evidence_priority(artifact) {
            candidates.push((artifact, priority));
        }
    }
    candidates.sort_by(|(left, left_priority), (right, right_priority)| {
        left_priority
            .cmp(right_priority)
            .then_with(|| left.path.cmp(&right.path))
    });
    let mut evidence = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (artifact, _) in candidates {
        if !seen.insert(artifact.path.clone()) {
            continue;
        }
        evidence.push(EvidenceRef {
            source: artifact.path.clone(),
            description: artifact
                .description
                .clone()
                .unwrap_or_else(|| format!("Stored {} artifact.", artifact.kind)),
            artifact_kind: Some(artifact.kind.clone()),
        });
        if evidence.len() >= 10 {
            break;
        }
    }
    evidence
}

/// Classify a report. The classifier is deterministic and relies only on stored
/// report fields. It never executes external commands.
pub fn explain_report(report: &RunReport) -> DiagnosticExplanation {
    let failure = failure_for(report);
    let package = package_from(report);
    let (category, confidence) = if let Some((failure, _)) = failure {
        category_for_failure(failure)
    } else {
        match report.status {
            RunStatus::Cancelled => (DiagnosisCategory::Cancelled, Confidence::High),
            RunStatus::Interrupted | RunStatus::TimedOut | RunStatus::Abandoned => {
                (DiagnosisCategory::Interrupted, Confidence::High)
            }
            RunStatus::Failed | RunStatus::Partial => {
                (DiagnosisCategory::Assertion, Confidence::Medium)
            }
            _ => (DiagnosisCategory::Unknown, Confidence::InsufficientEvidence),
        }
    };

    let mut alternatives = Vec::new();
    let mut uncertainty = Vec::new();
    if let Some((failure, _)) = failure {
        let message = failure.message.to_ascii_lowercase();
        if category == DiagnosisCategory::Unknown {
            uncertainty.push(
                "The report has a failure but no error code that maps to a known category."
                    .to_string(),
            );
        }
        if category != DiagnosisCategory::Device
            && (message.contains("device") || message.contains("emulator"))
            && (message.contains("offline")
                || message.contains("disconnect")
                || message.contains("not connected"))
        {
            alternatives.push(Diagnosis::from_parts(
                DiagnosisCategory::Device,
                Some(failure.code.as_str().to_string()),
                Confidence::Medium,
                "The message also indicates a device/emulator availability problem.",
                Some(failure.failed_operation.clone()),
            ));
        }
        if category != DiagnosisCategory::Timeout
            && (message.contains("timeout") || message.contains("timed out"))
        {
            alternatives.push(Diagnosis::from_parts(
                DiagnosisCategory::Timeout,
                Some(failure.code.as_str().to_string()),
                Confidence::Medium,
                "The message also reports a deadline being exceeded.",
                Some(failure.failed_operation.clone()),
            ));
        }
    } else if matches!(report.status, RunStatus::Failed | RunStatus::Partial) {
        uncertainty.push(
            "The run is failed but contains no structured failure object; inspect its steps and artifacts."
                .to_string(),
        );
    }

    let (code, operation) = failure
        .map(|(failure, _)| {
            (
                Some(failure.code.as_str().to_string()),
                Some(failure.failed_operation.clone()),
            )
        })
        .unwrap_or((None, None));
    let summary = match (failure, category) {
        (Some((failure, _)), _) => {
            let failed_checks = failure
                .context
                .get("failed_checks")
                .and_then(|value| value.as_str());
            match failed_checks {
                Some(checks) => format!(
                    "The run failed in '{}'. {} Failed checks: {}.",
                    failure.failed_operation, failure.message, checks
                ),
                None => format!(
                    "The run failed in '{}'. {}",
                    failure.failed_operation, failure.message
                ),
            }
        }
        (None, DiagnosisCategory::Cancelled) => {
            "The run was cancelled before it finalized; the session was preserved.".to_string()
        }
        (None, DiagnosisCategory::Interrupted) => {
            "The run did not finalize normally and was reconciled as interrupted or abandoned."
                .to_string()
        }
        _ => {
            "The stored report does not contain enough structured evidence for a precise diagnosis."
                .to_string()
        }
    };

    let evidence = relevant_evidence(report, failure.map(|(failure, _)| failure));

    DiagnosticExplanation {
        schema_version: DIAGNOSIS_SCHEMA_VERSION,
        generated_at: now_rfc3339(),
        run_id: report.run_id.clone(),
        project: report.project.clone(),
        project_root: report.project_root.clone(),
        scenario: report.command.clone(),
        status: report.status,
        diagnosis: Diagnosis::from_parts(category, code, confidence, summary, operation),
        alternatives,
        evidence,
        next_actions: next_actions(report, category, failure.map(|(failure, _)| failure)),
        fingerprint: fingerprint(
            category,
            failure.map(|(failure, _)| failure),
            report,
            package.as_deref(),
        ),
        fingerprint_version: FINGERPRINT_VERSION,
        session_id: report.session_id.clone(),
        device_serial: report.device_serial.clone(),
        package,
        artifacts: report.artifacts.clone(),
        uncertainty,
    }
}

impl Diagnosis {
    fn from_parts(
        category: DiagnosisCategory,
        code: Option<String>,
        confidence: Confidence,
        summary: impl Into<String>,
        failed_operation: Option<String>,
    ) -> Self {
        Self {
            category,
            code,
            confidence,
            summary: summary.into(),
            failed_operation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BuildIdentity;
    use crate::run::{RunReport, RunStatus, RunStep, StepStatus};

    fn failure(code: ErrorCode, message: &str, operation: &str) -> FailureInfo {
        FailureInfo {
            code,
            message: message.to_string(),
            failed_operation: operation.to_string(),
            recoverable: true,
            suggestions: Vec::new(),
            context: Default::default(),
            artifacts: Vec::new(),
        }
    }

    fn report_with(code: ErrorCode, message: &str) -> RunReport {
        let mut report = RunReport::new("run-1", "conformance.run", "App", "C:\\App");
        report.status = RunStatus::Failed;
        report.failure = Some(failure(code, message, "ui:tap"));
        report
    }

    #[test]
    fn selector_failure_is_high_confidence() {
        let explanation = explain_report(&report_with(
            ErrorCode::SelectorNotFound,
            "no UI element matched the requested selector",
        ));
        assert_eq!(explanation.diagnosis.category, DiagnosisCategory::Selector);
        assert_eq!(explanation.diagnosis.confidence, Confidence::High);
        assert!(
            explanation
                .next_actions
                .iter()
                .any(|action| { action.command.argv.iter().any(|arg| arg == "observe") })
        );
    }

    #[test]
    fn fingerprint_ignores_run_ids_and_paths() {
        let mut first = report_with(
            ErrorCode::InstallFailed,
            "install failed at C:\\a\\app.apk 12345",
        );
        first.run_id = "run-a".to_string();
        let mut second = report_with(
            ErrorCode::InstallFailed,
            "install failed at D:\\b\\app.apk 98765",
        );
        second.run_id = "run-b".to_string();
        assert_eq!(
            explain_report(&first).fingerprint,
            explain_report(&second).fingerprint
        );
    }

    #[test]
    fn different_categories_do_not_collide() {
        let selector = explain_report(&report_with(ErrorCode::SelectorNotFound, "missing"));
        let device = explain_report(&report_with(ErrorCode::DeviceNotFound, "missing"));
        assert_ne!(selector.fingerprint, device.fingerprint);
    }

    #[test]
    fn cancelled_run_explains_preservation() {
        let mut report = report_with(ErrorCode::Interrupted, "cancelled");
        report.status = RunStatus::Cancelled;
        let explanation = explain_report(&report);
        assert_eq!(
            explanation.diagnosis.category,
            DiagnosisCategory::Interrupted
        );
        assert!(
            explanation
                .next_actions
                .iter()
                .any(|action| { action.command.argv.iter().any(|arg| arg == "show") })
        );
    }

    #[test]
    fn device_and_environment_failures_get_actionable_commands() {
        let device = explain_report(&report_with(ErrorCode::DeviceNotFound, "device not found"));
        assert_eq!(device.diagnosis.category, DiagnosisCategory::Device);
        assert!(
            device
                .next_actions
                .iter()
                .any(|action| { action.command.argv.iter().any(|arg| arg == "devices") })
        );
        let environment = explain_report(&report_with(ErrorCode::ToolNotFound, "adb missing"));
        assert_eq!(
            environment.diagnosis.category,
            DiagnosisCategory::Environment
        );
        assert!(
            environment
                .next_actions
                .iter()
                .any(|action| { action.command.argv.iter().any(|arg| arg == "fix") })
        );
    }

    #[test]
    fn ambiguous_selector_uses_an_explicit_index_only_after_observation() {
        let mut report = report_with(ErrorCode::SelectorAmbiguous, "multiple elements matched");
        report.failure.as_mut().unwrap().context.insert(
            "selector".to_string(),
            serde_json::json!({"text": "Save", "index": 2}),
        );
        let explanation = explain_report(&report);
        assert_eq!(
            explanation.diagnosis.category,
            DiagnosisCategory::SelectorAmbiguous
        );
        assert!(explanation.next_actions.iter().any(|action| {
            action
                .command
                .argv
                .windows(2)
                .any(|pair| pair[0] == "--index" && pair[1] == "2")
        }));
        let first = &explanation.next_actions[0].command.argv;
        assert!(first.iter().any(|arg| arg == "observe"));
    }

    #[test]
    fn signature_diagnosis_uses_the_recorded_apk_path() {
        let mut report = report_with(ErrorCode::InstallSignatureMismatch, "signature mismatch");
        report.build = BuildIdentity {
            apk_path: Some("C:\\builds\\app-debug.apk".to_string()),
            ..BuildIdentity::default()
        };
        let explanation = explain_report(&report);
        assert_eq!(explanation.diagnosis.category, DiagnosisCategory::Signature);
        assert!(explanation.next_actions.iter().any(|action| {
            action
                .command
                .argv
                .iter()
                .any(|arg| arg == "C:\\builds\\app-debug.apk")
        }));
    }

    #[test]
    fn anr_messages_are_classified_without_an_llm() {
        let explanation = explain_report(&report_with(
            ErrorCode::CommandFailed,
            "ANR in com.example.app: Input dispatching timed out",
        ));
        assert_eq!(explanation.diagnosis.category, DiagnosisCategory::Anr);
        assert_eq!(explanation.diagnosis.confidence, Confidence::High);
    }

    #[test]
    fn materially_different_selectors_do_not_share_a_fingerprint() {
        let mut first = report_with(ErrorCode::SelectorNotFound, "not found");
        first
            .failure
            .as_mut()
            .unwrap()
            .context
            .insert("selector".to_string(), serde_json::json!({"text": "Save"}));
        let mut second = report_with(ErrorCode::SelectorNotFound, "not found");
        second.failure.as_mut().unwrap().context.insert(
            "selector".to_string(),
            serde_json::json!({"text": "Delete"}),
        );
        assert_ne!(
            explain_report(&first).fingerprint,
            explain_report(&second).fingerprint
        );
    }

    #[test]
    fn generated_commands_have_no_placeholder_arguments() {
        let mut reports = vec![
            report_with(ErrorCode::SelectorNotFound, "not found"),
            report_with(ErrorCode::SelectorAmbiguous, "ambiguous"),
            report_with(ErrorCode::DeviceNotFound, "device missing"),
            report_with(ErrorCode::ToolNotFound, "adb missing"),
            report_with(ErrorCode::InstallFailed, "install failed"),
            report_with(ErrorCode::InstallSignatureMismatch, "signature mismatch"),
            report_with(ErrorCode::UiAssertionFailed, "assertion failed"),
            report_with(ErrorCode::PermissionDenied, "permission denied"),
            report_with(ErrorCode::CommandTimeout, "timed out"),
            report_with(ErrorCode::ConfigInvalid, "invalid"),
            report_with(ErrorCode::CommandFailed, "ANR in com.example"),
            report_with(ErrorCode::CommandFailed, "app process died"),
        ];
        for report in &mut reports {
            report.build = BuildIdentity {
                apk_path: Some("C:\\builds\\app-debug.apk".to_string()),
                ..BuildIdentity::default()
            };
            if let Some(failure) = report.failure.as_mut() {
                failure
                    .context
                    .insert("package".to_string(), serde_json::json!("com.example.app"));
                failure.context.insert(
                    "selector".to_string(),
                    serde_json::json!({"text": "Save", "index": 1}),
                );
                failure.context.insert(
                    "permission".to_string(),
                    serde_json::json!("android.permission.CAMERA"),
                );
            }
            let explanation = explain_report(report);
            for action in &explanation.next_actions {
                assert!(!action.command.argv.is_empty());
                for arg in &action.command.argv {
                    assert!(
                        !arg.contains('<') && !arg.contains('>'),
                        "placeholder argument leaked into {:?}",
                        action.command.argv
                    );
                    assert!(!arg.trim().is_empty());
                }
            }
        }
    }

    #[test]
    fn aggregate_command_failure_defers_to_a_specific_step_failure() {
        let mut report = report_with(
            ErrorCode::CommandFailed,
            "external command 'python flow.py' exited with code 7",
        );
        let mut step = RunStep {
            id: "step-001".to_string(),
            name: "conformance:release-smoke".to_string(),
            command: "conformance release-smoke".to_string(),
            status: StepStatus::Failed,
            exit_code: Some(7),
            started_at: "2026-01-01T00:00:00Z".to_string(),
            ended_at: Some("2026-01-01T00:00:01Z".to_string()),
            duration_ms: 1000,
            failure: Some(failure(
                ErrorCode::UiAssertionFailed,
                "conformance flow 'release-smoke' failed",
                "conformance:release-smoke",
            )),
            artifacts: Vec::new(),
            details: serde_json::Value::Null,
            retries: 0,
        };
        step.failure.as_mut().unwrap().context.insert(
            "failed_checks".to_string(),
            serde_json::json!("status, artifact:recordings/*.mp4"),
        );
        report.steps.push(step);
        let explanation = explain_report(&report);
        assert_eq!(explanation.diagnosis.category, DiagnosisCategory::Assertion);
        assert_eq!(
            explanation.diagnosis.code.as_deref(),
            Some("E_UI_ASSERTION_FAILED")
        );
        assert!(explanation.diagnosis.summary.contains("recordings/*.mp4"));
    }

    #[test]
    fn a_specific_run_level_code_is_not_overridden_by_an_unrelated_step() {
        let mut report = report_with(ErrorCode::SelectorNotFound, "selector missing");
        report.steps.push(RunStep {
            id: "step-001".to_string(),
            name: "some other assertion".to_string(),
            command: "ui assert".to_string(),
            status: StepStatus::Failed,
            exit_code: Some(7),
            started_at: "2026-01-01T00:00:00Z".to_string(),
            ended_at: None,
            duration_ms: 1,
            failure: Some(failure(
                ErrorCode::UiAssertionFailed,
                "assertion failed",
                "ui.assert",
            )),
            artifacts: Vec::new(),
            details: serde_json::Value::Null,
            retries: 0,
        });
        let explanation = explain_report(&report);
        assert_eq!(explanation.diagnosis.category, DiagnosisCategory::Selector);
    }

    #[test]
    fn downgrade_failures_expose_the_explicit_downgrade_action() {
        let mut report = report_with(
            ErrorCode::InstallFailed,
            "INSTALL_FAILED_VERSION_DOWNGRADE: Downgrade detected",
        );
        report.build = BuildIdentity {
            apk_path: Some("C:\\builds\\app-debug.apk".to_string()),
            ..BuildIdentity::default()
        };
        let explanation = explain_report(&report);
        assert_eq!(explanation.diagnosis.category, DiagnosisCategory::Install);
        assert!(
            explanation
                .next_actions
                .iter()
                .any(|action| { action.command.argv.iter().any(|arg| arg == "--downgrade") })
        );
    }

    #[test]
    fn signature_message_is_classified_as_signature_even_with_install_failed_code() {
        let mut report = report_with(
            ErrorCode::InstallFailed,
            "INSTALL_FAILED_UPDATE_INCOMPATIBLE: signatures do not match",
        );
        report.build = BuildIdentity {
            apk_path: Some("C:\\builds\\app-debug.apk".to_string()),
            ..BuildIdentity::default()
        };
        let explanation = explain_report(&report);
        assert_eq!(explanation.diagnosis.category, DiagnosisCategory::Signature);
        assert!(
            explanation
                .next_actions
                .iter()
                .any(|action| { action.command.argv.iter().any(|arg| arg == "verify") })
        );
    }

    #[test]
    fn uiautomator_failures_are_automation_failures_not_unknown() {
        let explanation = explain_report(&report_with(
            ErrorCode::UiAutomatorUnavailable,
            "could not obtain a usable UIAutomator hierarchy",
        ));
        assert_eq!(
            explanation.diagnosis.category,
            DiagnosisCategory::UiAutomation
        );
        assert_eq!(explanation.diagnosis.confidence, Confidence::High);
        assert!(
            explanation
                .next_actions
                .iter()
                .any(|action| { action.command.argv.iter().any(|arg| arg == "observe") })
        );

        let legacy = explain_report(&report_with(
            ErrorCode::CommandFailed,
            "could not obtain a usable UIAutomator hierarchy",
        ));
        assert_eq!(legacy.diagnosis.category, DiagnosisCategory::UiAutomation);

        let killed = explain_report(&report_with(
            ErrorCode::CommandFailed,
            "ADB command failed with exit code 137",
        ));
        assert_eq!(killed.diagnosis.category, DiagnosisCategory::UiAutomation);
        assert_eq!(killed.diagnosis.confidence, Confidence::Medium);
    }
}
