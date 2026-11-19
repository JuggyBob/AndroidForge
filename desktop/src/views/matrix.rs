use eframe::egui;
use egui::RichText;
use serde_json::Value;

use crate::app::App;
use crate::components::*;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    page_header(
        ui,
        "Matrix",
        "Run one conformance flow against multiple independent AVDs, sequentially, with one owned session per target.",
    );
    card(ui, |ui| {
        section_title(ui, "Run a sequential matrix");
        ui.label(
            RichText::new("AVDs (one per line or comma-separated)")
                .size(9.5)
                .color(theme::MUTED),
        );
        ui.add(
            egui::TextEdit::multiline(&mut app.matrix.avds)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text("androidforge_test\naster_test"),
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new("Flow").size(9.5).color(theme::MUTED));
            ui.add(
                egui::TextEdit::singleline(&mut app.matrix.flow)
                    .desired_width(220.0)
                    .hint_text("conformance"),
            );
            ui.label(RichText::new("Owner").size(9.5).color(theme::MUTED));
            ui.add(
                egui::TextEdit::singleline(&mut app.matrix.owner)
                    .desired_width(160.0)
                    .hint_text("desktop-matrix"),
            );
            ui.checkbox(&mut app.matrix.install, "Install APK when required");
            ui.checkbox(&mut app.matrix.fail_fast, "Fail fast");
        });
        ui.horizontal(|ui| {
            if primary_button(ui, "Run matrix").clicked() && !app.matrix.loading {
                app.start_matrix(ui.ctx());
            }
            if app.matrix.loading {
                ui.spinner();
                muted(
                    ui,
                    "Running targets sequentially; this can take several minutes...",
                );
            }
            if secondary_button(ui, "Refresh runs").clicked() {
                app.manual_refresh(ui.ctx());
            }
        });
        if let Some(error) = &app.matrix.error {
            error_banner(ui, error);
        }
        small(
            ui,
            "Every target gets its own session. Existing sessions are never stolen or released. Emulators are not stopped automatically. Parallel workers run one independent child matrix per target.",
        );
    });
    let Some(result) = app.matrix.result.clone() else {
        return;
    };
    ui.add_space(theme::PAGE_GAP);
    summary(ui, &result);
    ui.add_space(theme::PAGE_GAP);
    target_table(ui, &result);
}

fn summary(ui: &mut egui::Ui, result: &Value) {
    let status = result["aggregate_status"].as_str().unwrap_or("unknown");
    let color = match status {
        "passed" => theme::GREEN,
        "partial" => theme::YELLOW,
        "failed" => theme::RED,
        _ => theme::MUTED,
    };
    card(ui, |ui| {
        ui.horizontal(|ui| {
            section_title(ui, "Matrix result");
            status_pill(ui, status, color);
            status_pill(
                ui,
                &format!(
                    "{} passed · {} failed · {} skipped",
                    result["passed"].as_u64().unwrap_or(0),
                    result["failed"].as_u64().unwrap_or(0),
                    result["skipped"].as_u64().unwrap_or(0)
                ),
                theme::MUTED,
            );
        });
        kv_grid(
            ui,
            "matrix-summary",
            &[
                (
                    "Matrix run",
                    result["matrix_run_id"].as_str().unwrap_or("-").to_string(),
                ),
                (
                    "Project",
                    result["project"].as_str().unwrap_or("-").to_string(),
                ),
                ("Flow", result["flow"].as_str().unwrap_or("-").to_string()),
                (
                    "Build",
                    result["build"]["version_name"]
                        .as_str()
                        .or(result["build"]["package"].as_str())
                        .unwrap_or("-")
                        .to_string(),
                ),
                (
                    "Duration",
                    result["duration_ms"]
                        .as_u64()
                        .map(crate::views::format_duration)
                        .unwrap_or_else(|| "-".to_string()),
                ),
                (
                    "Cleanup",
                    result["cleanup_policy"]["sessions"]
                        .as_str()
                        .unwrap_or("-")
                        .to_string(),
                ),
            ],
        );
    });
}

fn target_table(ui: &mut egui::Ui, result: &Value) {
    let Some(targets) = result["target_results"].as_array() else {
        return;
    };
    section_label(ui, "Per-device results");
    if targets.is_empty() {
        muted(ui, "No targets were recorded.");
        return;
    }
    for (index, target) in targets.iter().enumerate() {
        let status = target["status"].as_str().unwrap_or("unknown");
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{}. {}",
                        index + 1,
                        target["target"]["name"]
                            .as_str()
                            .or(target["target"]["serial"].as_str())
                            .unwrap_or("target")
                    ))
                    .size(11.0)
                    .strong(),
                );
                status_pill(ui, status, theme::status_color(status));
                if let Some(session) = target["session_id"].as_str() {
                    status_pill(ui, &format!("session {session}"), theme::MUTED);
                }
                if let Some(mode) = target["acquisition_mode"].as_str() {
                    status_pill(ui, mode, theme::BLUE);
                }
                if let Some(api) = target["api_level"].as_u64() {
                    status_pill(ui, &format!("API {api}"), theme::MUTED);
                }
            });
            if let Some(model) = target["model"].as_str() {
                ui.label(RichText::new(model).size(9.5).color(theme::MUTED));
            }
            if let Some(failure) = target["failure_code"].as_str() {
                error_banner(
                    ui,
                    &format!(
                        "{}: {}",
                        failure,
                        target["failure_message"].as_str().unwrap_or("failed")
                    ),
                );
            }
            if target["health"]["battery"]["level_percent"]
                .as_u64()
                .is_some()
            {
                let health = &target["health"];
                ui.label(
                    RichText::new(format!(
                        "Health: battery {}% · thermal {} · {} MiB free · wifi {}",
                        health["battery"]["level_percent"].as_u64().unwrap_or(0),
                        health["thermal"]["status"].as_str().unwrap_or("unknown"),
                        health["storage"]["available_bytes"]
                            .as_u64()
                            .map(|value| value / (1024 * 1024))
                            .unwrap_or(0),
                        health["network"]["wifi_enabled"]
                            .as_bool()
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "unknown".to_string())
                    ))
                    .size(9.5)
                    .color(theme::MUTED),
                );
            }
            for warning in target["health_warnings"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                ui.label(RichText::new(warning).size(9.0).color(theme::YELLOW));
            }
            ui.horizontal(|ui| {
                if let Some(run_id) = target["run_id"].as_str() {
                    copy_button(ui, run_id, "Copy run id");
                }
                if let Some(session) = target["session_id"].as_str() {
                    copy_button(ui, session, "Copy session");
                }
                copy_button(
                    ui,
                    &format!(
                        "androidforge --project {} agent matrix --avd {} --flow {} --json",
                        result["project_root"].as_str().unwrap_or("."),
                        target["target"]["name"].as_str().unwrap_or("<avd>"),
                        result["flow"].as_str().unwrap_or("conformance")
                    ),
                    "Copy rerun command",
                );
            });
            ui.label(
                RichText::new(format!(
                    "cleanup: {} · duration: {}",
                    target["cleanup"].as_str().unwrap_or("-"),
                    target["duration_ms"]
                        .as_u64()
                        .map(crate::views::format_duration)
                        .unwrap_or_else(|| "-".to_string())
                ))
                .size(9.0)
                .color(theme::MUTED),
            );
        });
        ui.add_space(4.0);
    }
}
