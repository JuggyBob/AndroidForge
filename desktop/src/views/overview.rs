use eframe::egui;
use egui::{RichText, Vec2};
use egui_extras::{Column, TableBuilder};

use crate::app::App;
use crate::components::*;
use crate::state::*;
use crate::theme;
use crate::views::format_duration;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    page_header(
        ui,
        "Overview",
        "Your Android workspace at a glance: environment, ownership and recent evidence.",
    );
    project_strip(app, ui);
    if let Some(error) = &app.cli.error {
        error_banner(ui, error);
        ui.add_space(6.0);
    }

    let failed = app
        .runs
        .value
        .runs
        .iter()
        .filter(|run| matches!(run.status.as_str(), "failed" | "partial"))
        .count();
    let active_sessions = app
        .sessions
        .value
        .sessions
        .iter()
        .filter(|session| !matches!(session.state.as_str(), "released" | "abandoned" | "failed"))
        .count();
    let stale_sessions = app
        .sessions
        .value
        .sessions
        .iter()
        .filter(|session| session.stale)
        .count();
    let environment = if app.doctor.value.status.is_empty() {
        "unknown"
    } else {
        app.doctor.value.status.as_str()
    };
    ui.columns(4, |columns| {
        metric(
            &mut columns[0],
            "Environment",
            &theme::status_label(environment),
            theme::status_color(environment),
            "doctor status",
        );
        metric(
            &mut columns[1],
            "Devices",
            &app.devices.value.devices.len().to_string(),
            theme::BLUE,
            "connected",
        );
        metric(
            &mut columns[2],
            "Sessions",
            &active_sessions.to_string(),
            if stale_sessions > 0 {
                theme::YELLOW
            } else {
                theme::GREEN
            },
            if stale_sessions > 0 {
                "stale ownership"
            } else {
                "active"
            },
        );
        metric(
            &mut columns[3],
            "Runs",
            &app.runs.value.runs.len().to_string(),
            if failed > 0 { theme::RED } else { theme::GREEN },
            if failed > 0 {
                "failures to triage"
            } else {
                "recent history"
            },
        );
    });

    ui.add_space(12.0);
    ui.columns(2, |columns| {
        environment_panel(app, &mut columns[0]);
        actions_panel(app, &mut columns[1], failed);
    });

    ui.add_space(12.0);
    section_label(ui, "Recent runs");
    if app.runs.value.runs.is_empty() {
        empty_state(
            ui,
            "No runs yet",
            "Run a fixture, conformance flow or external command to build a report.",
        );
    } else {
        recent_runs(app, ui);
    }
}

fn environment_panel(app: &mut App, ui: &mut egui::Ui) {
    card_min_height(ui, 238.0, |ui| {
        ui.horizontal(|ui| {
            section_title(ui, "Workspace health");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if secondary_button(ui, "Open setup").clicked() {
                    app.open_setup(SetupSection::Environment);
                }
            });
        });
        let blocking = app
            .doctor
            .value
            .checks
            .iter()
            .filter(|check| check.blocking)
            .collect::<Vec<_>>();
        let warnings = app
            .doctor
            .value
            .checks
            .iter()
            .filter(|check| check.status != "ok")
            .collect::<Vec<_>>();
        if app.doctor.value.checks.is_empty() {
            muted(
                ui,
                "Run the environment check to see SDK, ADB and device readiness.",
            );
        } else if warnings.is_empty() {
            success_banner(ui, "All reported checks are healthy.");
        } else {
            for check in warnings.iter().take(5) {
                ui.horizontal(|ui| {
                    status_pill(ui, &check.status, theme::status_color(&check.status));
                    ui.label(RichText::new(&check.name).size(11.0).strong());
                    ui.add(
                        egui::Label::new(
                            RichText::new(&check.detail).size(10.0).color(theme::MUTED),
                        )
                        .truncate(),
                    )
                    .on_hover_text(
                        check
                            .impact
                            .clone()
                            .unwrap_or_else(|| "No additional impact reported.".to_string()),
                    );
                });
            }
            if warnings.len() > 5 {
                muted(
                    ui,
                    format!("{} more check(s) need attention", warnings.len() - 5),
                );
            }
            if !blocking.is_empty() {
                ui.add_space(4.0);
                warning_banner(
                    ui,
                    &format!("{} blocking requirement(s) are not ready.", blocking.len()),
                );
            }
        }
        ui.add_space(4.0);
        if let Some(resolution) = &app.cli.resolution {
            path_row(ui, &resolution.path.display().to_string());
        }
        if let Some(age) = app.doctor.age() {
            small(ui, format!("updated {}s ago", age.as_secs()));
        }
    });
}

