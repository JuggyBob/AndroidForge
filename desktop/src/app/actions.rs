use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;

use super::{App, run_result};
use crate::backend::cli_runner::Invocation;
use crate::backend::evidence;
use crate::backend::json_parser::parse_result;
use crate::backend::process_manager::{self, OperationEvent};
use crate::backend::update;
use crate::models::{ApkInfo, InstallResult, ReconcileResult, ReportPath, RunReport};
use crate::state::*;

impl App {
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

    pub fn open_triage(&mut self, run_id: Option<String>, ctx: &egui::Context) {
        let run_id = run_id.or_else(|| {
            let runs = &self.runs.value.runs;
            runs.iter()
                .find(|run| matches!(run.status.as_str(), "failed" | "partial"))
                .or_else(|| {
                    runs.iter().find(|run| {
                        matches!(
                            run.status.as_str(),
                            "cancelled" | "interrupted" | "timed_out" | "timed-out" | "abandoned"
                        )
                    })
                })
                .or_else(|| runs.first())
                .map(|run| run.run_id.clone())
        });
        let Some(run_id) = run_id else {
            self.toast("No runs are available to triage", ToastLevel::Warning);
            return;
        };
        self.triage.run_id = Some(run_id.clone());
        self.triage.error = None;
        self.triage.flaky = None;
        self.set_page(Page::Triage);
        self.open_run(run_id, ctx);
        self.refresh_flaky(ctx);
    }

