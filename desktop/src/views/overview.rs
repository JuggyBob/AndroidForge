use eframe::egui;
use egui::{Color32, RichText};
use egui_extras::{Column, TableBuilder};

use crate::app::App;
use crate::components::{empty_state, muted, project_strip, section_title, title_row};
use crate::state::Page;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    title_row(
        ui,
        "Overview",
        "A compact view of the AndroidForge environment and recent activity.",
    );
    project_strip(app, ui);
    ui.add_space(8.0);
    if let Some(error) = &app.cli.error {
        crate::components::error_banner(ui, error);
        ui.add_space(6.0);
    }
    for (label, error) in [
        ("Devices", app.devices.error.as_deref()),
        ("Sessions", app.sessions.error.as_deref()),
        ("Runs", app.runs.error.as_deref()),
    ] {
        if let Some(error) = error {
            crate::components::error_banner(ui, &format!("{label}: {error}"));
            ui.add_space(4.0);
        }
    }

    ui.horizontal_wrapped(|ui| {
        let doctor_status = if app.doctor.value.status.is_empty() {
            "unknown"
        } else {
            app.doctor.value.status.as_str()
        };
        stat(
            ui,
            "Environment",
            doctor_status,
            theme::status_color(doctor_status),
            "doctor result / checks",
        );
        stat(
            ui,
            "Devices",
            &app.devices.value.devices.len().to_string(),
            theme::BLUE,
            "connected Android devices",
        );
        let active_sessions = app
            .sessions
            .value
            .sessions
            .iter()
            .filter(|session| session.state != "released" && session.state != "abandoned")
            .count();
        stat(
            ui,
            "Sessions",
            &active_sessions.to_string(),
            theme::GREEN,
            "non-tombstone sessions",
        );
        let failed = app
            .runs
            .value
            .runs
            .iter()
            .filter(|run| run.status == "failed" || run.status == "partial")
            .count();
        stat(
            ui,
            "Runs",
            &app.runs.value.runs.len().to_string(),
            if failed > 0 { theme::RED } else { theme::GREEN },
            "recent runs in the current project",
        );
    });

    ui.add_space(10.0);
    ui.columns(2, |columns| {
        environment(&mut columns[0], app);
        active_sessions(&mut columns[1], app);
    });

    ui.add_space(10.0);
    section_title(ui, "Recent runs");
    if app.runs.value.runs.is_empty() {
        empty_state(
            ui,
            "No runs yet",
            "Start a fixture, conformance flow or external run to populate this list.",
        );
    } else {
        runs_table(app, ui);
    }
}

fn stat(ui: &mut egui::Ui, label: &str, value: &str, color: Color32, detail: &str) {
    let dark = ui.style().visuals.dark_mode;
    egui::Frame::new()
        .fill(if dark {
            theme::DARK_SURFACE
        } else {
            theme::LIGHT_SURFACE
        })
        .stroke(egui::Stroke::new(
            1.0,
            if dark {
                theme::DARK_BORDER
            } else {
                theme::LIGHT_BORDER
            },
        ))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_min_width(150.0);
            ui.label(RichText::new(label).size(10.0).color(theme::MUTED));
            ui.label(RichText::new(value).size(18.0).strong().color(color));
            ui.label(RichText::new(detail).size(9.0).color(theme::MUTED));
        });
}

fn environment(ui: &mut egui::Ui, app: &App) {
    section_title(ui, "Environment health");
    let warnings = app
        .doctor
        .value
        .checks
        .iter()
        .filter(|check| check.status != "ok")
        .collect::<Vec<_>>();
    if warnings.is_empty() {
        muted(ui, "All reported checks are healthy.");
    } else {
        for check in warnings.iter().take(6) {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("● {}", check.status))
                        .size(10.0)
                        .color(theme::status_color(&check.status)),
                );
                ui.label(RichText::new(&check.name).size(11.0).strong());
                ui.label(RichText::new(&check.detail).size(10.0).color(theme::MUTED));
            });
        }
        if warnings.len() > 6 {
            muted(ui, format!("{} more warning(s)", warnings.len() - 6));
        }
    }
    ui.add_space(4.0);
    if let Some(resolution) = &app.cli.resolution {
        ui.horizontal(|ui| {
            muted(ui, "CLI:");
            ui.label(
                RichText::new(resolution.path.display().to_string())
                    .size(10.0)
                    .monospace(),
            );
            crate::components::copy_button(ui, &resolution.path.display().to_string(), "copy");
        });
    }
    if let Some(age) = app.doctor.age() {
        muted(ui, format!("updated {}s ago", age.as_secs()));
    }
}

fn active_sessions(ui: &mut egui::Ui, app: &App) {
    section_title(ui, "Sessions");
    if app.sessions.value.sessions.is_empty() {
        muted(ui, "No sessions are currently recorded.");
        return;
    }
    for session in app.sessions.value.sessions.iter().take(6) {
        ui.horizontal(|ui| {
            let color = theme::status_color(&session.state);
            ui.label(RichText::new("●").size(10.0).color(color));
            ui.label(
                RichText::new(session.serial.as_deref().unwrap_or("-"))
                    .size(11.0)
                    .strong(),
            );
            ui.label(
                RichText::new(format!("{} · {}", session.owner, session.state))
                    .size(10.0)
                    .color(theme::MUTED),
            );
            if session.stale {
                ui.label(RichText::new("stale").size(10.0).color(theme::RED));
            }
        });
    }
    if app.sessions.value.sessions.len() > 6 {
        muted(
            ui,
            format!("{} more session(s)", app.sessions.value.sessions.len() - 6),
        );
    }
}

fn runs_table(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let runs = app.runs.value.runs.clone();
    TableBuilder::new(ui)
        .sense(egui::Sense::click())
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(210.0).at_least(150.0))
        .column(Column::initial(70.0))
        .column(Column::remainder().at_least(90.0))
        .column(Column::initial(70.0))
        .column(Column::initial(90.0))
        .header(22.0, |mut header| {
            for name in ["Run", "Status", "Project", "Steps", "Duration"] {
                header.col(|ui| {
                    ui.label(RichText::new(name).size(10.0).strong().color(theme::MUTED));
                });
            }
        })
        .body(|body| {
            body.rows(24.0, runs.len(), |mut row| {
                let run = &runs[row.index()];
                let selected = app.selected_run.as_deref() == Some(run.run_id.as_str());
                row.set_selected(selected);
                row.col(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&run.run_id).size(10.0).monospace())
                            .truncate(),
                    )
                    .on_hover_text(&run.run_id);
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(&run.status)
                            .size(10.0)
                            .color(theme::status_color(&run.status)),
                    );
                });
                row.col(|ui| {
                    ui.label(RichText::new(&run.project).size(11.0));
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(format!("{}/{}", run.failed_steps, run.steps)).size(10.0),
                    );
                });
                row.col(|ui| {
                    ui.label(RichText::new(format_duration(run.duration_ms)).size(10.0));
                });
                if row.response().clicked() {
                    app.open_run(run.run_id.clone(), &ctx);
                    app.set_page(Page::Runs);
                }
            });
        });
}

pub fn format_duration(milliseconds: u64) -> String {
    if milliseconds < 1000 {
        format!("{milliseconds}ms")
    } else if milliseconds < 60_000 {
        format!("{:.1}s", milliseconds as f64 / 1000.0)
    } else {
        format!(
            "{}m {}s",
            milliseconds / 60_000,
            (milliseconds % 60_000) / 1000
        )
    }
}
