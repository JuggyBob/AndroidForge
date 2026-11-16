use std::path::PathBuf;

use eframe::egui;

use super::App;
use crate::backend::update;
use crate::state::*;

impl App {
    pub fn setup_test_device(&mut self, ctx: &egui::Context) {
        if self.runner.is_none() {
            self.setup.test_error = Some("Connect a compatible CLI first.".to_string());
            return;
        }
        if let Some(session) = self
            .sessions
            .value
            .sessions
            .iter()
            .find(|session| matches!(session.state.as_str(), "ready" | "attached"))
        {
            self.setup.test_ok = Some(true);
            self.setup.test_session = Some(session.id.clone());
            self.setup.created_session = false;
            self.setup.test_running = false;
            self.setup.test_error = None;
            self.setup.test_log.push(format!(
                "Using existing {} session {} on {}.",
                session.kind,
                session.id,
                session.serial.as_deref().unwrap_or("-")
            ));
            return;
        }
        let avd = if !self.setup.create_avd.trim().is_empty() {
            self.setup.create_avd.trim().to_string()
        } else if let Some(project) = self.selected_project() {
            project
                .config
                .project
                .default_avd
                .clone()
                .unwrap_or_default()
        } else {
            self.doctor.value.avds.first().cloned().unwrap_or_default()
        };
        if avd.is_empty() {
            if let Some(device) = self.devices.value.devices.first() {
                self.setup.test_running = true;
                self.setup
                    .test_log
                    .push(format!("Attaching to existing device {}.", device.serial));
                self.setup.created_session = true;
                self.start_operation(
                    ctx,
                    format!("Attach {}", device.serial),
                    OperationKind::SessionAttach,
                    self.invocation([
                        "session".to_string(),
                        "attach".to_string(),
                        "--serial".to_string(),
                        device.serial.clone(),
                        "--owner".to_string(),
                        self.settings.owner.clone(),
                    ]),
                );
            } else {
                self.setup.test_error = Some(
                    "No AVD is available. Create an AVD or attach a device, then recheck."
                        .to_string(),
                );
            }
            return;
        }
        self.setup.test_running = true;
        self.setup.test_ok = None;
        self.setup.test_session = None;
        self.setup.test_error = None;
        self.setup.created_session = true;
        self.setup
            .test_log
            .push(format!("Starting a temporary session on AVD {avd}."));
        self.start_operation(
            ctx,
            format!("Setup test on {avd}"),
            OperationKind::SessionStart,
            self.invocation([
                "session".to_string(),
                "start".to_string(),
                "--avd".to_string(),
                avd,
                "--owner".to_string(),
                self.settings.owner.clone(),
                "--headless".to_string(),
                "--reuse".to_string(),
            ]),
        );
    }

    pub fn setup_create_project(&mut self, ctx: &egui::Context) {
        if self.setup.create_name.trim().is_empty() || self.setup.create_package.trim().is_empty() {
            self.setup.create_error = Some("Project name and package id are required.".to_string());
            return;
        }
        if self.runner.is_none() {
            self.setup.create_error = Some("CLI is not connected.".to_string());
            return;
        }
        let Some(runner) = self.runner.clone() else {
            return;
        };
        let workspace = self.workspace();
        let mut args = vec![
            "init".to_string(),
            "--name".to_string(),
            self.setup.create_name.trim().to_string(),
            "--package".to_string(),
            self.setup.create_package.trim().to_string(),
        ];
        if !self.setup.create_avd.trim().is_empty() {
            args.push("--avd".to_string());
            args.push(self.setup.create_avd.trim().to_string());
        }
        let invocation = self.invocation(args);
        let target = workspace.display().to_string();
        self.setup.create_busy = true;
        self.setup.create_error = None;
        self.spawn_event(ctx, move || {
            let result = runner
                .run_success(&invocation)
                .map(|_| target)
                .map_err(|error| error.to_string());
            AppEvent::SetupProject(result)
        });
    }

    pub fn finish_onboarding(&mut self, ctx: &egui::Context) {
        self.settings.onboarding_completed = true;
        let session = self.setup.test_session.clone();
        let created = self.setup.created_session;
        if created && let Some(session) = session {
            let owner = self
                .sessions
                .value
                .sessions
                .iter()
                .find(|entry| entry.id == session)
                .map(|entry| entry.owner.clone())
                .unwrap_or_else(|| self.settings.owner.clone());
            self.start_operation(
                ctx,
                format!("Release setup session {session}"),
                OperationKind::SessionRelease,
                self.invocation([
                    "session".to_string(),
                    "release".to_string(),
                    "--session".to_string(),
                    session,
                    "--owner".to_string(),
                    owner,
                ]),
            );
        }
        self.setup.test_session = None;
        self.setup.created_session = false;
        self.set_page(Page::Overview);
        self.toast("Setup complete", ToastLevel::Success);
        self.manual_refresh(ctx);
    }

    pub fn restart_onboarding(&mut self) {
        self.settings.onboarding_completed = false;
        self.setup = SetupState::default();
        self.open_setup(SetupSection::Welcome);
    }

    pub fn check_updates(&mut self, ctx: &egui::Context) {
        self.update.loading = true;
        self.update.error = None;
        self.spawn_event(ctx, move || {
            AppEvent::Update(update::check_latest().map_err(|error| error.to_string()))
        });
    }

    pub fn stage_cli_update(&mut self, ctx: &egui::Context) {
        let Some(check) = self.update.value.clone() else {
            self.toast("Check for updates first", ToastLevel::Warning);
            return;
        };
        if !check.update_available {
            self.toast("The installed CLI is already current", ToastLevel::Info);
            return;
        }
        self.update.repair_busy = true;
        self.update.repair_message = Some("Downloading and verifying CLI...".to_string());
        self.spawn_event(ctx, move || {
            AppEvent::CliUpdateStaged(
                update::stage_cli_update(&check).map_err(|error| error.to_string()),
            )
        });
    }

    pub fn open_releases_page(&self) {
        let _ = open::that(update::RELEASES_URL);
    }

    pub fn set_workspace(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.settings.workspace = Some(path.display().to_string());
        self.projects.clear();
        self.project_path = None;
        self.settings.project = None;
        self.refresh_projects(ctx);
    }

    pub fn choose_cli_path(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.settings.cli_path = Some(path.display().to_string());
        self.connect(ctx);
    }

    pub fn clear_cli_path(&mut self, ctx: &egui::Context) {
        self.settings.cli_path = None;
        self.connect(ctx);
    }
}
