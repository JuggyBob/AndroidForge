use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;

use super::{App, run_result};
use crate::models::{FrameResult, LogcatPeek, UiInspectResult, UiNode};
use crate::state::*;

impl App {
    pub fn logcat_select_session(&mut self, session: Option<String>) {
        self.logcat.session_id = session;
        self.logcat.lines.clear();
        self.logcat.error = None;
    }

    pub fn start_logcat(&mut self, ctx: &egui::Context) {
        if self.logcat.session_id.is_none() {
            self.logcat.session_id = self.default_live_session();
        }
        let Some(session) = self.logcat.session_id.clone() else {
            self.logcat.error = Some("Select a ready session before streaming logcat.".to_string());
            return;
        };
        let _ = session;
        self.logcat.running = true;
        self.logcat.paused = false;
        self.logcat.error = None;
        self.logcat.exported = None;
        self.logcat.lines.clear();
        self.request_logcat(ctx);
    }

    pub fn stop_logcat(&mut self) {
        self.logcat.running = false;
        self.logcat.paused = false;
        self.logcat.loading = false;
    }

    pub fn toggle_logcat_pause(&mut self, ctx: &egui::Context) {
        self.logcat.paused = !self.logcat.paused;
        if !self.logcat.paused {
            self.request_logcat(ctx);
        }
    }

    pub fn clear_logcat(&mut self) {
        self.logcat.lines.clear();
        self.logcat.exported = None;
    }

