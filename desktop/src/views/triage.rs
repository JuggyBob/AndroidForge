use eframe::egui;
use egui::RichText;
use egui_extras::{Column, TableBuilder};

use crate::app::App;
use crate::components::*;
use crate::state::*;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    if app.run_detail.is_none() && app.triage.run_id.is_none() {
        app.open_triage(None, ui.ctx());
    }
    page_header(
        ui,
        "Triage",
        "The failure, the evidence and the signals that help decide whether it is reproducible.",
    );
    if app.run_detail.is_none() {
        empty_state(
            ui,
            "No run selected for triage",
            "Open a failed run from Runs or select the latest failure.",
        );
        if primary_button(ui, "Triage latest failure").clicked() {
            app.open_triage(None, ui.ctx());
        }
        return;
    }
    let report = app
        .run_detail
        .as_ref()
        .map(|state| state.detail.report.clone())
        .unwrap();
    let path = app
        .run_detail
        .as_ref()
        .map(|state| state.detail.path.path.clone())
        .unwrap();
    ui.horizontal(|ui| {
        ui.label(RichText::new(&report.run_id).size(17.0).strong());
        status_pill(ui, &report.status, theme::status_color(&report.status));
        if ghost_button(ui, "Back to runs").clicked() {
            app.set_page(Page::Runs);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if secondary_button(ui, "Open full run").clicked() {
                app.set_page(Page::Runs);
            }
            if ghost_button(ui, "Open folder").clicked() && !path.is_empty() {
                let _ = open::that(&path);
            }
            if ghost_button(ui, "Copy JSON").clicked()
                && let Ok(text) = serde_json::to_string_pretty(&report)
            {
                ui.ctx().copy_text(text);
            }
        });
    });
    ui.label(
        RichText::new(format!(
            "{} · {} · {} · {}",
            report.project,
            report.command,
            report.started_at,
            crate::views::format_duration(report.duration_ms)
        ))
        .size(10.0)
        .color(theme::MUTED),
    );
    ui.add_space(8.0);
    ui.columns(2, |columns| {
        failure_panel(app, &mut columns[0], &report, &path);
        signals_panel(app, &mut columns[1], &report);
    });
    ui.add_space(12.0);
    artifacts_panel(app, ui, &report, &path);
    ui.add_space(12.0);
    if let Some(flaky) = app.triage.flaky.clone() {
        flaky_panel(ui, &flaky);
    } else if app.triage.flaky_loading {
        ui.horizontal(|ui| {
            ui.spinner();
            muted(ui, "Analyzing recent runs for flaky steps...");
        });
    } else if let Some(error) = &app.triage.flaky_error {
        error_banner(ui, error);
    }
}

fn failure_panel(app: &mut App, ui: &mut egui::Ui, report: &crate::models::RunReport, path: &str) {
    card(ui, |ui| {
        section_title(ui, "Failure");
        if let Some(failure) = &report.failure {
            error_banner(ui, &format!("{}: {}", failure.code, failure.message));
            ui.add_space(4.0);
            kv_grid(
                ui,
                "triage-failure",
                &[
                    (
                        "Operation",
                        failure
                            .failed_operation
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Recoverable",
                        if failure.recoverable { "yes" } else { "no" }.to_string(),
                    ),
                    (
                        "Device",
                        report
                            .device_serial
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Package",
                        report
                            .build
                            .package
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            );
            for suggestion in &failure.suggestions {
                ui.label(
                    RichText::new(format!("→ {suggestion}"))
                        .size(10.0)
                        .color(theme::BLUE),
                );
            }
        } else {
            warning_banner(ui, "This run did not record a failure object.");
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if primary_button(ui, "Compare with baseline").clicked() {
                app.compare_form.current = Some(report.run_id.clone());
                app.set_page(Page::Compare);
            }
            if secondary_button(ui, "Open folder").clicked() && !path.is_empty() {
                let _ = open::that(path);
            }
        });
    });
}

fn signals_panel(app: &mut App, ui: &mut egui::Ui, report: &crate::models::RunReport) {
    card(ui, |ui| {
        section_title(ui, "Signals");
        let failure_count = report
            .steps
            .iter()
            .filter(|step| step.status == "failed")
            .count();
        kv_grid(
            ui,
            "triage-signals",
            &[
                (
                    "Steps",
                    format!("{} total · {failure_count} failed", report.steps.len()),
                ),
                ("Artifacts", format!("{} captured", report.artifacts.len())),
                (
                    "Build",
                    format!(
                        "{} {}",
                        report.build.package.as_deref().unwrap_or("-"),
                        report.build.version_name.as_deref().unwrap_or("-")
                    ),
                ),
            ],
        );
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if secondary_button(ui, "Open logcat").clicked()
                && let Some(session) = report.session_id.clone()
            {
                app.logcat_select_session(Some(session));
                app.set_page(Page::Logcat);
            }
            if secondary_button(ui, "Open inspector").clicked()
                && let Some(session) = report.session_id.clone()
            {
                app.inspector_select_session(Some(session));
                app.set_page(Page::Inspector);
                app.start_inspector(ui.ctx());
            }
        });
        ui.add_space(6.0);
        small(
            ui,
            "Flaky analysis below uses recent compatible terminal runs; it never labels a single failure as flaky.",
        );
    });
}

