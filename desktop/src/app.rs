use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::backend::DesktopError;
use crate::backend::cli_locator::{self};
use crate::backend::cli_runner::{CliRunner, Invocation};
use crate::backend::compatibility::{self};
use crate::backend::evidence;
use crate::backend::json_parser::{REPORT_SCHEMA_VERSION, parse_result};
use crate::backend::process_manager::{self, OperationEvent, OperationOutcome};
use crate::backend::project::{self, DiscoveredProject};
use crate::components;
use crate::models::{
    ApkInfo, Device, DeviceList, DoctorResult, FixtureList, HookList, InstallResult, ProjectConfig,
    ReportPath, RunList, RunReport, SessionInfo, SessionList,
};
use crate::state::*;
use crate::theme;
use crate::views;

const SETTINGS_KEY: &str = "androidforge.desktop.settings";

pub struct App {
    pub settings: DesktopSettings,
    pub applied_theme: ThemeMode,
    pub cli: CliState,
    pub runner: Option<CliRunner>,
    pub icon: egui::TextureHandle,
    pub page: Page,
    pub doctor: DataState<DoctorResult>,
    pub devices: DataState<DeviceList>,
    pub sessions: DataState<SessionList>,
    pub runs: DataState<RunList>,
    pub projects: Vec<DiscoveredProject>,
    pub projects_error: Option<String>,
    pub project_path: Option<PathBuf>,
    pub project: Option<ProjectBundle>,
    pub project_error: Option<String>,
    pub selected_device: Option<String>,
    pub selected_session: Option<String>,
    pub selected_run: Option<String>,
    pub run_detail: Option<RunDetailState>,
    pub run_loading: bool,
    pub operation: Option<OperationState>,
    pub refresh: RefreshState,
    pub toasts: Vec<Toast>,
    pub error_dialog: Option<UiError>,
    pub confirm: Option<ConfirmAction>,
    pub install: InstallState,
    pub session_form: Option<SessionForm>,
    pub attach_form: Option<AttachForm>,
    pub run_form: Option<RunForm>,
    pub settings_open: bool,
    pub event_tx: mpsc::Sender<AppEvent>,
    pub event_rx: mpsc::Receiver<AppEvent>,
    pub next_operation_id: u64,
    pub started: Instant,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = cc
            .storage
            .and_then(|storage| storage.get_string(SETTINGS_KEY))
            .and_then(|text| serde_json::from_str::<DesktopSettings>(&text).ok())
            .unwrap_or_default();