fn actions_panel(app: &mut App, ui: &mut egui::Ui, failed: usize) {
    card_min_height(ui, 238.0, |ui| {
        section_title(ui, "Next actions");
        if failed > 0 {
            warning_banner(
                ui,
                &format!("{failed} run(s) failed or partially completed."),
            );
            ui.add_space(4.0);
            if primary_button(ui, "Triage latest failure").clicked() {
                app.open_triage(None, ui.ctx());
            }
            ui.add_space(4.0);
        }
        ui.horizontal_wrapped(|ui| {
            if secondary_button(ui, "Refresh").clicked() {
                app.manual_refresh(ui.ctx());
            }
            if secondary_button(ui, "Start session").clicked() {
                app.session_form = Some(SessionForm {
                    owner: app.settings.owner.clone(),
                    headless: true,
                    ..Default::default()
                });
            }
            if secondary_button(ui, "Run project flow").clicked() {
                app.run_form = Some(RunForm::default());
            }
            if secondary_button(ui, "Open runs").clicked() {
                app.set_page(Page::Runs);
            }
        });
        ui.add_space(6.0);
        section_label(ui, "Current target");
        if let Some(session) = app.selected_session_info() {
            kv_grid(
                ui,
                "overview-target",
                &[
                    ("Session", session.id),
                    ("Owner", session.owner),
                    ("Device", session.serial.unwrap_or_else(|| "-".to_string())),
                    ("State", theme::status_label(&session.state)),
                ],
            );
        } else if let Some(device) = app.devices.value.devices.first() {
            kv_grid(
                ui,
                "overview-device",
                &[
                    ("Device", device.serial.clone()),
                    (
                        "Model",
                        device.model.clone().unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Android",
                        device
                            .android_version
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            );
        } else {
            muted(
                ui,
                "No device or session is selected. Start one from Setup.",
            );
        }
    });
}

fn recent_runs(app: &mut App, ui: &mut egui::Ui) {
    let runs = app.runs.value.runs.clone();
    let ctx = ui.ctx().clone();
    let mut open = None;
    TableBuilder::new(ui)
        .sense(egui::Sense::click())
        .striped(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(380.0).at_least(220.0))
        .column(Column::initial(80.0))
        .column(Column::initial(150.0))
        .column(Column::initial(80.0))
        .column(Column::remainder())
        .column(Column::initial(60.0))
        .header(24.0, |mut header| {
            for name in ["Run", "Status", "Scenario", "Duration", "", ""] {
                header.col(|ui| {
                    ui.label(RichText::new(name).size(9.0).strong().color(theme::MUTED));
                });
            }
        })
        .body(|body| {
            body.rows(28.0, runs.len().min(8), |mut row| {
                let run = &runs[row.index()];
                row.set_selected(app.selected_run.as_deref() == Some(run.run_id.as_str()));
                row.col(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&run.run_id).size(10.0).monospace())
                            .truncate(),
                    )
                    .on_hover_text(&run.run_id);
                });
                row.col(|ui| {
                    status_pill(ui, &run.status, theme::status_color(&run.status));
                });
                row.col(|ui| {
                    ui.label(RichText::new(&run.command).size(10.0).color(theme::MUTED));
                });
                row.col(|ui| {
                    ui.label(RichText::new(format_duration(run.duration_ms)).size(10.0));
                });
                row.col(|_ui| {});
                row.col(|ui| {
                    if ui.small_button("Open").clicked() {
                        open = Some(run.run_id.clone());
                    }
                });
                if row.response().clicked() {
                    open = Some(run.run_id.clone());
                }
            });
        });
    if let Some(run_id) = open {
        app.open_run(run_id, &ctx);
        app.set_page(Page::Runs);
    }
}

#[allow(dead_code)]
fn _keep_vec(_: Vec2) {}