    pub fn request_evidence_text(&mut self, key: String, ctx: &egui::Context) {
        let Some(detail) = self.run_detail.as_ref() else {
            return;
        };
        let run_dir = PathBuf::from(&detail.detail.path.path);
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

    pub(super) fn poll_operation(&mut self, ctx: &egui::Context) {
        let Some(state) = self.operation.as_mut() else {
            return;
        };
        let mut finished = None;
        while let Ok(event) = state.handle.receiver.try_recv() {
            match event {
                OperationEvent::Stdout(text) => {
                    state.lines.push(LogLine {
                        stream: LogStream::Stdout,
                        text,
                    });
                }
                OperationEvent::Stderr(text) => {
                    state.lines.push(LogLine {
                        stream: LogStream::Stderr,
                        text,
                    });
                }
                OperationEvent::Finished(outcome) => finished = Some(outcome),
            }
        }
        if state.lines.len() > 10_000 {
            state.lines.drain(0..state.lines.len() - 10_000);
        }
        let Some(outcome) = finished else {
            return;
        };
        let kind = state.kind;
        state.outcome = Some(outcome.clone());
        if !outcome.cancelled
            && let Some(error) = UiError::from_outcome(&outcome)
        {
            self.error_dialog = Some(error);
        }
        if !outcome.cancelled {
            match (&outcome.status_code, outcome.success) {
                (Some(_), true) => {
                    self.toast(format!("{} completed", outcome.title), ToastLevel::Success)
                }
                (Some(code), false) => self.toast(
                    format!("{} failed (exit {code})", outcome.title),
                    ToastLevel::Error,
                ),
                (None, _) => self.toast(
                    format!("{} ended without an exit code", outcome.title),
                    ToastLevel::Warning,
                ),
            }
        }
        if matches!(
            kind,
            OperationKind::SessionStart | OperationKind::SessionAttach
        ) && self.setup.test_running
        {
            self.setup.test_running = false;
            match outcome
                .envelope
                .as_ref()
                .and_then(|envelope| envelope.session_id.clone())
            {
                Some(session) => {
                    self.setup.test_ok = Some(true);
                    self.setup.test_session = Some(session.clone());
                    self.setup.test_error = None;
                    self.setup
                        .test_log
                        .push(format!("Session {session} is ready."));
                }
                None => {
                    self.setup.test_ok = Some(false);
                    self.setup.test_error = outcome.parse_error.clone().or_else(|| {
                        Some("the session start did not return a session id".to_string())
                    });
                }
            }
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
        if kind == OperationKind::Maintenance {
            self.maintenance.cleaning = false;
            if outcome.success {
                self.maintenance.preview = None;
                self.maintenance.preview_error = None;
                self.maintenance.cleanup_request = None;
            }
            self.refresh.in_flight.remove(&RefreshKind::Maintenance);
            self.refresh_maintenance(ctx);
            self.refresh.in_flight.remove(&RefreshKind::Runs);
            self.refresh_runs(ctx);
            self.refresh.in_flight.remove(&RefreshKind::Sessions);
            self.refresh_sessions(ctx);
            self.maintenance.last_result = Some(if outcome.success {
                "Maintenance completed.".to_string()
            } else {
                "Maintenance failed; inspect the output below.".to_string()
            });
        }
        if kind == OperationKind::Install
            && let Some(envelope) = &outcome.envelope
            && let Ok(result) = parse_result::<InstallResult>(envelope)
        {
            self.install.result = Some(result.build);
            self.install.preview = None;
            self.install.preview_error = None;
            self.install.mismatch = None;
            self.install.open = true;
        }
        if outcome.cancelled && kind.creates_run() {
            self.reconcile_cancelled_run(ctx);
        }
        if let Some(run_id) = outcome
            .envelope
            .as_ref()
            .and_then(|envelope| envelope.run_id.clone())
            && self.page != Page::Setup
        {
            self.open_run(run_id, ctx);
        }
        ctx.request_repaint();
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
                        self.toast(
                            "Cancellation had no effect: the operation had already finished or its process could not be terminated",
                            ToastLevel::Warning,
                        );
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
                    "Install APK",
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
                self.start_operation(
                    ctx,
                    format!("Hook {name}"),
                    OperationKind::Hook,
                    self.project_invocation([
                        "hook".to_string(),
                        "run".to_string(),
                        name.clone(),
                        "--session".to_string(),
                        session,
                    ]),
                );
            }
            ConfirmAction::Cleanup(request) => {
                let mut args = vec![
                    "maintenance".to_string(),
                    "clean".to_string(),
                    "--older-than".to_string(),
                    format!("{}d", request.older_days.max(1)),
                    "--keep-last".to_string(),
                    request.keep_last.to_string(),
                    "--yes".to_string(),
                ];
                if request.sessions {
                    args.push("--sessions".to_string());
                }
                if request.runs {
                    args.push("--runs".to_string());
                }
                self.maintenance.cleaning = true;
                self.maintenance.last_result = None;
                self.start_operation(
                    ctx,
                    "Maintenance cleanup",
                    OperationKind::Maintenance,
                    self.project_invocation(args),
                );
            }
            ConfirmAction::InstallCli {
                staged,
                target,
                version,
            } => {
                self.update.repair_busy = true;
                self.update.repair_message = Some(format!("Installing CLI {version}..."));
                self.spawn_event(ctx, move || {
                    AppEvent::CliUpdateInstalled(
                        update::install_staged_cli(&staged, &target)
                            .map(|backup| {
                                format!(
                                    "CLI {version} installed. Previous executable kept at {}.",
                                    backup.display()
                                )
                            })
                            .map_err(|error| error.to_string()),
                    )
                });
            }
        }
    }

    pub fn dismiss_operation(&mut self) {
        self.operation = None;
    }

    pub fn clear_run_detail(&mut self) {
        self.run_detail = None;
        self.run_loading = false;
    }

    pub fn reconnect(&mut self, ctx: &egui::Context) {
        self.connect(ctx);
    }

    pub fn confirm(&mut self, action: ConfirmAction) {
        self.confirm = Some(action);
    }

    pub(super) fn reconcile_runs(&mut self, ctx: &egui::Context) {
        let Some(runner) = self.runner.clone() else {
            return;
        };
        let invocation = self
            .project_invocation(["report", "reconcile"])
            .timeout(Duration::from_secs(30));
        self.last_reconcile = Some(Instant::now());
        self.spawn_event(ctx, move || {
            AppEvent::Reconcile(run_result::<ReconcileResult>(&runner, invocation))
        });
    }

    pub(super) fn reconcile_cancelled_run(&mut self, ctx: &egui::Context) {
        let Some(runner) = self.runner.clone() else {
            return;
        };
        let invocation = self
            .project_invocation([
                "report",
                "reconcile",
                "--latest-running",
                "--outcome",
                "cancelled",
            ])
            .timeout(Duration::from_secs(30));
        self.spawn_event(ctx, move || {
            AppEvent::Reconcile(run_result::<ReconcileResult>(&runner, invocation))
        });
    }

