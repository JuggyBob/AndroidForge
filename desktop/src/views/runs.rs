use eframe::egui;
use egui::RichText;

use crate::app::App;
use crate::components::*;
use crate::state::*;
use crate::theme;
use crate::views::{format_duration, run_details};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    if app.run_detail.is_some() {
        run_details::show(app, ui);
        return;
    }
    page_header(
        ui,
        "Runs",
        "Reports are the source of truth. A process exit code is never treated as a passed run.",
    );
    project_strip(app, ui);
    toolbar(ui, |ui| {
        if secondary_button(ui, "Refresh").clicked() {
            app.manual_refresh(ui.ctx());
        }
        if secondary_button(ui, "Install APK").clicked() {
            app.install.open = true;
        }
        let can_run = app.selected_project().is_some()
            && app.selected_session.is_some()
            && !app.active_operation();
        if ui
            .add_enabled_ui(can_run, |ui| secondary_button(ui, "Run project flow"))
            .inner
            .on_hover_text("Fixture or conformance flow for the selected project and session")
            .clicked()
        {
            app.run_form = Some(RunForm::default());
        }
        if secondary_button(ui, "Triage latest failure").clicked() {
            app.open_triage(None, ui.ctx());
        }
        if secondary_button(ui, "Compare runs").clicked() {
            app.set_page(Page::Compare);
        }
    });
    stale_note(ui, &app.runs, "Run list");
    ui.add_space(4.0);
    if app.runs.value.runs.is_empty() {
        if app.runs.loading {
            ui.horizontal(|ui| {
                ui.spinner();
                muted(ui, "Loading runs...");
            });
        } else {
            empty_state(
                ui,
                "No runs found",
                "Run a fixture, conformance flow or external command to create a report.",
            );
        }
        return;
    }
    let runs = app.runs.value.runs.clone();
    let ctx = ui.ctx().clone();
    let mut open = None;
    let mut triage = None;
    let width = ui.available_width();
    let widths = proportional_widths(
        width,
        &[0.34, 0.07, 0.11, 0.06, 0.08, 0.16, 0.09, 0.09],
        10.0,
    );
    let run_width = widths[0];
    let status_width = widths[1];
    let scenario_width = widths[2];
    let steps_width = widths[3];
    let duration_width = widths[4];
    let started_width = widths[5];
    let open_width = widths[6];
    let triage_width = widths[7];
    table_header(
        ui,
        &[
            ("Run", run_width),
            ("Status", status_width),
            ("Scenario", scenario_width),
            ("Steps", steps_width),
            ("Duration", duration_width),
            ("Started", started_width),
            ("", open_width),
            ("", triage_width),
        ],
    );
    for run in &runs {
        let selected = app.selected_run.as_deref() == Some(run.run_id.as_str());
        let response = selectable_row(ui, selected, theme::ROW_HEIGHT, |ui| {
            row_cell(ui, run_width, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(&run.run_id).size(10.0).monospace()).truncate(),
                )
                .on_hover_text(&run.run_id);
            });
            row_cell(ui, status_width, |ui| {
                status_pill(ui, &run.status, theme::status_color(&run.status));
            });
            row_cell(ui, scenario_width, |ui| {
                ui.add(egui::Label::new(RichText::new(&run.command).size(10.0)).truncate());
            });
            row_cell(ui, steps_width, |ui| {
                ui.label(RichText::new(format!("{}/{}", run.failed_steps, run.steps)).size(10.0));
            });
            row_cell(ui, duration_width, |ui| {
                ui.label(RichText::new(format_duration(run.duration_ms)).size(10.0));
            });
            row_cell(ui, started_width, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(&run.started_at).size(9.0).color(theme::MUTED))
                        .truncate(),
                );
            });
            row_cell(ui, open_width, |ui| {
                if secondary_button(ui, "Open").clicked() {
                    open = Some(run.run_id.clone());
                }
            });
            row_cell(ui, triage_width, |ui| {
                if matches!(run.status.as_str(), "failed" | "partial" | "cancelled") {
                    if secondary_button(ui, "Triage").clicked() {
                        triage = Some(run.run_id.clone());
                    }
                }
            });
        });
        if response.clicked() {
            open = Some(run.run_id.clone());
        }
        ui.add_space(2.0);
    }
    if let Some(run_id) = open {
        app.open_run(run_id, &ctx);
    }
    if let Some(run_id) = triage {
        app.open_triage(Some(run_id), &ctx);
    }

    if let Some(bundle) = &app.project
        && !bundle.hooks.is_empty()
    {
        ui.add_space(theme::PAGE_GAP);
        section_label(ui, "Project hooks");
        let hooks = bundle.hooks.clone();
        for hook in hooks {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&hook.name).size(10.5).monospace());
                ui.add(
                    egui::Label::new(
                        RichText::new(hook.description.as_deref().unwrap_or(""))
                            .size(9.5)
                            .color(theme::MUTED),
                    )
                    .truncate(),
                );
                if action_button(
                    ui,
                    "Run...",
                    !app.active_operation() && app.selected_session.is_some(),
                    "Run this hook on the selected session",
                )
                .clicked()
                {
                    app.confirm(ConfirmAction::Hook {
                        name: hook.name.clone(),
                    });
                }
            });
        }
    }
}
