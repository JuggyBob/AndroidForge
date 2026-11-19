use std::collections::{BTreeMap, HashSet, VecDeque};
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
use crate::backend::update::UpdateCheck;
use crate::models::{
    ApkInfo, DeviceList, DiagnosisExplanation, DoctorResult, ErrorBody, FixtureInfo, FlakyReport,
    FrameResult, HookInfo, LogcatPeek, MaintenanceSummary, ProjectConfig, ReconcileResult,
    RepairPlan, ReportPath, RunComparison, RunList, RunReport, SessionInfo, SessionList,
    UiInspectResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Setup,
    Overview,
    Context,
    Matrix,
    Devices,
    Sessions,
    Ownership,
    Runs,
    Triage,
    Compare,
    Logcat,
    Inspector,
    Maintenance,
    Settings,
}

impl Page {
    pub const ALL: [Page; 14] = [
        Page::Setup,
        Page::Overview,
        Page::Context,
        Page::Matrix,
        Page::Devices,
        Page::Sessions,
        Page::Ownership,
        Page::Runs,
        Page::Triage,
        Page::Compare,
        Page::Logcat,
        Page::Inspector,
        Page::Maintenance,
        Page::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::Setup => "Setup",
            Page::Overview => "Overview",
            Page::Context => "Context",
            Page::Matrix => "Matrix",
            Page::Devices => "Devices",
            Page::Sessions => "Sessions",
            Page::Ownership => "Ownership",
            Page::Runs => "Runs",
            Page::Triage => "Triage",
            Page::Compare => "Compare",
            Page::Logcat => "Logcat",
            Page::Inspector => "Inspector",
            Page::Maintenance => "Maintenance",
            Page::Settings => "Settings",
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

fn default_logcat_lines() -> usize {
    1500
}

fn default_preview_interval_ms() -> u64 {
    1200
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopSettings {
    pub cli_path: Option<String>,
    pub workspace: Option<String>,
    pub project: Option<String>,
    pub owner: String,
    pub theme: ThemeMode,
    pub accent: [u8; 3],
    pub font: Option<String>,
    pub onboarding_completed: bool,
    pub logcat_lines: usize,
    pub preview_interval_ms: u64,
    pub update_check_on_start: bool,
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
            accent: [240, 166, 50],
            font: None,
            onboarding_completed: false,
            logcat_lines: default_logcat_lines(),
            preview_interval_ms: default_preview_interval_ms(),
            update_check_on_start: false,
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
    Maintenance,
}

#[derive(Debug, Clone, Default)]
pub struct RefreshState {
    pub in_flight: HashSet<RefreshKind>,
    pub last_doctor: Option<Instant>,
    pub last_devices: Option<Instant>,
    pub last_sessions: Option<Instant>,
    pub last_runs: Option<Instant>,
    pub last_projects: Option<Instant>,
    pub last_maintenance: Option<Instant>,
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
    Maintenance,
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

    pub fn is_setup_lifecycle(self) -> bool {
        matches!(
            self,
            Self::SessionStart
                | Self::SessionAttach
                | Self::SessionRelease
                | Self::SessionStop
                | Self::SessionRestart
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
    Install {
        apk_path: PathBuf,
        fresh: bool,
    },
    Fixture {
        name: String,
        yes: bool,
    },
    Conformance {
        yes: bool,
    },
    Hook {
        name: String,
    },
    Cleanup(CleanupRequest),
    InstallCli {
        staged: PathBuf,
        target: PathBuf,
        version: String,
    },
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
            Self::Cleanup(_) => "Clean maintenance data",
            Self::InstallCli { .. } => "Replace CLI executable",
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
            Self::Cleanup(_) => "Clean now",
            Self::InstallCli { .. } => "Replace CLI",
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
                | Self::Cleanup(_)
                | Self::InstallCli { .. }
                | Self::CancelOperation
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SetupSection {
    #[default]
    Welcome,
    Cli,
    Environment,
    Project,
    Device,
    Updates,
    Finish,
    Agent,
}

impl SetupSection {
    pub const ALL: [SetupSection; 8] = [
        SetupSection::Welcome,
        SetupSection::Cli,
        SetupSection::Environment,
        SetupSection::Project,
        SetupSection::Device,
        SetupSection::Updates,
        SetupSection::Finish,
        SetupSection::Agent,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Welcome => "Welcome",
            Self::Cli => "CLI",
            Self::Environment => "Environment",
            Self::Project => "Project",
            Self::Device => "Device",
            Self::Updates => "Updates",
            Self::Finish => "Finish",
            Self::Agent => "Agent Handoff",
        }
    }

    pub fn position(self) -> usize {
        Self::ALL.iter().position(|item| *item == self).unwrap_or(0)
    }
}

#[derive(Default)]
pub struct SetupState {
    pub section: SetupSection,
    pub section_animation_started: Option<Instant>,
    pub highlight_y: Option<f32>,
    pub step_rects: Vec<(usize, egui::Rect)>,
    pub create_name: String,
    pub create_package: String,
    pub create_avd: String,
    pub create_busy: bool,
    pub create_error: Option<String>,
    pub test_running: bool,
    pub test_ok: Option<bool>,
    pub test_session: Option<String>,
    pub test_log: Vec<String>,
    pub test_error: Option<String>,
    pub checked_cli: bool,
    pub created_session: bool,
    pub repair_plan: Option<RepairPlan>,
    pub repair_loading: bool,
    pub repair_error: Option<String>,
}

#[derive(Default)]
pub struct UpdateState {
    pub value: Option<UpdateCheck>,
    pub loading: bool,
    pub error: Option<String>,
    pub staged: Option<PathBuf>,
    pub repair_busy: bool,
    pub repair_message: Option<String>,
    pub last_check: Option<Instant>,
}

#[derive(Debug, Clone, Default)]
pub struct CleanupRequest {
    pub older_days: u32,
    pub keep_last: usize,
    pub sessions: bool,
    pub runs: bool,
}

#[derive(Default)]
pub struct MaintenanceState {
    pub summary: DataState<MaintenanceSummary>,
    pub older_days: u32,
    pub keep_last: usize,
    pub include_sessions: bool,
    pub include_runs: bool,
    pub preview: Option<Value>,
    pub preview_loading: bool,
    pub preview_error: Option<String>,
    pub cleaning: bool,
    pub last_result: Option<String>,
    pub cleanup_request: Option<CleanupRequest>,
}

#[derive(Default)]
pub struct TriageState {
    pub run_id: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub flaky: Option<FlakyReport>,
    pub flaky_loading: bool,
    pub flaky_error: Option<String>,
    pub explanation: Option<DiagnosisExplanation>,
    pub explanation_loading: bool,
    pub explanation_error: Option<String>,
    pub history: Option<serde_json::Value>,
    pub history_loading: bool,
    pub history_error: Option<String>,
    pub bundle: BundleState,
}

#[derive(Default)]
pub struct BundleState {
    pub preview: Option<serde_json::Value>,
    pub preview_loading: bool,
    pub preview_error: Option<String>,
    pub result: Option<serde_json::Value>,
    pub creating: bool,
    pub create_error: Option<String>,
}

#[derive(Default)]
pub struct ContextState {
    pub snapshot: Option<serde_json::Value>,
    pub loading: bool,
    pub error: Option<String>,
    pub updated: Option<Instant>,
}

#[derive(Default)]
pub struct MatrixState {
    pub avds: String,
    pub flow: String,
    pub owner: String,
    pub install: bool,
    pub fail_fast: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub result: Option<serde_json::Value>,
}

#[derive(Default)]
pub struct CompareState {
    pub current: Option<String>,
    pub baseline: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub comparison: Option<RunComparison>,
}

#[derive(Default)]
pub struct CompareForm {
    pub current: Option<String>,
    pub baseline: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LogcatEntry {
    pub timestamp: String,
    pub level: String,
    pub tag: String,
    pub message: String,
    pub raw: String,
    pub crash: bool,
}

pub struct LogcatState {
    pub session_id: Option<String>,
    pub package: String,
    pub level: String,
    pub search: String,
    pub follow: bool,
    pub paused: bool,
    pub running: bool,
    pub loading: bool,
    pub lines: VecDeque<LogcatEntry>,
    pub error: Option<String>,
    pub notes: Vec<String>,
    pub retry: Option<serde_json::Value>,
    pub last_capture: Option<Instant>,
    pub exported: Option<String>,
    pub max_lines: usize,
    pub scroll_offset: f32,
}

impl Default for LogcatState {
    fn default() -> Self {
        Self {
            session_id: None,
            package: String::new(),
            level: "verbose".to_string(),
            search: String::new(),
            follow: true,
            paused: false,
            running: false,
            loading: false,
            lines: VecDeque::new(),
            error: None,
            notes: Vec::new(),
            retry: None,
            last_capture: None,
            exported: None,
            max_lines: 4000,
            scroll_offset: 0.0,
        }
    }
}

pub struct InspectorState {
    pub session_id: Option<String>,
    pub image: Option<egui::TextureHandle>,
    pub image_size: Option<(u32, u32)>,
    pub display_size: Option<(u32, u32)>,
    pub hierarchy: Option<crate::models::UiHierarchy>,
    pub selected_node: Option<usize>,
    pub collapsed: HashSet<usize>,
    pub search: String,
    pub raw_xml: bool,
    pub auto_refresh: bool,
    pub interval_ms: u64,
    pub last_frame: Option<Instant>,
    pub last_hierarchy: Option<Instant>,
    pub frame_loading: bool,
    pub hierarchy_loading: bool,
    pub frame_error: Option<String>,
    pub hierarchy_error: Option<String>,
    pub action_busy: bool,
    pub last_frame_path: Option<String>,
    pub last_action: Option<String>,
    pub typed_text: String,
    pub highlight_pulse: f32,
    pub performance: Option<crate::models::PerformanceReport>,
    pub performance_loading: bool,
    pub performance_error: Option<String>,
    pub performance_cold: bool,
}

impl Default for InspectorState {
    fn default() -> Self {
        Self {
            session_id: None,
            image: None,
            image_size: None,
            display_size: None,
            hierarchy: None,
            selected_node: None,
            collapsed: HashSet::new(),
            search: String::new(),
            raw_xml: false,
            auto_refresh: true,
            interval_ms: 1200,
            last_frame: None,
            last_hierarchy: None,
            frame_loading: false,
            hierarchy_loading: false,
            frame_error: None,
            hierarchy_error: None,
            action_busy: false,
            last_frame_path: None,
            last_action: None,
            typed_text: String::new(),
            highlight_pulse: 0.0,
            performance: None,
            performance_loading: false,
            performance_error: None,
            performance_cold: false,
        }
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
    pub dismissed: Option<Instant>,
}

impl Toast {
    pub fn new(text: impl Into<String>, level: ToastLevel) -> Self {
        Self {
            text: text.into(),
            level,
            created: Instant::now(),
            dismissed: None,
        }
    }

    pub fn dismiss(&mut self) {
        if self.dismissed.is_none() {
            self.dismissed = Some(Instant::now());
        }
    }

    pub fn expired(&self) -> bool {
        self.dismissed
            .map(|dismissed| dismissed.elapsed() > Duration::from_millis(260))
            .unwrap_or_else(|| self.created.elapsed() > Duration::from_millis(8260))
    }

    pub fn closing(&self) -> bool {
        self.dismissed.is_some() || self.created.elapsed() > Duration::from_secs(8)
    }

    pub fn slide_offset(&self) -> f32 {
        let enter = (self.created.elapsed().as_secs_f32() / 0.24).clamp(0.0, 1.0);
        let enter_offset = (1.0 - ease_out_cubic(enter)) * 34.0;
        let leave = if let Some(dismissed) = self.dismissed {
            (dismissed.elapsed().as_secs_f32() / 0.24).clamp(0.0, 1.0)
        } else if self.created.elapsed() > Duration::from_secs(8) {
            ((self.created.elapsed().as_secs_f32() - 8.0) / 0.24).clamp(0.0, 1.0)
        } else {
            0.0
        };
        enter_offset + ease_in_cubic(leave) * 34.0
    }

    pub fn opacity(&self) -> f32 {
        let enter = (self.created.elapsed().as_secs_f32() / 0.24).clamp(0.0, 1.0);
        let leave = if let Some(dismissed) = self.dismissed {
            (dismissed.elapsed().as_secs_f32() / 0.24).clamp(0.0, 1.0)
        } else if self.created.elapsed() > Duration::from_secs(8) {
            ((self.created.elapsed().as_secs_f32() - 8.0) / 0.24).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (enter * (1.0 - leave)).clamp(0.0, 1.0)
    }
}

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn ease_in_cubic(t: f32) -> f32 {
    t.powi(3)
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
        if let Some(envelope) = &outcome.envelope
            && let Some(body) = &envelope.error
        {
            return Some(Self::from_error_body(body, outcome.stdout.clone()));
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
    Update(std::result::Result<UpdateCheck, String>),
    Reconcile(std::result::Result<ReconcileResult, String>),
    Comparison(std::result::Result<RunComparison, String>),
    Flaky(std::result::Result<FlakyReport, String>),
    Context(std::result::Result<serde_json::Value, String>),
    Health(std::result::Result<serde_json::Value, String>),
    Wireless(std::result::Result<serde_json::Value, String>),
    Matrix(std::result::Result<serde_json::Value, String>),
    RecurrenceHistory(std::result::Result<serde_json::Value, String>),
    BundlePreview {
        run_id: String,
        result: std::result::Result<serde_json::Value, String>,
    },
    BundleCreated {
        run_id: String,
        result: std::result::Result<serde_json::Value, String>,
    },
    Maintenance(std::result::Result<MaintenanceSummary, String>),
    MaintenancePreview {
        request: CleanupRequest,
        result: std::result::Result<Value, String>,
    },
    Logcat(std::result::Result<LogcatPeek, String>),
    Frame(std::result::Result<FrameResult, String>),
    Hierarchy(std::result::Result<UiInspectResult, String>),
    Performance(std::result::Result<crate::models::PerformanceReport, String>),
    CliUpdateStaged(std::result::Result<PathBuf, String>),
    CliUpdateInstalled(std::result::Result<String, String>),
    SetupTest(std::result::Result<String, String>),
    SetupProject(std::result::Result<String, String>),
    UiAction {
        label: String,
        result: std::result::Result<String, String>,
    },
    Explanation(std::result::Result<DiagnosisExplanation, String>),
    RepairPlan(std::result::Result<RepairPlan, String>),
}