    pub fn export_logcat(&mut self) {
        let path = std::env::temp_dir().join(format!(
            "androidforge-logcat-{}.txt",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        ));
        let text = self
            .logcat
            .lines
            .iter()
            .map(|line| line.raw.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        match std::fs::write(&path, text) {
            Ok(()) => {
                self.logcat.exported = Some(path.display().to_string());
                self.toast("Logcat export written", ToastLevel::Success);
            }
            Err(error) => {
                self.toast(format!("Logcat export failed: {error}"), ToastLevel::Error);
            }
        }
    }

    pub(super) fn poll_logcat(&mut self, ctx: &egui::Context) {
        if !self.logcat.running || self.logcat.paused || self.logcat.loading {
            return;
        }
        if self
            .logcat
            .last_capture
            .is_some_and(|last| last.elapsed() < Duration::from_millis(800))
        {
            return;
        }
        self.request_logcat(ctx);
    }

    fn request_logcat(&mut self, ctx: &egui::Context) {
        let Some(runner) = self.runner.clone() else {
            self.logcat.error = Some("CLI is not connected.".to_string());
            return;
        };
        let Some(session) = self.logcat.session_id.clone() else {
            self.logcat.error = Some("Select a session before streaming logcat.".to_string());
            return;
        };
        let mut args = vec![
            "diagnostics".to_string(),
            "logcat".to_string(),
            "peek".to_string(),
            "--session".to_string(),
            session,
            "--lines".to_string(),
            self.settings.logcat_lines.max(100).to_string(),
            "--level".to_string(),
            self.logcat.level.clone(),
            "--retries".to_string(),
            "2".to_string(),
        ];
        if !self.logcat.package.trim().is_empty() {
            args.push("--package".to_string());
            args.push(self.logcat.package.trim().to_string());
        }
        if let Some(last) = self.logcat.lines.back() {
            args.push("--since".to_string());
            args.push(last.timestamp.clone());
        }
        let invocation = self
            .project_invocation(args)
            .timeout(Duration::from_secs(20));
        self.logcat.loading = true;
        self.logcat.last_capture = Some(Instant::now());
        self.spawn_event(ctx, move || {
            AppEvent::Logcat(run_result::<LogcatPeek>(&runner, invocation))
        });
    }

    pub fn inspector_select_session(&mut self, session: Option<String>) {
        self.inspector.session_id = session;
        self.inspector.hierarchy = None;
        self.inspector.selected_node = None;
        self.inspector.collapsed.clear();
        self.inspector.image = None;
        self.inspector.display_size = None;
        self.inspector.frame_error = None;
        self.inspector.hierarchy_error = None;
        self.inspector.performance = None;
        self.inspector.performance_error = None;
    }

    pub fn start_inspector(&mut self, ctx: &egui::Context) {
        if self.inspector.session_id.is_none() {
            self.inspector.session_id = self.default_live_session();
        }
        self.inspector.auto_refresh = true;
        self.request_frame(ctx);
        self.request_hierarchy(ctx);
    }

    pub fn inspector_pause(&mut self) {
        self.inspector.auto_refresh = !self.inspector.auto_refresh;
    }

    pub fn refresh_inspector(&mut self, ctx: &egui::Context) {
        // Auto-refresh can already have a capture/dump in flight. Starting a
        // second UIAutomator dump concurrently makes the device-side service
        // temporarily unavailable, so coalesce manual refreshes.
        if self.inspector.frame_loading || self.inspector.hierarchy_loading {
            return;
        }
        self.request_frame(ctx);
        self.request_hierarchy(ctx);
    }

    pub fn inspector_measure_performance(&mut self, ctx: &egui::Context) {
        if self.inspector.performance_loading {
            return;
        }
        let Some(runner) = self.runner.clone() else {
            self.inspector.performance_error = Some("CLI is not connected".to_string());
            return;
        };
        let Some(session) = self.inspector.session_id.clone() else {
            self.inspector.performance_error = Some("Select a session first".to_string());
            return;
        };
        let cold = self.inspector.performance_cold;
        let mut args = vec![
            "diagnostics".to_string(),
            "performance".to_string(),
            "--session".to_string(),
            session,
        ];
        if cold {
            args.push("--cold".to_string());
        }
        let invocation = self
            .project_invocation(args)
            .timeout(Duration::from_secs(180));
        self.inspector.performance_loading = true;
        self.inspector.performance_error = None;
        self.spawn_event(ctx, move || {
            AppEvent::Performance(run_result::<crate::models::PerformanceReport>(
                &runner, invocation,
            ))
        });
    }
    pub(super) fn poll_inspector(&mut self, ctx: &egui::Context) {
        if self.page != Page::Inspector || !self.inspector.auto_refresh {
            return;
        }
        if self.inspector.session_id.is_none() {
            self.inspector.session_id = self.default_live_session();
        }
        if self.inspector.session_id.is_none() {
            return;
        }
        let interval = Duration::from_millis(self.inspector.interval_ms.max(500));
        if !self.inspector.frame_loading
            && self
                .inspector
                .last_frame
                .map(|last| last.elapsed() >= interval)
                .unwrap_or(true)
        {
            self.request_frame(ctx);
        }
        if !self.inspector.hierarchy_loading
            && self
                .inspector
                .last_hierarchy
                .map(|last| last.elapsed() >= interval * 3)
                .unwrap_or(true)
        {
            self.request_hierarchy(ctx);
        }
    }

    pub(super) fn request_frame(&mut self, ctx: &egui::Context) {
        let Some(runner) = self.runner.clone() else {
            return;
        };
        let Some(session) = self.inspector.session_id.clone() else {
            self.inspector.frame_error = Some("Select a session to preview a device.".to_string());
            return;
        };
        let path = std::env::temp_dir().join(format!(
            "androidforge-preview-{}-{}.png",
            session,
            chrono::Utc::now().format("%Y%m%d%H%M%S%3f")
        ));
        let invocation = self
            .project_invocation([
                "capture".to_string(),
                "frame".to_string(),
                "--session".to_string(),
                session,
                "--output".to_string(),
                path.display().to_string(),
            ])
            .timeout(Duration::from_secs(30));
        self.inspector.frame_loading = true;
        self.spawn_event(ctx, move || {
            AppEvent::Frame(run_result::<FrameResult>(&runner, invocation))
        });
    }

    pub(super) fn request_hierarchy(&mut self, ctx: &egui::Context) {
        let Some(runner) = self.runner.clone() else {
            return;
        };
        let Some(session) = self.inspector.session_id.clone() else {
            self.inspector.hierarchy_error =
                Some("Select a session to inspect UI hierarchy.".to_string());
            return;
        };
        let invocation = self
            .project_invocation([
                "ui".to_string(),
                "inspect".to_string(),
                "--session".to_string(),
                session,
            ])
            .timeout(Duration::from_secs(30));
        self.inspector.hierarchy_loading = true;
        self.spawn_event(ctx, move || {
            AppEvent::Hierarchy(run_result::<UiInspectResult>(&runner, invocation))
        });
    }

    pub fn selected_node(&self) -> Option<&UiNode> {
        let hierarchy = self.inspector.hierarchy.as_ref()?;
        let id = self.inspector.selected_node?;
        hierarchy.nodes.iter().find(|node| node.id == id)
    }

    pub fn inspector_tap(&mut self, ctx: &egui::Context) {
        let args = self.selected_node().map(selector_args).unwrap_or_default();
        if args.is_empty() {
            self.toast(
                "This node has no semantic selector. Use the coordinate fallback explicitly.",
                ToastLevel::Warning,
            );
            return;
        }
        self.run_ui_action(ctx, "Tap", prepend("tap", args));
    }

    pub fn inspector_long_press(&mut self, ctx: &egui::Context) {
        let args = self.selected_node().map(selector_args).unwrap_or_default();
        if args.is_empty() {
            self.toast("This node has no semantic selector", ToastLevel::Warning);
            return;
        }
        self.run_ui_action(ctx, "Long press", prepend("long-press", args));
    }

    pub fn inspector_coordinate_tap(&mut self, ctx: &egui::Context) {
        let Some(node) = self.selected_node() else {
            return;
        };
        let x = (node.bounds.left + node.bounds.right) / 2;
        let y = (node.bounds.top + node.bounds.bottom) / 2;
        self.run_ui_action(
            ctx,
            "Coordinate tap",
            vec![
                "tap".to_string(),
                "--x".to_string(),
                x.to_string(),
                "--y".to_string(),
                y.to_string(),
            ],
        );
    }

    pub fn inspector_type(&mut self, ctx: &egui::Context) {
        let Some(node) = self.selected_node() else {
            return;
        };
        if !node.editable_like() {
            self.toast(
                "The selected node is not an editable field",
                ToastLevel::Warning,
            );
            return;
        }
        let mut args = vec!["text".to_string(), self.inspector.typed_text.clone()];
        args.extend(selector_args(node));
        args.push("--clear".to_string());
        args.push("--hide-keyboard".to_string());
        self.run_ui_action(ctx, "Type text", args);
        self.inspector.typed_text.clear();
    }

    pub fn inspector_back(&mut self, ctx: &egui::Context) {
        self.run_ui_action(
            ctx,
            "Back",
            vec!["back".to_string(), "--hide-keyboard".to_string()],
        );
    }

    pub fn inspector_home(&mut self, ctx: &egui::Context) {
        self.run_ui_action(ctx, "Home", vec!["home".to_string()]);
    }

    pub fn inspector_scroll(&mut self, ctx: &egui::Context, direction: &str) {
        let mut args = vec![
            "scroll".to_string(),
            "--direction".to_string(),
            direction.to_string(),
        ];
        if let Some(node) = self.selected_node() {
            let selector = selector_args(node);
            args.extend(selector);
        }
        self.run_ui_action(ctx, "Scroll", args);
    }

    fn run_ui_action(&mut self, ctx: &egui::Context, label: &str, args: Vec<String>) {
        if self.inspector.action_busy {
            return;
        }
        let Some(runner) = self.runner.clone() else {
            self.toast("CLI is not connected", ToastLevel::Error);
            return;
        };
        let Some(session) = self.inspector.session_id.clone() else {
            self.toast("Select a session first", ToastLevel::Error);
            return;
        };
        let mut full = vec![
            "ui".to_string(),
            args.first()
                .cloned()
                .unwrap_or_else(|| "inspect".to_string()),
        ];
        full.extend(args.into_iter().skip(1));
        full.push("--session".to_string());
        full.push(session);
        let invocation = self
            .project_invocation(full)
            .timeout(Duration::from_secs(30));
        self.inspector.action_busy = true;
        let label = label.to_string();
        self.spawn_event(ctx, move || {
            let result = runner
                .run_success(&invocation)
                .map(|output| {
                    output
                        .envelope
                        .result
                        .as_ref()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "ok".to_string())
                })
                .map_err(|error| error.to_string());
            AppEvent::UiAction { label, result }
        });
    }

    #[allow(dead_code)]
    fn _frame_keep(_: &FrameResult) {}
}

fn prepend(command: &str, mut args: Vec<String>) -> Vec<String> {
    args.insert(0, command.to_string());
    args
}

pub fn selector_args(node: &UiNode) -> Vec<String> {
    if let Some(resource_id) = &node.resource_id
        && !resource_id.is_empty()
    {
        return vec!["--resource-id".to_string(), resource_id.clone()];
    }
    if let Some(text) = &node.text
        && !text.is_empty()
    {
        return vec!["--text".to_string(), text.clone()];
    }
    if let Some(description) = &node.content_desc
        && !description.is_empty()
    {
        return vec!["--content-desc".to_string(), description.clone()];
    }
    Vec::new()
}

trait EditableNode {
    fn editable_like(&self) -> bool;
}

impl EditableNode for UiNode {
    fn editable_like(&self) -> bool {
        self.class
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase()
            .contains("edittext")
    }
}

#[allow(dead_code)]
fn _keep_path(_path: PathBuf) {}