fn artifacts_panel(
    app: &mut App,
    ui: &mut egui::Ui,
    report: &crate::models::RunReport,
    path: &str,
) {
    section_label(ui, "Evidence");
    if report.artifacts.is_empty() {
        muted(ui, "No artifacts were recorded.");
        return;
    }
    let mut filter = app
        .run_detail
        .as_ref()
        .map(|state| state.evidence.filter.clone())
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Filter").size(10.0).color(theme::MUTED));
        ui.add(
            egui::TextEdit::singleline(&mut filter)
                .desired_width(240.0)
                .hint_text("screenshot, hierarchy, logcat..."),
        );
    });
    if let Some(state) = app.run_detail.as_mut() {
        state.evidence.filter = filter.clone();
    }
    let artifacts = report.artifacts.clone();
    let mut open_path = None;
    ui.horizontal_wrapped(|ui| {
        for artifact in &artifacts {
            let haystack = format!(
                "{} {} {}",
                artifact.kind,
                artifact.path,
                artifact.description.as_deref().unwrap_or("")
            )
            .to_ascii_lowercase();
            if !filter.is_empty() && !haystack.contains(&filter.to_ascii_lowercase()) {
                continue;
            }
            let label = format!(
                "{} {}",
                icon_for_kind(&artifact.kind),
                artifact.path.rsplit('/').next().unwrap_or(&artifact.path)
            );
            if ui
                .add(
                    egui::Button::new(RichText::new(label).size(9.5)).fill(theme::surface_fill(ui)),
                )
                .on_hover_text(&artifact.path)
                .clicked()
            {
                open_path = Some(format!("{}/{}", path.replace('\\', "/"), artifact.path));
            }
        }
    });
    if let Some(path) = open_path {
        let _ = open::that(path);
    }
}

fn flaky_panel(ui: &mut egui::Ui, flaky: &crate::models::FlakyReport) {
    section_title(ui, "Flaky-step analysis");
    muted(ui, &flaky.heuristic);
    ui.add_space(4.0);
    let interesting = flaky
        .steps
        .iter()
        .filter(|step| matches!(step.classification.as_str(), "flaky" | "stable_failure"))
        .take(8)
        .collect::<Vec<_>>();
    if interesting.is_empty() {
        muted(
            ui,
            "No flaky or consistently failing steps in the analyzed window.",
        );
        return;
    }
    let steps = interesting
        .iter()
        .map(|step| (*step).clone())
        .collect::<Vec<_>>();
    TableBuilder::new(ui)
        .striped(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::remainder().at_least(200.0))
        .column(Column::initial(90.0))
        .column(Column::initial(80.0))
        .column(Column::initial(100.0))
        .header(24.0, |mut header| {
            for name in ["Step", "Classification", "Pass / fail", "Runs"] {
                header.col(|ui| {
                    ui.label(RichText::new(name).size(9.0).strong().color(theme::MUTED));
                });
            }
        })
        .body(|body| {
            body.rows(28.0, steps.len(), |mut row| {
                let step = &steps[row.index()];
                row.col(|ui| {
                    ui.label(RichText::new(&step.step_name).size(10.0));
                });
                row.col(|ui| {
                    status_pill(
                        ui,
                        &step.classification,
                        theme::status_color(&step.classification),
                    );
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(format!("{} / {}", step.successes, step.failures)).size(10.0),
                    );
                });
                row.col(|ui| {
                    ui.label(RichText::new(step.executions.to_string()).size(10.0));
                });
            });
        });
}

fn icon_for_kind(kind: &str) -> &'static str {
    if kind.contains("screenshot") {
        "▣"
    } else if kind.contains("log") {
        "≡"
    } else if kind.contains("hierarchy") {
        "⌘"
    } else {
        "·"
    }
}
