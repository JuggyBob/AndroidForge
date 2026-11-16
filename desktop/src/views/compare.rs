use eframe::egui;
use egui::RichText;
use egui_extras::{Column, TableBuilder};

use crate::app::App;
use crate::components::*;
use crate::state::*;
use crate::theme;
use crate::views::format_duration;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    page_header(
        ui,
        "Compare",
        "Compare a run with a compatible baseline. Build and environment differences are called out explicitly.",
    );
    let runs = app.runs.value.runs.clone();
    if runs.is_empty() {
        empty_state(
            ui,
            "No runs to compare",
            "Create at least one report first.",
        );
        return;
    }
    card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            let current = app
                .compare_form
                .current
                .clone()
                .or_else(|| runs.first().map(|run| run.run_id.clone()));
            let current_label = current.clone().unwrap_or_else(|| "Current run".to_string());
            egui::ComboBox::from_id_salt("compare-current")
                .width(300.0)
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    for run in &runs {
                        if ui
                            .selectable_label(
                                app.compare_form.current.as_deref() == Some(run.run_id.as_str()),
                                format!("{} · {}", run.run_id, run.status),
                            )
                            .clicked()
                        {
                            app.choose_compare_current(run.run_id.clone());
                        }
                    }
                });
            ui.label(RichText::new("vs").size(10.0).color(theme::MUTED));
            let baseline_label = app
                .compare_form
                .baseline
                .clone()
                .unwrap_or_else(|| "Auto: latest compatible passed run".to_string());
            egui::ComboBox::from_id_salt("compare-baseline")
                .width(300.0)
                .selected_text(baseline_label)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            app.compare_form.baseline.is_none(),
                            "Auto: latest compatible passed run",
                        )
                        .clicked()
                    {
                        app.choose_compare_baseline(None);
                    }
                    for run in &runs {
                        if ui
                            .selectable_label(
                                app.compare_form.baseline.as_deref() == Some(run.run_id.as_str()),
                                format!("{} · {}", run.run_id, run.status),
                            )
                            .clicked()
                        {
                            app.choose_compare_baseline(Some(run.run_id.clone()));
                        }
                    }
                });
            if primary_button(ui, "Compare").clicked() {
                app.start_comparison(ui.ctx());
            }
            if app.compare.loading {
                ui.spinner();
            }
        });
        if let Some(error) = &app.compare.error {
            error_banner(ui, error);
        }
    });
    ui.add_space(10.0);
    let Some(comparison) = app.compare.comparison.clone() else {
        empty_state(
            ui,
            "No comparison yet",
            "Choose a run and baseline, then compare. Runs are only comparable when project and scenario match.",
        );
        return;
    };
    if !comparison.comparable {
        warning_banner(
            ui,
            "This comparison is not meaningful: the runs belong to different projects or scenarios.",
        );
    } else if !comparison.notes.is_empty() {
        for note in &comparison.notes {
            info_banner(ui, note);
        }
    }
    ui.add_space(6.0);
    ui.columns(4, |columns| {
        metric(
            &mut columns[0],
            "Current",
            &theme::status_label(&comparison.current_status),
            theme::status_color(&comparison.current_status),
            &comparison.current_run_id,
        );
        metric(
            &mut columns[1],
            "Baseline",
            &theme::status_label(&comparison.baseline_status),
            theme::status_color(&comparison.baseline_status),
            &comparison.baseline_run_id,
        );
        metric(
            &mut columns[2],
            "Duration",
            &format_duration(comparison.current_duration_ms),
            if comparison.duration_delta_ms > 0 {
                theme::YELLOW
            } else {
                theme::GREEN
            },
            &format!(
                "{:+.1}s vs baseline",
                comparison.duration_delta_ms as f64 / 1000.0
            ),
        );
        metric(
            &mut columns[3],
            "New failures",
            &comparison.new_failures.len().to_string(),
            if comparison.new_failures.is_empty() {
                theme::GREEN
            } else {
                theme::RED
            },
            "steps",
        );
    });
    ui.add_space(10.0);
    if !comparison.build_changes.is_empty() {
        section_label(ui, "Build identity changes");
        changes_table(ui, "compare-build", &comparison.build_changes);
    }
    if !comparison.environment_changes.is_empty() {
        section_label(ui, "Environment changes");
        changes_table(ui, "compare-env", &comparison.environment_changes);
    }
    section_label(ui, "Step changes");
    steps_table(ui, &comparison);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if secondary_button(ui, "Copy Markdown").clicked() {
            ui.ctx().copy_text(comparison_markdown(&comparison));
            app.toast("Comparison copied as Markdown", ToastLevel::Success);
        }
        if secondary_button(ui, "Copy JSON").clicked()
            && let Ok(text) = serde_json::to_string_pretty(&comparison)
        {
            ui.ctx().copy_text(text);
            app.toast("Comparison copied as JSON", ToastLevel::Success);
        }
        if secondary_button(ui, "Open current").clicked() {
            app.open_run(comparison.current_run_id.clone(), ui.ctx());
            app.set_page(Page::Runs);
        }
        if secondary_button(ui, "Open baseline").clicked() {
            app.open_run(comparison.baseline_run_id.clone(), ui.ctx());
            app.set_page(Page::Runs);
        }
    });
}

