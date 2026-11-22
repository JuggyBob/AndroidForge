use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::backend::DesktopError;
use crate::backend::cli_locator;
use crate::backend::cli_runner::{CliRunner, Invocation};
use crate::backend::compatibility;
use crate::backend::project::{self, DiscoveredProject};
use crate::components;
use crate::models::{
    Device, DeviceList, DoctorResult, FixtureList, HookList, ProjectConfig, RunList, SessionInfo,
    SessionList,
};
use crate::state::*;
use crate::theme;
use crate::views;

mod actions;
mod events;
pub(crate) mod live;
mod setup;

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
    pub nav_rects: HashMap<Page, egui::Rect>,
    pub nav_highlight_y: Option<f32>,
    pub page_animation_started: Instant,
    pub page_scroll_reset: bool,
    pub chrome_dark: Option<bool>,
    pub accent_wheel: Option<egui::TextureHandle>,
    pub font_search: String,
    pub setup: SetupState,
    pub update: UpdateState,
    pub triage: TriageState,
    pub context: ContextState,
    pub health: Option<serde_json::Value>,
    pub health_loading: bool,
    pub health_error: Option<String>,
    pub health_serial: Option<String>,
    pub wireless_endpoint: String,
    pub wireless_code: String,
    pub wireless_result: Option<serde_json::Value>,
    pub wireless_error: Option<String>,
    pub wireless_loading: bool,
    pub matrix: MatrixState,
    pub compare: CompareState,
    pub compare_form: CompareForm,
    pub logcat: LogcatState,
    pub inspector: InspectorState,
    pub maintenance: MaintenanceState,
    pub event_tx: mpsc::Sender<AppEvent>,
    pub event_rx: mpsc::Receiver<AppEvent>,
    pub next_operation_id: u64,
    pub started: Instant,
    pub last_reconcile: Option<Instant>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::new_with_page(cc, None)
    }

    pub fn new_with_page(cc: &eframe::CreationContext<'_>, initial_page: Option<Page>) -> Self {
        let settings = cc
            .storage
            .and_then(|storage| storage.get_string(SETTINGS_KEY))
            .and_then(|text| serde_json::from_str::<DesktopSettings>(&text).ok())
            .unwrap_or_default();

        theme::set_accent(egui::Color32::from_rgb(
            settings.accent[0],
            settings.accent[1],
            settings.accent[2],
        ));
        crate::fonts::apply(&cc.egui_ctx, settings.font.as_deref());
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
            page: initial_page.unwrap_or(Page::Overview),
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
            nav_rects: HashMap::new(),
            nav_highlight_y: None,
            page_animation_started: Instant::now(),
            page_scroll_reset: true,
            chrome_dark: None,
            accent_wheel: None,
            font_search: String::new(),
            setup: SetupState::default(),
            update: UpdateState::default(),
            triage: TriageState::default(),
            context: ContextState::default(),
            health: None,
            health_loading: false,
            health_error: None,
            health_serial: None,
            wireless_endpoint: String::new(),
            wireless_code: String::new(),
            wireless_result: None,
            wireless_error: None,
            wireless_loading: false,
            matrix: MatrixState::default(),
            compare: CompareState::default(),
            compare_form: CompareForm::default(),
            logcat: LogcatState::default(),
            inspector: InspectorState::default(),
            maintenance: MaintenanceState {
                older_days: 30,
                keep_last: 10,
                include_sessions: true,
                include_runs: true,
                ..Default::default()
            },
            event_tx,
            event_rx,
            next_operation_id: 1,
            started: Instant::now(),
            last_reconcile: None,
        };
        app.applied_theme = app.settings.theme;
        app.connect(&cc.egui_ctx);
        if !app.settings.onboarding_completed {
            if app.cli.info.is_some() && app.project_path.is_some() {
                app.settings.onboarding_completed = true;
                app.page = Page::Overview;
            } else {
                app.page = Page::Setup;
            }
        }
        if app.settings.update_check_on_start {
            app.check_updates(&cc.egui_ctx);
        }
        if let Some(page) = initial_page {
            app.page = page;
        }
        app
    }

    pub fn connect(&mut self, ctx: &egui::Context) {
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
                if !self.settings.onboarding_completed {
                    self.page = Page::Setup;
                }
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
                self.refresh_maintenance(ctx);
                if self.project_path.is_some() {
                    self.refresh_project(ctx);
                }
                self.reconcile_runs(ctx);
                if self.page == Page::Setup && self.settings.onboarding_completed {
                    self.page = Page::Overview;
                }
            }
            Err(error) => {
                self.cli.error = Some(desktop_error_message(error));
                self.cli.resolution = Some(resolution);
                if !self.settings.onboarding_completed {
                    self.page = Page::Setup;
                }
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
            self.page_animation_started = Instant::now();
            self.page_scroll_reset = true;
            if !matches!(page, Page::Runs | Page::Triage) {
                self.run_detail = None;
            }
            if page == Page::Logcat && self.logcat.session_id.is_none() {
                self.logcat.session_id = self.default_live_session();
            }
            if page == Page::Inspector && self.inspector.session_id.is_none() {
                self.inspector.session_id = self.default_live_session();
            }
        }
    }

    pub fn open_settings(&mut self) {
        self.settings_open = false;
        self.set_page(Page::Settings);
    }

    pub fn open_cli(&self) {
        let path = self
            .cli
            .resolution
            .as_ref()
            .map(|resolution| resolution.path.clone())
            .or_else(|| self.cli.configured_path.clone());
        let Some(path) = path else {
            return;
        };
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
            // /K keeps the console window open after the CLI exits, instead
            // of flashing a window and immediately closing it. raw_arg avoids
            // Rust's automatic argument escaping around the quoted path.
            let command = format!("\"{}\"", path.display());
            let mut console = std::process::Command::new("cmd.exe");
            console
                .arg("/K")
                .raw_arg(command)
                .current_dir(self.workspace())
                .creation_flags(CREATE_NEW_CONSOLE);
            // Make the CLI resolvable as `androidforge` after the initial
            // full-path invocation, so the console is actually usable.
            if let Some(directory) = path.parent() {
                let mut search_path = vec![directory.to_path_buf()];
                if let Some(existing) = std::env::var_os("PATH") {
                    search_path.extend(std::env::split_paths(&existing));
                }
                if let Ok(joined) = std::env::join_paths(search_path) {
                    console.env("PATH", joined);
                }
            }
            let _ = console.spawn();
        }
        #[cfg(not(windows))]
        {
            let _ = std::process::Command::new(path).spawn();
        }
    }

    pub fn toast(&mut self, text: impl Into<String>, level: ToastLevel) {
        self.toasts.push(Toast::new(text, level));
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    pub fn invocation(&self, args: impl IntoIterator<Item = impl Into<String>>) -> Invocation {
        Invocation::new(args)
            .cwd(Some(self.workspace()))
            .timeout(Duration::from_secs(60))
    }

    pub fn project_invocation(
        &self,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Invocation {
        self.invocation(args).project(self.project_path.clone())
    }

    pub fn spawn_event<F>(&self, ctx: &egui::Context, job: F)
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

    pub fn manual_refresh(&mut self, ctx: &egui::Context) {
        self.refresh.in_flight.clear();
        self.refresh_doctor(ctx);
        self.refresh_devices(ctx);
        self.refresh_sessions(ctx);
        self.refresh_runs(ctx);
        self.refresh_projects(ctx);
        self.refresh_maintenance(ctx);
        if self.project_path.is_some() {
            self.refresh_project(ctx);
        }
    }

    pub fn select_project(&mut self, path: Option<PathBuf>, ctx: &egui::Context) {
        if self.project_path == path {
            return;
        }
        self.project_path = path;
        self.settings.project = self
            .project_path
            .as_ref()
            .map(|path| path.display().to_string());
        self.project = None;
        self.project_error = None;
        self.run_detail = None;
        self.runs = DataState::default();
        if self.project_path.is_some() {
            self.refresh_project(ctx);
            self.refresh_runs(ctx);
        }
        ctx.request_repaint();
    }

    pub fn open_setup(&mut self, section: SetupSection) {
        self.set_setup_section(section);
        self.set_page(Page::Setup);
    }

    pub fn set_setup_section(&mut self, section: SetupSection) {
        if self.setup.section != section {
            self.setup.section = section;
            self.setup.section_animation_started = Some(Instant::now());
            self.page_scroll_reset = true;
        }
    }

    fn refresh_doctor(&mut self, ctx: &egui::Context) {
        if !self.refresh.in_flight.insert(RefreshKind::Doctor) {
            return;
        }
        let Some(runner) = self.runner.clone() else {
            self.refresh.in_flight.remove(&RefreshKind::Doctor);
            return;
        };
        let invocation = self.project_invocation(["doctor"]);
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
            .project_invocation(["report", "list", "--limit", "100"])
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
                    crate::backend::json_parser::parse_result::<ProjectConfig>(&output.envelope)
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
                .and_then(|output| {
                    crate::backend::json_parser::parse_result::<FixtureList>(&output.envelope).ok()
                })
                .map(|list| list.fixtures)
                .unwrap_or_default();
            let hooks = runner
                .run_success(&Invocation {
                    args: vec!["hook".into(), "list".into()],
                    ..base
                })
                .ok()
                .and_then(|output| {
                    crate::backend::json_parser::parse_result::<HookList>(&output.envelope).ok()
                })
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

    fn poll_refresh(&mut self, ctx: &egui::Context) {
        if self.runner.is_none() || self.operation_blocks_actions() {
            return;
        }
        let now = Instant::now();
        let due = |last: Option<Instant>, interval: Duration| {
            last.map(|last| now.duration_since(last) >= interval)
                .unwrap_or(true)
        };
        if matches!(
            self.page,
            Page::Overview
                | Page::Context
                | Page::Devices
                | Page::Sessions
                | Page::Ownership
                | Page::Runs
                | Page::Triage
                | Page::Compare
        ) {
            if due(self.refresh.last_doctor, Duration::from_secs(60)) {
                self.refresh_doctor(ctx);
            }
            if due(self.refresh.last_devices, Duration::from_secs(4)) {
                self.refresh_devices(ctx);
            }
            if due(self.refresh.last_sessions, Duration::from_secs(4)) {
                self.refresh_sessions(ctx);
            }
            if due(self.refresh.last_runs, Duration::from_secs(6)) {
                self.refresh_runs(ctx);
            }
        }
        if self.page == Page::Maintenance
            && due(self.refresh.last_maintenance, Duration::from_secs(15))
        {
            self.refresh_maintenance(ctx);
        }
        if self.page == Page::Context
            && self.context.snapshot.is_none()
            && !self.context.loading
            && due(self.context.updated, Duration::from_secs(10))
        {
            self.refresh_context(ctx);
        }
    }

    fn default_live_session(&self) -> Option<String> {
        let is_live = |session: &SessionInfo| {
            matches!(session.state.as_str(), "ready" | "attached" | "starting")
        };
        self.selected_session
            .as_ref()
            .and_then(|id| {
                self.sessions
                    .value
                    .sessions
                    .iter()
                    .find(|session| &session.id == id)
            })
            .filter(|session| is_live(session))
            .map(|session| session.id.clone())
            .or_else(|| {
                self.sessions
                    .value
                    .sessions
                    .iter()
                    .find(|session| is_live(session))
                    .map(|session| session.id.clone())
            })
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

pub(crate) fn run_result<T: serde::de::DeserializeOwned>(
    runner: &CliRunner,
    invocation: Invocation,
) -> std::result::Result<T, String> {
    runner
        .run_success(&invocation)
        .map_err(|error| error.to_string())
        .and_then(|output| {
            crate::backend::json_parser::parse_result::<T>(&output.envelope)
                .map_err(|error| error.to_string())
        })
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(text) = serde_json::to_string(&self.settings) {
            storage.set_string(SETTINGS_KEY, text);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.applied_theme != self.settings.theme {
            self.applied_theme = self.settings.theme;
            theme::apply(&ctx, self.settings.theme);
        }
        self.drain_events(&ctx);
        self.poll_operation(&ctx);
        self.poll_refresh(&ctx);
        self.poll_logcat(&ctx);
        self.poll_inspector(&ctx);

        let dark = ui.style().visuals.dark_mode;
        if self.chrome_dark != Some(dark) {
            crate::platform::apply_titlebar(frame, dark);
            self.chrome_dark = Some(dark);
        }
        let panel = if dark {
            theme::DARK_PANEL
        } else {
            theme::LIGHT_PANEL
        };
        egui::Panel::top("androidforge-topbar")
            .exact_size(54.0)
            .frame(
                egui::Frame::new()
                    .fill(panel)
                    .stroke(Stroke::new(1.0, theme::border_color(ui)))
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ui, |ui| components::topbar(self, ui));
        egui::Panel::left("androidforge-sidebar")
            .exact_size(222.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(panel)
                    .stroke(Stroke::new(1.0, theme::border_color(ui)))
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(ui, |ui| components::sidebar(self, ui));

        if self.operation.is_some() {
            egui::Panel::bottom("androidforge-operation")
                .resizable(true)
                .default_size(190.0)
                .min_size(120.0)
                .max_size(420.0)
                .frame(
                    egui::Frame::new()
                        .fill(panel)
                        .stroke(Stroke::new(1.0, theme::border_color(ui)))
                        .inner_margin(egui::Margin::symmetric(12, 8)),
                )
                .show(ui, |ui| views::operations::show(self, ui));
        }

        if self.page == Page::Setup {
            egui::Panel::bottom("androidforge-setup-footer")
                .exact_size(56.0)
                .frame(
                    egui::Frame::new()
                        .fill(panel)
                        .stroke(Stroke::new(1.0, theme::border_color(ui)))
                        .inner_margin(egui::Margin::symmetric(18, 8)),
                )
                .show(ui, |ui| views::setup::footer(self, ui));
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(if dark {
                        theme::DARK_BG
                    } else {
                        theme::LIGHT_BG
                    })
                    .inner_margin(egui::Margin::symmetric(18, 14)),
            )
            .show(ui, |ui| {
                let mut scroll = egui::ScrollArea::vertical()
                    .id_salt("main-page-scroll")
                    .auto_shrink([false, false]);
                if self.page_scroll_reset {
                    scroll = scroll.vertical_scroll_offset(0.0);
                    self.page_scroll_reset = false;
                }
                scroll.show(ui, |ui| views::page(self, ui));
            });

        crate::dialogs::dialogs(self, &ctx);
        crate::dialogs::toasts(self, &ctx);
        if self.logcat.running || self.inspector.auto_refresh {
            ctx.request_repaint_after(Duration::from_millis(350));
        } else {
            ctx.request_repaint_after(Duration::from_millis(750));
        }
    }
}

use egui::Stroke;