        theme::configure_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, settings.theme);
        let icon = theme::load_icon_texture(&cc.egui_ctx);
        let (event_tx, event_rx) = mpsc::channel();
        let project_path = settings.project.as_ref().map(PathBuf::from);
        let mut app = Self {
            settings,
            applied_theme: ThemeMode::System,
            cli: CliState::default(),
            runner: None,
            icon,
            page: Page::Overview,
            doctor: DataState::default(),
            devices: DataState::default(),
            sessions: DataState::default(),
            runs: DataState::default(),
            projects: Vec::new(),
            projects_error: None,
            project_path,
            project: None,
            project_error: None,
            selected_device: None,
            selected_session: None,
            selected_run: None,
            run_detail: None,
            run_loading: false,
            operation: None,
            refresh: RefreshState::default(),
            toasts: Vec::new(),
            error_dialog: None,
            confirm: None,
            install: InstallState::default(),
            session_form: None,
            attach_form: None,
            run_form: None,
            settings_open: false,
            event_tx,
            event_rx,
            next_operation_id: 1,
            started: Instant::now(),
        };
        app.applied_theme = app.settings.theme;
        app.connect(&cc.egui_ctx);
        app
    }

    fn connect(&mut self, ctx: &egui::Context) {
        self.cli = CliState {
            configured_path: self.settings.cli_path.as_ref().map(PathBuf::from),
            ..Default::default()
        };
        self.runner = None;
        let explicit = self.cli.configured_path.clone();
        let resolution = match cli_locator::resolve(explicit.as_deref()) {
            Ok(resolution) => resolution,
            Err(error) => {
                self.cli.error = Some(desktop_error_message(error));
                return;
            }
        };
        let runner = CliRunner::new(resolution.path.clone());
        match compatibility::probe(&runner) {
            Ok(info) => {
                self.cli.info = Some(info);
                self.cli.resolution = Some(resolution);
                self.runner = Some(runner);
                self.cli.error = None;
                self.refresh_projects(ctx);
                self.refresh_doctor(ctx);
                self.refresh_devices(ctx);
                self.refresh_sessions(ctx);
                self.refresh_runs(ctx);
                if self.project_path.is_some() {
                    self.refresh_project(ctx);
                }
            }
            Err(error) => {
                self.cli.error = Some(desktop_error_message(error));
                self.cli.resolution = Some(resolution);
            }
        }
    }

    pub fn runner(&self) -> Option<&CliRunner> {
        self.runner.as_ref()
    }

    pub fn workspace(&self) -> PathBuf {
        self.settings
            .workspace
            .as_ref()
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .or_else(cli_locator::workspace_root)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn selected_project(&self) -> Option<&DiscoveredProject> {
        let path = self.project_path.as_ref()?;
        self.projects.iter().find(|project| &project.root == path)
    }

    pub fn selected_session_info(&self) -> Option<SessionInfo> {
        let id = self.selected_session.as_ref()?;
        self.sessions
            .value
            .sessions
            .iter()
            .find(|session| &session.id == id)
            .cloned()
    }

    pub fn selected_device_info(&self) -> Option<Device> {
        let serial = self.selected_device.as_ref()?;
        self.devices
            .value
            .devices
            .iter()
            .find(|device| &device.serial == serial)
            .cloned()
    }

    pub fn active_operation(&self) -> bool {
        self.operation
            .as_ref()
            .map(OperationState::is_running)
            .unwrap_or(false)
    }

    pub fn operation_blocks_actions(&self) -> bool {
        self.active_operation()
    }

    pub fn set_page(&mut self, page: Page) {
        if self.page != page {
            self.page = page;
            self.run_detail = None;
        }
    }

    pub fn toast(&mut self, text: impl Into<String>, level: ToastLevel) {
        self.toasts.push(Toast::new(text, level));
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_event(event, ctx);
        }
    }

    fn handle_event(&mut self, event: AppEvent, ctx: &egui::Context) {
        match event {
            AppEvent::Doctor(result) => {
                self.refresh.in_flight.remove(&RefreshKind::Doctor);
                self.doctor.loading = false;
                match result {
                    Ok(value) => {
                        self.doctor.value = value;
                        self.doctor.updated = Some(Instant::now());
                        self.doctor.error = None;
                    }
                    Err(error) => self.doctor.error = Some(error),
                }
            }
            AppEvent::Devices(result) => {
                self.refresh.in_flight.remove(&RefreshKind::Devices);
                self.devices.loading = false;
                match result {
                    Ok(value) => {
                        self.devices.value = value;
                        self.devices.updated = Some(Instant::now());
                        self.devices.error = None;
                    }
                    Err(error) => self.devices.error = Some(error),
                }
            }
            AppEvent::Sessions(result) => {
                self.refresh.in_flight.remove(&RefreshKind::Sessions);
                self.sessions.loading = false;
                match result {
                    Ok(value) => {
                        if let Some(selected) = &self.selected_session {
                            if !value.sessions.iter().any(|session| &session.id == selected) {
                                self.selected_session = None;
                            }
                        }
                        self.sessions.value = value;
                        self.sessions.updated = Some(Instant::now());
                        self.sessions.error = None;
                    }
                    Err(error) => self.sessions.error = Some(error),
                }
            }
            AppEvent::Runs(result) => {
                self.refresh.in_flight.remove(&RefreshKind::Runs);
                self.runs.loading = false;
                match result {
                    Ok(value) => {
                        self.runs.value = value;
                        self.runs.updated = Some(Instant::now());
                        self.runs.error = None;
                    }
                    Err(error) => self.runs.error = Some(error),
                }
            }
            AppEvent::Projects(result) => {
                self.refresh.in_flight.remove(&RefreshKind::Projects);
                match result {
                    Ok(projects) => {
                        self.projects = projects;
                        self.projects_error = None;
                    }
                    Err(error) => self.projects_error = Some(error),
                }
            }
            AppEvent::Project { path, result } => {
                self.refresh.in_flight.remove(&RefreshKind::Project);
                if self.project_path.as_deref() != Some(path.as_path()) {
                    return;
                }
                match result {
                    Ok(bundle) => {
                        self.project = Some(bundle);
                        self.project_error = None;
                    }
                    Err(error) => self.project_error = Some(error),
                }
            }
            AppEvent::RunDetail { run_id, result } => {
                self.run_loading = false;
                if self.selected_run.as_deref() != Some(run_id.as_str()) {
                    return;
                }
                match result {
                    Ok(detail) => {
                        self.run_detail = Some(RunDetailState {
                            detail,
                            evidence: EvidenceState {
                                zoom: 1.0,
                                fit: true,
                                ..Default::default()
                            },
                        });
                    }
                    Err(error) => {
                        self.error_dialog = Some(UiError::from_message(error));
                        self.run_detail = None;
                    }
                }
            }
            AppEvent::EvidenceText { key, result } => {
                if self.evidence_text_key() != Some(key.clone()) {
                    return;
                }
                if let Some(state) = self.run_detail.as_mut() {
                    match result {
                        Ok(preview) => {
                            state.evidence.text = Some(preview);
                            state.evidence.text_error = None;
                        }
                        Err(error) => {
                            state.evidence.text_error = Some(error);
                            state.evidence.text = None;
                        }
                    }
                }
            }
            AppEvent::ApkInfo { path, result } => {
                if self.install.apk_path.as_deref() != Some(path.as_path()) {
                    return;
                }
                self.install.loading = false;
                match result {
                    Ok(info) => {
                        self.install.mismatch = self.certificate_mismatch(&info);
                        self.install.info = Some(info);
                        self.install.error = None;
                    }
                    Err(error) => {
                        self.install.error = Some(error);
                        self.install.info = None;
                    }
                }
            }
            AppEvent::InstallPreview { path, result } => {
                if self.install.apk_path.as_deref() != Some(path.as_path()) {
                    return;
                }
                self.install.preview_loading = false;
                match result {
                    Ok(value) => {
                        self.install.preview = Some(value);
                        self.install.preview_error = None;
                    }
                    Err(error) => self.install.preview_error = Some(error),
                }
            }
        }
        ctx.request_repaint();
    }

    fn evidence_text_key(&self) -> Option<String> {
        self.run_detail
            .as_ref()
            .and_then(|state| state.evidence.text_key.clone())
    }

    fn certificate_mismatch(&self, info: &ApkInfo) -> Option<String> {
        let expected = self
            .project
            .as_ref()
            .map(|bundle| bundle.config.project.expected_cert_sha256.clone())
            .unwrap_or_default();
        if expected.is_empty() {
            return None;
        }
        let actual = info.signature.cert_sha256.as_deref()?;
        let actual = actual.replace(':', "").to_ascii_uppercase();
        let matches = expected
            .iter()
            .any(|value| value.replace(':', "").eq_ignore_ascii_case(&actual));
        if matches {
            None
        } else {
            Some(format!(
                "The APK signer {} is not in the project's expected certificate list.",
                info.signature.cert_sha256.as_deref().unwrap_or("-")
            ))
        }
    }
}

