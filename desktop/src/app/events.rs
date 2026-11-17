use std::path::PathBuf;
use std::time::Instant;

use eframe::egui;

use super::App;
use crate::backend::update;
use crate::models::*;
use crate::state::*;

impl App {
    pub(super) fn drain_events(&mut self, ctx: &egui::Context) {
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
                        if let Some(selected) = &self.selected_session
                            && !value.sessions.iter().any(|session| &session.id == selected)
                        {
                            self.selected_session = None;
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
                        if let Some(path) = &self.project_path
                            && !self.projects.iter().any(|project| &project.root == path)
                        {
                            self.project_error =
                                Some("The selected project is no longer discoverable.".to_string());
                        }
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
            AppEvent::Update(result) => {
                self.update.loading = false;
                self.update.last_check = Some(Instant::now());
                match result {
                    Ok(value) => {
                        if value.update_available {
                            self.toast(
                                format!(
                                    "AndroidForge {} is available (running {})",
                                    value.latest_version, value.current_version
                                ),
                                ToastLevel::Info,
                            );
                        }
                        self.update.value = Some(value);
                        self.update.error = None;
                    }
                    Err(error) => self.update.error = Some(error),
                }
            }
            AppEvent::Reconcile(result) => {
                self.last_reconcile = Some(Instant::now());
                match result {
                    Ok(report) if !report.reconciled.is_empty() => {
                        let count = report.reconciled.len();
                        self.toast(
                            format!("Reconciled {count} unfinished run(s)"),
                            ToastLevel::Warning,
                        );
                        self.refresh_runs(ctx);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        self.toast(
                            format!("Run reconciliation failed: {error}"),
                            ToastLevel::Error,
                        );
                    }
                }
            }
            AppEvent::Comparison(result) => {
                self.compare.loading = false;
                match result {
                    Ok(value) => {
                        self.compare.comparison = Some(value);
                        self.compare.error = None;
                    }
                    Err(error) => {
                        self.compare.error = Some(error);
                        self.compare.comparison = None;
                    }
                }
            }
            AppEvent::Flaky(result) => {
                self.triage.flaky_loading = false;
                match result {
                    Ok(value) => {
                        self.triage.flaky = Some(value);
                        self.triage.flaky_error = None;
                    }
                    Err(error) => self.triage.flaky_error = Some(error),
                }
            }
            AppEvent::Maintenance(result) => {
                self.refresh.in_flight.remove(&RefreshKind::Maintenance);
                self.maintenance.summary.loading = false;
                match result {
                    Ok(value) => {
                        self.maintenance.summary.value = value;
                        self.maintenance.summary.updated = Some(Instant::now());
                        self.maintenance.summary.error = None;
                    }
                    Err(error) => self.maintenance.summary.error = Some(error),
                }
            }
            AppEvent::MaintenancePreview { request, result } => {
                self.maintenance.preview_loading = false;
                match result {
                    Ok(value) => {
                        self.maintenance.preview = Some(value);
                        self.maintenance.preview_error = None;
                        self.maintenance.cleanup_request = Some(request);
                    }
                    Err(error) => {
                        self.maintenance.preview = None;
                        self.maintenance.preview_error = Some(error);
                    }
                }
            }
            AppEvent::Logcat(result) => {
                self.logcat.loading = false;
                self.logcat.last_capture = Some(Instant::now());
                match result {
                    Ok(peek) => {
                        self.logcat.retry = peek.retry.clone();
                        self.merge_logcat(peek);
                        self.logcat.error = None;
                    }
                    Err(error) => {
                        self.logcat.error = Some(error);
                    }
                }
            }
            AppEvent::Frame(result) => {
                self.inspector.frame_loading = false;
                self.inspector.last_frame = Some(Instant::now());
                match result {
                    Ok(frame) => {
                        self.inspector.last_frame_path = Some(frame.path.clone());
                        match std::fs::read(&frame.path) {
                            Ok(bytes) => match image::load_from_memory(&bytes) {
                                Ok(image) => {
                                    let image = image.to_rgba8();
                                    let size = [image.width() as usize, image.height() as usize];
                                    let color = egui::ColorImage::from_rgba_unmultiplied(
                                        size,
                                        image.as_raw(),
                                    );
                                    let texture = ctx.load_texture(
                                        format!("inspector-frame-{}", frame.captured_at),
                                        color,
                                        egui::TextureOptions::LINEAR,
                                    );
                                    self.inspector.image = Some(texture);
                                    self.inspector.image_size =
                                        Some((image.width(), image.height()));
                                    self.inspector.display_size = frame.display_size;
                                    self.inspector.frame_error = None;
                                }
                                Err(error) => self.inspector.frame_error = Some(error.to_string()),
                            },
                            Err(error) => self.inspector.frame_error = Some(error.to_string()),
                        }
                    }
                    Err(error) => self.inspector.frame_error = Some(error),
                }
            }
            AppEvent::Hierarchy(result) => {
                self.inspector.hierarchy_loading = false;
                self.inspector.last_hierarchy = Some(Instant::now());
                match result {
                    Ok(value) => {
                        let hierarchy = value.hierarchy;
                        if let Some(selected) = self.inspector.selected_node
                            && !hierarchy.nodes.iter().any(|node| node.id == selected)
                        {
                            self.inspector.selected_node = None;
                        }
                        self.inspector.hierarchy = Some(hierarchy);
                        self.inspector.hierarchy_error = None;
                    }
                    Err(error) => self.inspector.hierarchy_error = Some(error),
                }
            }
            AppEvent::CliUpdateStaged(result) => {
                self.update.repair_busy = false;
                match result {
                    Ok(staged) => {
                        self.update.staged = Some(staged.clone());
                        self.update.repair_message = Some(
                            "Update downloaded and verified. Confirm replacement.".to_string(),
                        );
                        if let (Some(target), Some(check)) = (
                            self.cli
                                .resolution
                                .as_ref()
                                .map(|resolution| resolution.path.clone()),
                            self.update.value.as_ref(),
                        ) {
                            self.confirm(ConfirmAction::InstallCli {
                                staged,
                                target,
                                version: check.latest_version.clone(),
                            });
                        }
                    }
                    Err(error) => {
                        self.update.repair_message = Some(format!("Update failed: {error}"));
                        self.toast(format!("Update failed: {error}"), ToastLevel::Error);
                    }
                }
            }
            AppEvent::CliUpdateInstalled(result) => {
                self.update.repair_busy = false;
                match result {
                    Ok(message) => {
                        self.update.repair_message = Some(message.clone());
                        self.toast(message, ToastLevel::Success);
                        self.connect(ctx);
                    }
                    Err(error) => {
                        self.update.repair_message = Some(format!("CLI repair failed: {error}"));
                        self.toast(format!("CLI repair failed: {error}"), ToastLevel::Error);
                    }
                }
            }
            AppEvent::SetupTest(result) => {
                self.setup.test_running = false;
                match result {
                    Ok(session) => {
                        self.setup.test_ok = Some(true);
                        self.setup.test_session = Some(session);
                        self.setup.test_error = None;
                        self.setup
                            .test_log
                            .push("Device test passed; session is ready.".to_string());
                    }
                    Err(error) => {
                        self.setup.test_ok = Some(false);
                        self.setup.test_error = Some(error.clone());
                        self.setup.test_log.push(error);
                    }
                }
            }
            AppEvent::SetupProject(result) => {
                self.setup.create_busy = false;
                match result {
                    Ok(path) => {
                        self.toast("Project adapter created", ToastLevel::Success);
                        self.setup.create_error = None;
                        self.refresh_projects(ctx);
                        self.select_project(Some(PathBuf::from(path)), ctx);
                    }
                    Err(error) => self.setup.create_error = Some(error),
                }
            }
            AppEvent::UiAction { label, result } => {
                self.inspector.action_busy = false;
                match result {
                    Ok(message) => {
                        self.inspector.last_action = Some(format!("{label}: {message}"));
                        self.toast(format!("{label} completed"), ToastLevel::Success);
                        self.request_hierarchy(ctx);
                        self.request_frame(ctx);
                    }
                    Err(error) => {
                        self.inspector.last_action = Some(format!("{label}: {error}"));
                        self.toast(format!("{label} failed: {error}"), ToastLevel::Error);
                    }
                }
            }
            AppEvent::Explanation(result) => {
                self.triage.explanation_loading = false;
                match result {
                    Ok(explanation) => {
                        self.triage.explanation = Some(explanation.clone());
                        self.triage.explanation_error = None;
                        self.refresh_recurrence_history(&explanation.fingerprint, ctx);
                    }
                    Err(error) => {
                        self.triage.explanation_error = Some(error);
                    }
                }
            }
            AppEvent::Context(result) => {
                self.context.loading = false;
                match result {
                    Ok(snapshot) => {
                        self.context.snapshot = Some(snapshot);
                        self.context.error = None;
                        self.context.updated = Some(Instant::now());
                    }
                    Err(error) => {
                        self.context.error = Some(error);
                    }
                }
            }
            AppEvent::Health(result) => {
                self.health_loading = false;
                match result {
                    Ok(health) => {
                        self.health = Some(health);
                        self.health_error = None;
                    }
                    Err(error) => self.health_error = Some(error),
                }
            }
            AppEvent::Wireless(result) => {
                self.wireless_loading = false;
                match result {
                    Ok(value) => {
                        self.wireless_result = Some(value);
                        self.wireless_error = None;
                    }
                    Err(error) => self.wireless_error = Some(error),
                }
            }
            AppEvent::Matrix(result) => {
                self.matrix.loading = false;
                match result {
                    Ok(value) => {
                        self.matrix.result = Some(value);
                        self.matrix.error = None;
                        self.toast("Matrix run finished", ToastLevel::Info);
                        self.refresh_runs(ctx);
                        self.refresh_sessions(ctx);
                        self.refresh_devices(ctx);
                    }
                    Err(error) => self.matrix.error = Some(error),
                }
            }
            AppEvent::RecurrenceHistory(result) => {
                self.triage.history_loading = false;
                match result {
                    Ok(history) => {
                        self.triage.history = Some(history);
                        self.triage.history_error = None;
                    }
                    Err(error) => self.triage.history_error = Some(error),
                }
            }
            AppEvent::BundlePreview { run_id, result } => {
                self.triage.bundle.preview_loading = false;
                if self.triage.run_id.as_deref() != Some(run_id.as_str()) {
                    return;
                }
                match result {
                    Ok(preview) => {
                        self.triage.bundle.preview = Some(preview);
                        self.triage.bundle.preview_error = None;
                    }
                    Err(error) => self.triage.bundle.preview_error = Some(error),
                }
            }
            AppEvent::BundleCreated { run_id, result } => {
                self.triage.bundle.creating = false;
                if self.triage.run_id.as_deref() != Some(run_id.as_str()) {
                    return;
                }
                match result {
                    Ok(result) => {
                        let path = result
                            .get("output")
                            .and_then(|value| value.as_str())
                            .unwrap_or("the bundle output")
                            .to_string();
                        self.triage.bundle.result = Some(result);
                        self.triage.bundle.create_error = None;
                        self.toast(
                            format!("Diagnostic bundle created: {path}"),
                            ToastLevel::Success,
                        );
                    }
                    Err(error) => {
                        self.triage.bundle.create_error = Some(error.clone());
                        self.toast(
                            format!("Bundle creation failed: {error}"),
                            ToastLevel::Error,
                        );
                    }
                }
            }
            AppEvent::RepairPlan(result) => {
                self.setup.repair_loading = false;
                match result {
                    Ok(plan) => {
                        let count = plan.issues.len();
                        self.setup.repair_plan = Some(plan);
                        self.setup.repair_error = None;
                        if count > 0 {
                            self.toast(
                                format!(
                                    "Repair plan ready: {count} issue(s); no changes were made"
                                ),
                                ToastLevel::Info,
                            );
                        }
                    }
                    Err(error) => self.setup.repair_error = Some(error),
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

    fn merge_logcat(&mut self, peek: LogcatPeek) {
        let mut incoming = peek.lines;
        // The peek command starts at the previous tail timestamp, so the last
        // line from the previous batch may be repeated. Drop through that
        // boundary line, but keep every genuinely new line (including repeats
        // of identical log messages that occurred after the boundary).
        if let Some(last) = self.logcat.lines.back().map(|entry| entry.raw.clone())
            && let Some(position) = incoming.iter().rposition(|raw| raw.trim_end() == last)
        {
            incoming.drain(0..=position);
        }
        for raw in incoming {
            if let Some(entry) = parse_log_entry(&raw) {
                self.logcat.lines.push_back(entry);
            }
        }
        while self.logcat.lines.len() > self.logcat.max_lines {
            self.logcat.lines.pop_front();
        }
        self.logcat.notes = peek.notes;
        if let Some(retry) = &self.logcat.retry
            && retry["retries_used"].as_u64().unwrap_or(0) > 0
        {
            self.logcat.notes.push(format!(
                "logcat read required {} retry attempt(s)",
                retry["retries_used"].as_u64().unwrap_or(0)
            ));
        }
        if peek.truncated {
            self.logcat
                .notes
                .push("the logcat window was truncated".to_string());
        }
    }
}

fn parse_log_entry(raw: &str) -> Option<LogcatEntry> {
    let raw = raw.trim_end().to_string();
    if raw.is_empty() || raw.starts_with("--------- beginning") {
        return None;
    }
    let timestamp = raw.get(0..18).unwrap_or("").trim().to_string();
    let mut index = 0;
    let bytes = raw.as_bytes();
    let mut fields = 0;
    while index < bytes.len() && fields < 4 {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        fields += 1;
    }
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    let rest = raw.get(index..).unwrap_or("");
    let (priority_tag, message) = rest.split_once(": ").unwrap_or((rest, ""));
    let level = priority_tag
        .chars()
        .next()
        .map(|ch| ch.to_string())
        .unwrap_or_else(|| "?".to_string());
    let tag = priority_tag
        .split_once('/')
        .map(|(_, tag)| tag.to_string())
        .unwrap_or_else(|| priority_tag.to_string());
    let crash = tag.contains("AndroidRuntime")
        || message.contains("FATAL EXCEPTION")
        || message.contains("ANR in ");
    Some(LogcatEntry {
        timestamp,
        level,
        tag,
        message: message.to_string(),
        raw,
        crash,
    })
}

#[allow(dead_code)]
fn _keep_update_import(_check: &update::UpdateCheck) {}
