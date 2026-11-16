use eframe::egui;
use egui::RichText;
use egui_extras::{Column, TableBuilder};

use crate::app::App;
use crate::components::{
    action_button, empty_state, muted, project_strip, section_title, stale_note, title_row,
};
use crate::state::*;
use crate::theme;
use crate::views::overview::format_duration;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    title_row(
        ui,
        "Runs",
        "Reports are the source of truth. A successful process exit is not treated as a passed run.",
    );
    project_strip(app, ui);
    ui.horizontal(|ui| {
        if ui.button("Refresh").clicked() {
            app.manual_refresh(ui.ctx());
        }
        if ui.button("Install APK...").clicked() {
            app.install.open = true;
        }
        let can_run = app.selected_project().is_some()
            && app.selected_session.is_some()
            && !app.active_operation();
        if ui
            .add_enabled(can_run, egui::Button::new("Run project flow..."))
            .on_hover_text("Fixture or conformance flow for the selected project and session")
            .clicked()
        {
            app.run_form = Some(RunForm::default());
        }
    });
    stale_note(ui, &app.runs, "Run list");
    ui.add_space(5.0);
    if app.runs.value.runs.is_empty() {
        empty_state(
            ui,
            "No runs found",
            "Run a fixture, conformance flow or external script to create a report.",
        );
        return;
    }
    let runs = app.runs.value.runs.clone();
    let ctx = ui.ctx().clone();
    let mut open_run = None;
    TableBuilder::new(ui)
        .sense(egui::Sense::click())
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(220.0).at_least(160.0))
        .column(Column::initial(65.0))
        .column(Column::initial(90.0))
        .column(Column::initial(70.0))
        .column(Column::initial(80.0))
        .column(Column::remainder().at_least(120.0))
        .column(Column::initial(46.0))
        .header(22.0, |mut header| {
            for name in [
                "Run", "Status", "Project", "Steps", "Duration", "Started", "",
            ] {
                header.col(|ui| {
                    ui.label(RichText::new(name).size(10.0).strong().color(theme::MUTED));
                });
            }
        })
        .body(|body| {
            body.rows(24.0, runs.len(), |mut row| {
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
                    ui.label(
                        RichText::new(&run.status)
                            .size(10.0)
                            .color(theme::status_color(&run.status)),
                    );
                });
                row.col(|ui| {
                    ui.label(RichText::new(&run.project).size(10.0));
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
                    ui.add(egui::Label::new(RichText::new(&run.started_at).size(9.0)).truncate())
                        .on_hover_text(&run.started_at);
                });
                row.col(|ui| {
                    if ui.small_button("Open").clicked() {
                        open_run = Some(run.run_id.clone());
                    }
                });
                if row.response().clicked() {
                    open_run = Some(run.run_id.clone());
                }
            });
        });
    if let Some(run_id) = open_run {
        app.open_run(run_id, &ctx);
    }
    muted(
        ui,
        "Select a run to open its steps, build identity and evidence.",
    );

    if let Some(bundle) = &app.project {
        if !bundle.hooks.is_empty() {
            ui.add_space(10.0);
            section_title(ui, "Project hooks");
            let busy = app.active_operation() || app.selected_session.is_none();
            let hooks = bundle.hooks.clone();
            for hook in hooks {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&hook.name).size(11.0).monospace());
                    ui.label(
                        RichText::new(hook.description.as_deref().unwrap_or(""))
                            .size(10.0)
                            .color(theme::MUTED),
                    );
                    if action_button(ui, "Run...", !busy, "Run this hook on the selected session")
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
}