fn desktop_error_message(error: DesktopError) -> String {
    match error {
        DesktopError::CliNotFound { searched } => format!(
            "AndroidForge CLI was not found. Searched:\n{}",
            searched
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        ),
        other => other.to_string(),
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(text) = serde_json::to_string(&self.settings) {
            storage.set_string(SETTINGS_KEY, text);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.applied_theme != self.settings.theme {
            self.applied_theme = self.settings.theme;
            theme::apply(&ctx, self.settings.theme);
        }
        self.drain_events(&ctx);
        self.poll_operation(&ctx);
        self.poll_refresh(&ctx);

        egui::Panel::top("androidforge-topbar")
            .exact_size(44.0)
            .frame(
                egui::Frame::new()
                    .fill(if ui.style().visuals.dark_mode {
                        theme::DARK_PANEL
                    } else {
                        theme::LIGHT_PANEL
                    })
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| components::topbar(self, ui));
        egui::Panel::left("androidforge-sidebar")
            .exact_size(188.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(if ui.style().visuals.dark_mode {
                        theme::DARK_PANEL
                    } else {
                        theme::LIGHT_PANEL
                    })
                    .inner_margin(egui::Margin::symmetric(8, 8)),
            )
            .show(ui, |ui| components::sidebar(self, ui));

        if self.operation.is_some() {
            egui::Panel::bottom("androidforge-operation")
                .resizable(true)
                .default_size(180.0)
                .min_size(120.0)
                .max_size(420.0)
                .frame(
                    egui::Frame::new()
                        .fill(if ui.style().visuals.dark_mode {
                            theme::DARK_PANEL
                        } else {
                            theme::LIGHT_PANEL
                        })
                        .inner_margin(egui::Margin::symmetric(10, 6)),
                )
                .show(ui, |ui| views::operations::show(self, ui));
        }

        egui::CentralPanel::default().show(ui, |ui| match self.page {
            Page::Overview => views::overview::show(self, ui),
            Page::Devices => views::devices::show(self, ui),
            Page::Sessions => views::sessions::show(self, ui),
            Page::Runs => {
                if self.run_detail.is_some() {
                    views::run_details::show(self, ui);
                } else {
                    views::runs::show(self, ui);
                }
            }
        });

        crate::dialogs::dialogs(self, &ctx);
        crate::dialogs::toasts(self, &ctx);
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

impl App {
    fn poll_refresh(&mut self, ctx: &egui::Context) {
        if self.runner.is_none() || self.operation_blocks_actions() {
            return;
        }
        let now = Instant::now();
        let due = |last: Option<Instant>, interval: Duration| {
            last.map(|last| now.duration_since(last) >= interval)
                .unwrap_or(true)
        };
        match self.page {
            Page::Overview => {
                if due(self.refresh.last_doctor, Duration::from_secs(60)) {
                    self.refresh_doctor(ctx);
                }
                if due(self.refresh.last_devices, Duration::from_secs(3)) {
                    self.refresh_devices(ctx);
                }
                if due(self.refresh.last_sessions, Duration::from_secs(3)) {
                    self.refresh_sessions(ctx);
                }
                if due(self.refresh.last_runs, Duration::from_secs(5)) {
                    self.refresh_runs(ctx);
                }
            }
            Page::Devices => {
                if due(self.refresh.last_devices, Duration::from_secs(3)) {
                    self.refresh_devices(ctx);
                }
                if due(self.refresh.last_sessions, Duration::from_secs(3)) {
                    self.refresh_sessions(ctx);
                }
            }
            Page::Sessions => {
                if due(self.refresh.last_sessions, Duration::from_secs(3)) {
                    self.refresh_sessions(ctx);
                }
                if due(self.refresh.last_devices, Duration::from_secs(5)) {
                    self.refresh_devices(ctx);
                }
            }
            Page::Runs => {
                if due(self.refresh.last_runs, Duration::from_secs(5)) {
                    self.refresh_runs(ctx);
                }
            }
        }
    }

    pub fn manual_refresh(&mut self, ctx: &egui::Context) {
        self.refresh.in_flight.clear();
        self.refresh_doctor(ctx);
        self.refresh_devices(ctx);
        self.refresh_sessions(ctx);
        self.refresh_runs(ctx);
        self.refresh_projects(ctx);
        if self.project_path.is_some() {
            self.refresh_project(ctx);
        }
    }

    fn spawn_event<F>(&self, ctx: &egui::Context, job: F)
    where
        F: FnOnce() -> AppEvent + Send + 'static,
    {
        let sender = self.event_tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let event = job();
            let _ = sender.send(event);
            ctx.request_repaint();
        });
    }

    pub fn invocation(&self, args: impl IntoIterator<Item = impl Into<String>>) -> Invocation {
        Invocation::new(args)
            .cwd(Some(self.workspace()))
            .timeout(Duration::from_secs(60))
    }

    fn project_invocation(&self, args: impl IntoIterator<Item = impl Into<String>>) -> Invocation {
        self.invocation(args).project(self.project_path.clone())
    }

    fn refresh_doctor(&mut self, ctx: &egui::Context) {
        if !self.refresh.in_flight.insert(RefreshKind::Doctor) {
            return;
        }
        let Some(runner) = self.runner.clone() else {
            self.refresh.in_flight.remove(&RefreshKind::Doctor);
            return;
        };
        let invocation = self
            .project_invocation(["doctor"])
            .timeout(Duration::from_secs(30));
        self.refresh.last_doctor = Some(Instant::now());
        self.doctor.loading = true;
        self.spawn_event(ctx, move || {
            AppEvent::Doctor(run_result::<DoctorResult>(&runner, invocation))
        });
    }

    fn refresh_devices(&mut self, ctx: &egui::Context) {
        if !self.refresh.in_flight.insert(RefreshKind::Devices) {
            return;
        }
        let Some(runner) = self.runner.clone() else {
            self.refresh.in_flight.remove(&RefreshKind::Devices);
            return;
        };
        let invocation = self.invocation(["devices", "list"]);
        self.refresh.last_devices = Some(Instant::now());
        self.devices.loading = true;
        self.spawn_event(ctx, move || {
            AppEvent::Devices(run_result::<DeviceList>(&runner, invocation))
        });
    }

    fn refresh_sessions(&mut self, ctx: &egui::Context) {
        if !self.refresh.in_flight.insert(RefreshKind::Sessions) {
            return;
        }
        let Some(runner) = self.runner.clone() else {
            self.refresh.in_flight.remove(&RefreshKind::Sessions);
            return;
        };
        let invocation = self.invocation(["session", "list"]);
        self.refresh.last_sessions = Some(Instant::now());
        self.sessions.loading = true;
        self.spawn_event(ctx, move || {
            AppEvent::Sessions(run_result::<SessionList>(&runner, invocation))
        });
    }

    fn refresh_runs(&mut self, ctx: &egui::Context) {
        if !self.refresh.in_flight.insert(RefreshKind::Runs) {
            return;
        }
        let Some(runner) = self.runner.clone() else {
            self.refresh.in_flight.remove(&RefreshKind::Runs);
            return;
        };
        let invocation = self
            .project_invocation(["report", "list", "--limit", "50"])
            .timeout(Duration::from_secs(30));
        self.refresh.last_runs = Some(Instant::now());
        self.runs.loading = true;
        self.spawn_event(ctx, move || {
            AppEvent::Runs(run_result::<RunList>(&runner, invocation))
        });
    }

    pub fn refresh_projects(&mut self, ctx: &egui::Context) {
        if !self.refresh.in_flight.insert(RefreshKind::Projects) {
            return;
        }
        let workspace = self.workspace();
        self.refresh.last_projects = Some(Instant::now());
        self.spawn_event(ctx, move || {
            AppEvent::Projects(project::discover(&workspace).map_err(|error| error.to_string()))
        });
    }

    fn refresh_project(&mut self, ctx: &egui::Context) {
        if !self.refresh.in_flight.insert(RefreshKind::Project) {
            return;
        }
        let Some(runner) = self.runner.clone() else {
            self.refresh.in_flight.remove(&RefreshKind::Project);
            return;
        };
        let Some(path) = self.project_path.clone() else {
            self.refresh.in_flight.remove(&RefreshKind::Project);
            return;
        };
        let base = self
            .invocation(std::iter::empty::<String>())
            .project(Some(path.clone()));
        let path_for_job = path.clone();
        self.project_error = None;
        self.spawn_event(ctx, move || {
            let config = runner
                .run_success(&Invocation {
                    args: vec!["config".into(), "show".into()],
                    ..base.clone()
                })
                .map_err(|error| error.to_string())
                .and_then(|output| {
                    parse_result::<ProjectConfig>(&output.envelope)
                        .map_err(|error| error.to_string())
                });
            let config = match config {
                Ok(config) => config,
                Err(error) => {
                    return AppEvent::Project {
                        path: path_for_job,
                        result: Err(error),
                    };
                }
            };
            let fixtures = runner
                .run_success(&Invocation {
                    args: vec!["fixture".into(), "list".into()],
                    ..base.clone()
                })
                .ok()
                .and_then(|output| parse_result::<FixtureList>(&output.envelope).ok())
                .map(|list| list.fixtures)
                .unwrap_or_default();
            let hooks = runner
                .run_success(&Invocation {
                    args: vec!["hook".into(), "list".into()],
                    ..base
                })
                .ok()
                .and_then(|output| parse_result::<HookList>(&output.envelope).ok())
                .map(|list| list.hooks)
                .unwrap_or_default();
            AppEvent::Project {
                path: path_for_job,
                result: Ok(ProjectBundle {
                    config,
                    fixtures,
                    hooks,
                }),
            }
        });
    }

    pub fn open_run(&mut self, run_id: String, ctx: &egui::Context) {
        let Some(runner) = self.runner.clone() else {
            return;
        };
        self.selected_run = Some(run_id.clone());
        self.run_loading = true;
        self.run_detail = None;
        let run_for_job = run_id.clone();
        let workspace = self.workspace();
        self.spawn_event(ctx, move || {
            let base = Invocation::new(["report"])
                .cwd(Some(workspace))
                .timeout(Duration::from_secs(30));
            let show = runner.run_success(&Invocation {
                args: vec!["report".into(), "show".into(), run_for_job.clone()],
                ..base.clone()
            });
            let report = show.map_err(|error| error.to_string()).and_then(|output| {
                parse_result::<RunReport>(&output.envelope).map_err(|error| error.to_string())
            });
            let report = match report {
                Ok(report) => report,
                Err(error) => {
                    return AppEvent::RunDetail {
                        run_id: run_for_job,
                        result: Err(error),
                    };
                }
            };
            let path_output = runner.run_success(&Invocation {
                args: vec!["report".into(), "path".into(), run_for_job.clone()],
                ..base
            });
            let path = path_output
                .map_err(|error| error.to_string())
                .and_then(|output| {
                    parse_result::<ReportPath>(&output.envelope).map_err(|error| error.to_string())
                })
                .unwrap_or_default();
            let run_dir = PathBuf::from(&path.path);
            let raw_report = evidence::read_text_preview(&run_dir.join("report.json")).ok();
            let raw_metadata = evidence::read_text_preview(&run_dir.join("metadata.json")).ok();
            let raw_manifest = evidence::read_text_preview(&run_dir.join("manifest.json")).ok();
            let manifest = evidence::verify_manifest(&run_dir);
            AppEvent::RunDetail {
                run_id: run_for_job,
                result: Ok(RunDetail {
                    report,
                    path,
                    raw_report,
                    raw_metadata,
                    raw_manifest,
                    manifest,
                }),
            }
        });
    }

    pub fn request_evidence_text(&mut self, key: String, ctx: &egui::Context) {
        let Some(runner) = self.runner.clone() else {
            return;
        };
        let Some(detail) = self.run_detail.as_ref() else {
            return;
        };
        let Some(run_dir) = Some(PathBuf::from(&detail.detail.path.path)) else {
            return;
        };
        let key_for_job = key.clone();
        if let Some(state) = self.run_detail.as_mut() {
            state.evidence.text_key = Some(key.clone());
            state.evidence.text = None;
            state.evidence.text_error = None;
        }
        self.spawn_event(ctx, move || {
            let result = evidence::safe_join(&run_dir, &key_for_job)
                .and_then(|path| evidence::read_text_preview(&path))
                .map_err(|error| error.to_string());
            AppEvent::EvidenceText {
                key: key_for_job,
                result,
            }
        });
        let _ = runner;
    }

    pub fn choose_apk(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.install.apk_path = Some(path.clone());
        self.install.info = None;
        self.install.error = None;
        self.install.preview = None;
        self.install.preview_error = None;
        self.install.result = None;
        self.install.loading = true;
        let Some(runner) = self.runner.clone() else {
            self.install.loading = false;
            self.install.error = Some("CLI is not connected".to_string());
            return;
        };
        let workspace = self.workspace();
        self.spawn_event(ctx, move || {
            let invocation = Invocation::new(["apk", "inspect", &path.display().to_string()])
                .cwd(Some(workspace))
                .timeout(Duration::from_secs(60));
            AppEvent::ApkInfo {
                path: path.clone(),
                result: run_result::<ApkInfo>(&runner, invocation),
            }
        });
    }

    pub fn preview_install(&mut self, ctx: &egui::Context) {
        let Some(path) = self.install.apk_path.clone() else {
            return;
        };
        let Some(session) = self.selected_session.clone() else {
            self.install.preview_error = Some("Select a session first".to_string());
            return;
        };
        let Some(runner) = self.runner.clone() else {
            return;
        };
        self.install.preview_loading = true;
        let invocation = self
            .project_invocation([
                "app",
                "install",
                &path.display().to_string(),
                "--dry-run",
                "--session",
                &session,
            ])
            .timeout(Duration::from_secs(120));
        self.spawn_event(ctx, move || AppEvent::InstallPreview {
            path: path.clone(),
            result: runner
                .run_success(&invocation)
                .map_err(|error| error.to_string())
                .and_then(|output| {
                    output
                        .envelope
                        .result
                        .ok_or_else(|| "CLI returned no preview".to_string())
                }),
        });
    }

    pub fn start_operation(
        &mut self,
        ctx: &egui::Context,
        title: impl Into<String>,
        kind: OperationKind,
        invocation: Invocation,
    ) {
        if self.active_operation() {
            self.toast(
                "Another desktop operation is already running",
                ToastLevel::Warning,
            );
            return;
        }
        let Some(runner) = self.runner.clone() else {
            self.toast("CLI is not connected", ToastLevel::Error);
            return;
        };
        let id = self.next_operation_id;
        self.next_operation_id += 1;
        match process_manager::spawn_operation(&runner, &invocation, id, title) {
            Ok(handle) => {
                self.operation = Some(OperationState {
                    handle,
                    kind,
                    lines: Vec::new(),
                    outcome: None,
                    cancel_requested: false,
                });
                ctx.request_repaint();
            }
            Err(error) => {
                self.toast(
                    format!("Could not start operation: {error}"),
                    ToastLevel::Error,
                );
            }
        }
    }

    fn poll_operation(&mut self, ctx: &egui::Context) {
        let Some(state) = self.operation.as_mut() else {
            return;
        };
        let mut finished = None;
        while let Ok(event) = state.handle.receiver.try_recv() {
            match event {
                OperationEvent::Stdout(line) => {
                    state.lines.push(LogLine {
                        stream: LogStream::Stdout,
                        text: line,
                    });
                }
                OperationEvent::Stderr(line) => {
                    state.lines.push(LogLine {
                        stream: LogStream::Stderr,
                        text: line,
                    });
                }
                OperationEvent::Finished(outcome) => {
                    finished = Some(outcome);
                }
            }
            if state.lines.len() > 3000 {
                state.lines.drain(0..1000);
            }
        }
        if let Some(outcome) = finished {
            state.outcome = Some(outcome);
            let kind = state.kind;
            let outcome = state.outcome.clone().expect("outcome just set");
            self.on_operation_finished(ctx, kind, &outcome);
        }
        if self
            .operation
            .as_ref()
            .is_some_and(OperationState::is_running)
        {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }

    fn on_operation_finished(
        &mut self,
        ctx: &egui::Context,
        kind: OperationKind,
        outcome: &OperationOutcome,
    ) {
        if outcome.cancelled {
            self.toast(
                "Operation cancelled. The Android session was left intact.",
                ToastLevel::Warning,
            );
        } else if outcome.success {
            self.toast(format!("{} completed", outcome.title), ToastLevel::Success);
        } else if let Some(error) = UiError::from_outcome(outcome) {
            self.error_dialog = Some(error);
        }

        if kind.refreshes_sessions() {
            self.refresh.in_flight.remove(&RefreshKind::Sessions);
            self.refresh.in_flight.remove(&RefreshKind::Devices);
            self.refresh_sessions(ctx);
            self.refresh_devices(ctx);
        }
        if kind.creates_run() {
            self.refresh.in_flight.remove(&RefreshKind::Runs);
            self.refresh_runs(ctx);
        }
        if kind == OperationKind::Install {
            if let Some(envelope) = &outcome.envelope {
                if let Ok(result) = parse_result::<InstallResult>(envelope) {
                    self.install.result = Some(result.build);
                    self.install.preview = None;
                    self.install.preview_error = None;
                    self.install.mismatch = None;
                    self.install.open = true;
                }
            }
        }
        if let Some(run_id) = outcome
            .envelope
            .as_ref()
            .and_then(|envelope| envelope.run_id.clone())
        {
            self.set_page(Page::Runs);
            self.open_run(run_id, ctx);
        }
    }

    pub fn dismiss_operation(&mut self) {
        self.operation = None;
    }

    pub fn confirm(&mut self, action: ConfirmAction) {
        self.confirm = Some(action);
    }

    pub fn execute_confirmation(&mut self, ctx: &egui::Context) {
        let Some(action) = self.confirm.take() else {
            return;
        };
        match action {
            ConfirmAction::CancelOperation => {
                if let Some(operation) = self.operation.as_mut() {
                    operation.cancel_requested = true;
                    if operation.handle.cancel() {
                        self.toast(
                            "Cancellation requested; closing process tree",
                            ToastLevel::Warning,
                        );
                    } else {
                        self.toast("Could not terminate the operation", ToastLevel::Error);
                    }
                }
            }
            ConfirmAction::ReleaseSession(session) => {
                let invocation = self.invocation([
                    "session",
                    "release",
                    "--session",
                    &session.id,
                    "--owner",
                    &session.owner,
                ]);
                self.start_operation(
                    ctx,
                    format!("Release {}", session.id),
                    OperationKind::SessionRelease,
                    invocation,
                );
            }
            ConfirmAction::StopSession(session) => {
                let mut args = vec![
                    "session".to_string(),
                    "stop".to_string(),
                    "--session".to_string(),
                    session.id.clone(),
                    "--owner".to_string(),
                    session.owner.clone(),
                ];
                if session.kind == "attached" {
                    args.push("--attached".to_string());
                }
                self.start_operation(
                    ctx,
                    format!("Stop {}", session.id),
                    OperationKind::SessionStop,
                    self.invocation(args),
                );
            }
            ConfirmAction::RestartSession(session) => {
                let invocation = self.invocation([
                    "session",
                    "restart",
                    "--session",
                    &session.id,
                    "--owner",
                    &session.owner,
                ]);
                self.start_operation(
                    ctx,
                    format!("Restart {}", session.id),
                    OperationKind::SessionRestart,
                    invocation,
                );
            }
            ConfirmAction::Install { apk_path, fresh } => {
                let Some(session) = self.selected_session.clone() else {
                    self.toast("Select a session before installing", ToastLevel::Error);
                    return;
                };
                let mut args = vec![
                    "app".to_string(),
                    "install".to_string(),
                    apk_path.display().to_string(),
                    "--session".to_string(),
                    session,
                ];
                if fresh {
                    args.push("--fresh".to_string());
                    args.push("--yes".to_string());
                }
                self.start_operation(
                    ctx,
                    format!("Install {}", apk_path.display()),
                    OperationKind::Install,
                    self.project_invocation(args),
                );
            }
            ConfirmAction::Fixture { name, yes } => {
                let Some(session) = self.selected_session.clone() else {
                    self.toast(
                        "Select a session before running a fixture",
                        ToastLevel::Error,
                    );
                    return;
                };
                let mut args = vec![
                    "fixture".to_string(),
                    "run".to_string(),
                    name.clone(),
                    "--session".to_string(),
                    session,
                ];
                if yes {
                    args.push("--yes".to_string());
                }
                self.start_operation(
                    ctx,
                    format!("Fixture {name}"),
                    OperationKind::Fixture,
                    self.project_invocation(args),
                );
            }
            ConfirmAction::Conformance { yes } => {
                let Some(session) = self.selected_session.clone() else {
                    self.toast(
                        "Select a session before running conformance",
                        ToastLevel::Error,
                    );
                    return;
                };
                let mut args = vec![
                    "conformance".to_string(),
                    "run".to_string(),
                    "--session".to_string(),
                    session,
                ];
                if yes {
                    args.push("--yes".to_string());
                }
                self.start_operation(
                    ctx,
                    "Conformance run",
                    OperationKind::Conformance,
                    self.project_invocation(args),
                );
            }
            ConfirmAction::Hook { name } => {
                let Some(session) = self.selected_session.clone() else {
                    self.toast("Select a session before running a hook", ToastLevel::Error);
                    return;
                };
                let invocation =
                    self.project_invocation(["hook", "run", &name, "--session", &session]);
                self.start_operation(ctx, format!("Hook {name}"), OperationKind::Hook, invocation);
            }
        }
    }

    pub fn select_project(&mut self, path: Option<PathBuf>, ctx: &egui::Context) {
        self.project_path = path.clone();
        self.settings.project = path.as_ref().map(|path| path.display().to_string());
        self.project = None;
        self.project_error = None;
        self.run_detail = None;
        self.selected_run = None;
        if path.is_some() {
            self.refresh_project(ctx);
        } else {
            self.refresh.in_flight.remove(&RefreshKind::Project);
        }
        self.refresh_runs(ctx);
        self.refresh_doctor(ctx);
    }

    pub fn reconnect(&mut self, ctx: &egui::Context) {
        self.connect(ctx);
    }

    pub fn open_settings(&mut self) {
        self.settings_open = true;
    }

    pub fn dismiss_error(&mut self) {
        self.error_dialog = None;
    }

    pub fn clear_run_detail(&mut self) {
        self.run_detail = None;
        self.selected_run = None;
    }
}

fn run_result<T: serde::de::DeserializeOwned>(
    runner: &CliRunner,
    invocation: Invocation,
) -> std::result::Result<T, String> {
    runner
        .run_success(&invocation)
        .and_then(|output| parse_result::<T>(&output.envelope))
        .map_err(|error| error.to_string())
}

pub fn version_label() -> String {
    format!(
        "Desktop {} · report schema {}",
        env!("CARGO_PKG_VERSION"),
        REPORT_SCHEMA_VERSION
    )
}

#[allow(dead_code)]
fn _assert_send_sync() {
    fn assert_send<T: Send>() {}
    assert_send::<AppEvent>();
}
