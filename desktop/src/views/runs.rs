use eframe::egui;
use egui::RichText;
use egui_extras::{Column, TableBuilder};

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
            .add_enabled(can_run, egui::Button::new("Run project flow"))
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
        empty_state(
            ui,
            "No runs found",
            "Run a fixture, conformance flow or external command to create a report.",
        );
        return;
    }
    let runs = app.runs.value.runs.clone();
    let ctx = ui.ctx().clone();
    let mut open = None;
    let mut triage = None;
    TableBuilder::new(ui)
        .sense(egui::Sense::click())
        .striped(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(350.0).at_least(230.0))
        .column(Column::initial(85.0))
        .column(Column::initial(115.0))
        .column(Column::initial(60.0))
        .column(Column::initial(80.0))
        .column(Column::initial(145.0))
        .column(Column::remainder())
        .column(Column::initial(120.0))
        .header(24.0, |mut header| {
            for name in [
                "Run", "Status", "Scenario", "Steps", "Duration", "Started", "", "",
            ] {
                header.col(|ui| {
                    ui.label(RichText::new(name).size(9.0).strong().color(theme::MUTED));
                });
            }
        })
        .body(|body| {
            body.rows(30.0, runs.len(), |mut row| {
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
                    ui.add(egui::Label::new(RichText::new(&run.command).size(10.0)).truncate());
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(format!("{}/{}", run.failed_steps, run.steps)).size(10.0),
                    );
                });
                row.col(|ui| {
                    ui.label(RichText::new(format_duration(run.duration_ms)).size(10.0));
                });
                row.col(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&run.started_at).size(9.0).color(theme::MUTED),
                        )
                        .truncate(),
                    );
                });
                row.col(|_ui| {});
                row.col(|ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button("Open").clicked() {
                            open = Some(run.run_id.clone());
                        }
                        if matches!(run.status.as_str(), "failed" | "partial" | "cancelled")
                            && ui.small_button("Triage").clicked()
                        {
                            triage = Some(run.run_id.clone());
                        }
                    });
                });
                if row.response().clicked() {
                    open = Some(run.run_id.clone());
                }
            });
        });
    if let Some(run_id) = open {
        app.open_run(run_id, &ctx);
    }
    if let Some(run_id) = triage {
        app.open_triage(Some(run_id), &ctx);
    }

    if let Some(bundle) = &app.project
        && !bundle.hooks.is_empty()
    {
        ui.add_space(12.0);
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