fn changes_table(ui: &mut egui::Ui, id: &str, changes: &[crate::models::FieldChange]) {
    egui::Grid::new(id)
        .num_columns(3)
        .spacing([14.0, 4.0])
        .max_col_width(260.0)
        .show(ui, |ui| {
            for heading in ["Field", "Baseline", "Current"] {
                ui.label(
                    RichText::new(heading)
                        .size(9.0)
                        .strong()
                        .color(theme::MUTED),
                );
            }
            ui.end_row();
            for change in changes {
                ui.label(RichText::new(&change.field).size(9.5).color(theme::MUTED));
                let baseline = short_value(&change.baseline);
                let current = short_value(&change.current);
                ui.add(egui::Label::new(RichText::new(baseline).size(9.5).monospace()).truncate())
                    .on_hover_text(&change.baseline);
                ui.add(
                    egui::Label::new(
                        RichText::new(current)
                            .size(9.5)
                            .monospace()
                            .color(theme::YELLOW),
                    )
                    .truncate(),
                )
                .on_hover_text(&change.current);
                ui.end_row();
            }
        });
}

fn short_value(value: &str) -> String {
    if value.chars().count() <= 28 {
        value.to_string()
    } else {
        let mut text = value.chars().take(26).collect::<String>();
        text.push('…');
        text
    }
}

fn steps_table(ui: &mut egui::Ui, comparison: &crate::models::RunComparison) {
    let steps = comparison.step_changes.clone();
    TableBuilder::new(ui)
        .striped(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::remainder().at_least(180.0))
        .column(Column::initial(110.0))
        .column(Column::initial(110.0))
        .column(Column::initial(90.0))
        .column(Column::initial(100.0))
        .header(24.0, |mut header| {
            for name in ["Step", "Current", "Baseline", "Delta", "Change"] {
                header.col(|ui| {
                    ui.label(RichText::new(name).size(9.0).strong().color(theme::MUTED));
                });
            }
        })
        .body(|body| {
            body.rows(28.0, steps.len(), |mut row| {
                let step = &steps[row.index()];
                row.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(&step.name).size(10.0)).truncate())
                        .on_hover_text(&step.command);
                });
                row.col(|ui| {
                    if let Some(status) = &step.current_status {
                        status_pill(ui, status, theme::status_color(status));
                    } else {
                        small(ui, "missing");
                    }
                });
                row.col(|ui| {
                    if let Some(status) = &step.baseline_status {
                        status_pill(ui, status, theme::status_color(status));
                    } else {
                        small(ui, "missing");
                    }
                });
                row.col(|ui| {
                    if let Some(delta) = step.duration_delta_ms {
                        let color = if delta > 0 {
                            theme::YELLOW
                        } else if delta < 0 {
                            theme::GREEN
                        } else {
                            theme::MUTED
                        };
                        ui.label(
                            RichText::new(format!("{:+.0}ms", delta as f64))
                                .size(9.5)
                                .color(color),
                        );
                    } else {
                        small(ui, "-");
                    }
                });
                row.col(|ui| {
                    status_pill(ui, &step.change, theme::status_color(&step.change));
                });
            });
        });
}

fn comparison_markdown(comparison: &crate::models::RunComparison) -> String {
    let mut out = format!(
        "# Run comparison\n\n- Current: `{}` ({})\n- Baseline: `{}` ({})\n- Comparable: {}\n- Duration: {} → {} ({:+}ms)\n",
        comparison.current_run_id,
        comparison.current_status,
        comparison.baseline_run_id,
        comparison.baseline_status,
        comparison.comparable,
        format_duration(comparison.baseline_duration_ms),
        format_duration(comparison.current_duration_ms),
        comparison.duration_delta_ms
    );
    if !comparison.new_failures.is_empty() {
        out.push_str(&format!(
            "\n## New failures\n\n{}\n",
            comparison
                .new_failures
                .iter()
                .map(|step| format!("- {step}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !comparison.resolved_failures.is_empty() {
        out.push_str(&format!(
            "\n## Resolved failures\n\n{}\n",
            comparison
                .resolved_failures
                .iter()
                .map(|step| format!("- {step}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    out.push_str(
        "\n## Steps\n\n| Step | Current | Baseline | Change |\n| --- | --- | --- | --- |\n",
    );
    for step in &comparison.step_changes {
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            step.name,
            step.current_status.as_deref().unwrap_or("-"),
            step.baseline_status.as_deref().unwrap_or("-"),
            step.change
        ));
    }
    out
}