    pub fn refresh_maintenance(&mut self, ctx: &egui::Context) {
        if !self.refresh.in_flight.insert(RefreshKind::Maintenance) {
            return;
        }
        let Some(runner) = self.runner.clone() else {
            self.refresh.in_flight.remove(&RefreshKind::Maintenance);
            return;
        };
        let invocation = self
            .project_invocation(["maintenance", "summary"])
            .timeout(Duration::from_secs(30));
        self.refresh.last_maintenance = Some(Instant::now());
        self.maintenance.summary.loading = true;
        self.spawn_event(ctx, move || {
            AppEvent::Maintenance(run_result::<crate::models::MaintenanceSummary>(
                &runner, invocation,
            ))
        });
    }

    pub fn refresh_flaky(&mut self, ctx: &egui::Context) {
        let Some(runner) = self.runner.clone() else {
            return;
        };
        self.triage.flaky_loading = true;
        let invocation = self
            .project_invocation(["report", "flaky", "--limit", "30", "--min-runs", "3"])
            .timeout(Duration::from_secs(30));
        self.spawn_event(ctx, move || {
            AppEvent::Flaky(run_result::<crate::models::FlakyReport>(
                &runner, invocation,
            ))
        });
    }

    pub fn start_comparison(&mut self, ctx: &egui::Context) {
        let Some(current) = self
            .compare_form
            .current
            .clone()
            .or_else(|| self.compare.current.clone())
            .or_else(|| self.runs.value.runs.first().map(|run| run.run_id.clone()))
        else {
            self.compare.error = Some("Select a current run first.".to_string());
            return;
        };
        let Some(runner) = self.runner.clone() else {
            self.compare.error = Some("CLI is not connected.".to_string());
            return;
        };
        self.compare.current = Some(current.clone());
        self.compare.baseline = self.compare_form.baseline.clone();
        self.compare.loading = true;
        self.compare.error = None;
        let mut args = vec!["report".to_string(), "compare".to_string(), current.clone()];
        if let Some(baseline) = &self.compare.baseline {
            args.push("--baseline".to_string());
            args.push(baseline.clone());
        }
        let invocation = self
            .project_invocation(args)
            .timeout(Duration::from_secs(60));
        self.spawn_event(ctx, move || {
            AppEvent::Comparison(run_result::<crate::models::RunComparison>(
                &runner, invocation,
            ))
        });
    }

    pub fn choose_compare_current(&mut self, run_id: String) {
        self.compare_form.current = Some(run_id);
    }

    pub fn choose_compare_baseline(&mut self, run_id: Option<String>) {
        self.compare_form.baseline = run_id;
    }

    pub fn preview_cleanup(&mut self, ctx: &egui::Context) {
        let Some(runner) = self.runner.clone() else {
            return;
        };
        let request = CleanupRequest {
            older_days: self.maintenance.older_days.max(1),
            keep_last: self.maintenance.keep_last,
            sessions: self.maintenance.include_sessions,
            runs: self.maintenance.include_runs,
        };
        let mut args = vec![
            "maintenance".to_string(),
            "clean".to_string(),
            "--dry-run".to_string(),
            "--older-than".to_string(),
            format!("{}d", request.older_days),
            "--keep-last".to_string(),
            request.keep_last.to_string(),
        ];
        if request.sessions {
            args.push("--sessions".to_string());
        }
        if request.runs {
            args.push("--runs".to_string());
        }
        self.maintenance.preview_loading = true;
        self.maintenance.preview_error = None;
        let invocation = self
            .project_invocation(args)
            .timeout(Duration::from_secs(30));
        self.spawn_event(ctx, move || AppEvent::MaintenancePreview {
            request,
            result: runner
                .run_success(&invocation)
                .map_err(|error| error.to_string())
                .and_then(|output| {
                    output
                        .envelope
                        .result
                        .ok_or_else(|| "CLI returned no cleanup preview".to_string())
                }),
        });
    }

    pub fn request_cleanup_confirmation(&mut self) {
        if let Some(request) = &self.maintenance.cleanup_request {
            self.confirm(ConfirmAction::Cleanup(request.clone()));
        } else {
            self.toast("Preview cleanup before deleting data", ToastLevel::Warning);
        }
    }
}
