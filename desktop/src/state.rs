use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::cli_locator::CliResolution;
use crate::backend::compatibility::CliInfo;
use crate::backend::evidence::{ManifestVerification, TextPreview};
use crate::backend::process_manager::{OperationHandle, OperationOutcome};
use crate::backend::project::DiscoveredProject;
use crate::models::{
    ApkInfo, DeviceList, DoctorResult, ErrorBody, FixtureInfo, HookInfo, ProjectConfig, ReportPath,
    RunList, RunReport, SessionInfo, SessionList,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Overview,
    Devices,
    Sessions,
    Runs,
}

impl Page {
    pub const ALL: [Page; 4] = [Page::Overview, Page::Devices, Page::Sessions, Page::Runs];

    pub fn label(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Devices => "Devices",
            Page::Sessions => "Sessions",
            Page::Runs => "Runs",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    System,
    Dark,
    Light,
}

impl ThemeMode {
    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::System => "System",
            ThemeMode::Dark => "Dark",
            ThemeMode::Light => "Light",
        }
    }

    pub fn preference(self) -> egui::ThemePreference {
        match self {
            ThemeMode::System => egui::ThemePreference::System,
            ThemeMode::Dark => egui::ThemePreference::Dark,
            ThemeMode::Light => egui::ThemePreference::Light,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopSettings {
    pub cli_path: Option<String>,
    pub workspace: Option<String>,
    pub project: Option<String>,
    pub owner: String,
    pub theme: ThemeMode,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            cli_path: None,
            workspace: crate::backend::cli_locator::workspace_root()
                .map(|path| path.display().to_string()),
            project: None,
            owner: std::env::var("ANDROIDFORGE_OWNER")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| std::env::var("USERNAME").ok())
                .unwrap_or_else(|| "Desktop User".to_string()),
            theme: ThemeMode::System,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CliState {
    pub info: Option<CliInfo>,
    pub resolution: Option<CliResolution>,
    pub error: Option<String>,
    pub configured_path: Option<PathBuf>,
}

pub struct DataState<T> {
    pub value: T,
    pub updated: Option<Instant>,
    pub error: Option<String>,
    pub loading: bool,
}

impl<T: Default> Default for DataState<T> {
    fn default() -> Self {
        Self {
            value: T::default(),
            updated: None,
            error: None,
            loading: false,
        }
    }
}

impl<T> DataState<T> {
    pub fn age(&self) -> Option<Duration> {
        self.updated.map(|updated| updated.elapsed())
    }

    pub fn is_stale(&self) -> bool {
        self.error.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefreshKind {
    Doctor,
    Devices,
    Sessions,
    Runs,
    Projects,
    Project,
}

#[derive(Debug, Clone, Default)]
pub struct RefreshState {
    pub in_flight: HashSet<RefreshKind>,
    pub last_doctor: Option<Instant>,
    pub last_devices: Option<Instant>,
    pub last_sessions: Option<Instant>,
    pub last_runs: Option<Instant>,
    pub last_projects: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct ProjectBundle {
    pub config: ProjectConfig,
    pub fixtures: Vec<FixtureInfo>,
    pub hooks: Vec<HookInfo>,
}

#[derive(Debug, Clone)]
pub struct RunDetail {
    pub report: RunReport,
    pub path: ReportPath,
    pub raw_report: Option<TextPreview>,
    pub raw_metadata: Option<TextPreview>,
    pub raw_manifest: Option<TextPreview>,
    pub manifest: ManifestVerification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EvidenceTab {
    #[default]
    Summary,
    Artifacts,
    Raw,
}

#[derive(Default)]
pub struct EvidenceState {
    pub tab: EvidenceTab,
    pub selected_artifact: Option<String>,
    pub filter: String,
    pub search: String,
    pub text: Option<TextPreview>,
    pub text_key: Option<String>,
    pub text_error: Option<String>,
    pub image_texture: Option<egui::TextureHandle>,
    pub image_size: Option<(u32, u32)>,
    pub image_path: Option<String>,
    pub image_error: Option<String>,
    pub zoom: f32,
    pub fit: bool,
}

pub struct RunDetailState {
    pub detail: RunDetail,
    pub evidence: EvidenceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    SessionStart,
    SessionAttach,
    SessionRelease,
    SessionStop,
    SessionRestart,
    Install,
    Fixture,
    Conformance,
    Hook,
}

impl OperationKind {
    pub fn refreshes_sessions(self) -> bool {
        matches!(
            self,
            Self::SessionStart
                | Self::SessionAttach
                | Self::SessionRelease
                | Self::SessionStop
                | Self::SessionRestart
        )
    }

    pub fn creates_run(self) -> bool {
        matches!(
            self,
            Self::Fixture | Self::Conformance | Self::Hook | Self::Install
        )
    }

    pub fn cancellable(self) -> bool {
        matches!(
            self,
            Self::Fixture | Self::Conformance | Self::Hook | Self::Install
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub stream: LogStream,
    pub text: String,
}

pub struct OperationState {
    pub handle: OperationHandle,
    pub kind: OperationKind,
    pub lines: Vec<LogLine>,
    pub outcome: Option<OperationOutcome>,
    pub cancel_requested: bool,
}

impl OperationState {
    pub fn is_running(&self) -> bool {
        self.outcome.is_none()
    }
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    ReleaseSession(SessionInfo),
    StopSession(SessionInfo),
    RestartSession(SessionInfo),
    Install { apk_path: PathBuf, fresh: bool },
    Fixture { name: String, yes: bool },
    Conformance { yes: bool },
    Hook { name: String },
    CancelOperation,
}

impl ConfirmAction {
    pub fn title(&self) -> &'static str {
        match self {
            Self::ReleaseSession(_) => "Release session",
            Self::StopSession(_) => "Stop session",
            Self::RestartSession(_) => "Restart session",
            Self::Install { .. } => "Install APK",
            Self::Fixture { .. } => "Run fixture",
            Self::Conformance { .. } => "Run conformance",
            Self::Hook { .. } => "Run hook",
            Self::CancelOperation => "Cancel operation",
        }
    }

    pub fn confirm_label(&self) -> &'static str {
        match self {
            Self::ReleaseSession(_) => "Release",
            Self::StopSession(_) => "Stop",
            Self::RestartSession(_) => "Restart",
            Self::Install { .. } => "Install",
            Self::Fixture { .. } | Self::Conformance { .. } | Self::Hook { .. } => "Run",
            Self::CancelOperation => "Cancel operation",
        }
    }

    pub fn destructive(&self) -> bool {
        matches!(
            self,
            Self::StopSession(_)
                | Self::RestartSession(_)
                | Self::Install { .. }
                | Self::Fixture { .. }
                | Self::Conformance { .. }
                | Self::Hook { .. }
                | Self::CancelOperation
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct SessionForm {
    pub avd: String,
    pub port: String,
    pub owner: String,
    pub headless: bool,
    pub prepare: bool,
    pub reuse: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AttachForm {
    pub serial: String,
    pub owner: String,
    pub prepare: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RunForm {
    pub fixture: Option<String>,
    pub conformance: bool,
    pub yes: bool,
}

#[derive(Debug, Clone, Default)]
pub struct InstallState {
    pub open: bool,
    pub apk_path: Option<PathBuf>,
    pub info: Option<ApkInfo>,
    pub loading: bool,
    pub error: Option<String>,
    pub fresh: bool,
    pub preview: Option<Value>,
    pub preview_loading: bool,
    pub preview_error: Option<String>,
    pub result: Option<crate::models::BuildIdentity>,
    pub mismatch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub level: ToastLevel,
    pub created: Instant,
}

impl Toast {
    pub fn new(text: impl Into<String>, level: ToastLevel) -> Self {
        Self {
            text: text.into(),
            level,
            created: Instant::now(),
        }
    }

    pub fn expired(&self) -> bool {
        self.created.elapsed() > Duration::from_secs(8)
    }
}

#[derive(Debug, Clone, Default)]
pub struct UiError {
    pub code: String,
    pub message: String,
    pub cause: Option<String>,
    pub suggestions: Vec<String>,
    pub context: BTreeMap<String, Value>,
    pub raw_json: Option<String>,
    pub artifacts: Vec<crate::models::ArtifactRef>,
    pub device_serial: Option<String>,
    pub session_id: Option<String>,
}

impl UiError {
    pub fn from_error_body(body: &ErrorBody, raw: impl Into<String>) -> Self {
        Self {
            code: body.code.clone(),
            message: body.message.clone(),
            cause: body.cause.clone(),
            suggestions: body.suggestions.clone(),
            context: body.context.clone(),
            raw_json: Some(raw.into()),
            artifacts: body.artifacts.clone(),
            device_serial: body.device_serial.clone(),
            session_id: body.session_id.clone(),
        }
    }

    pub fn from_message(message: impl Into<String>) -> Self {
        Self {
            code: "E_DESKTOP".to_string(),
            message: message.into(),
            ..Default::default()
        }
    }

    pub fn from_outcome(outcome: &OperationOutcome) -> Option<Self> {
        if outcome.cancelled {
            return Some(Self {
                code: "E_OPERATION_CANCELLED".to_string(),
                message:
                    "The desktop operation was cancelled. The Android session was left intact."
                        .to_string(),
                raw_json: Some(outcome.stdout.clone()),
                ..Default::default()
            });
        }
        if let Some(envelope) = &outcome.envelope {
            if let Some(body) = &envelope.error {
                return Some(Self::from_error_body(body, outcome.stdout.clone()));
            }
        }
        if !outcome.success {
            return Some(Self {
                code: "E_DESKTOP_OPERATION".to_string(),
                message: outcome.parse_error.clone().unwrap_or_else(|| {
                    "The CLI operation did not complete successfully".to_string()
                }),
                raw_json: Some(if outcome.stderr.is_empty() {
                    outcome.stdout.clone()
                } else {
                    outcome.stderr.clone()
                }),
                ..Default::default()
            });
        }
        None
    }
}

#[derive(Debug)]
pub enum AppEvent {
    Doctor(std::result::Result<DoctorResult, String>),
    Devices(std::result::Result<DeviceList, String>),
    Sessions(std::result::Result<SessionList, String>),
    Runs(std::result::Result<RunList, String>),
    Projects(std::result::Result<Vec<DiscoveredProject>, String>),
    Project {
        path: PathBuf,
        result: std::result::Result<ProjectBundle, String>,
    },
    RunDetail {
        run_id: String,
        result: std::result::Result<RunDetail, String>,
    },
    EvidenceText {
        key: String,
        result: std::result::Result<TextPreview, String>,
    },
    ApkInfo {
        path: PathBuf,
        result: std::result::Result<ApkInfo, String>,
    },
    InstallPreview {
        path: PathBuf,
        result: std::result::Result<Value, String>,
    },
}
