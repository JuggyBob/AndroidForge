use eframe::egui;
use egui::RichText;

use crate::app::App;
use crate::components::*;
use crate::state::*;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    page_header(
        ui,
        "Maintenance",
        "Inspect AndroidForge-owned state, preview deletions and prune only terminal data.",
    );
    toolbar(ui, |ui| {
        if secondary_button(ui, "Refresh").clicked() {
            app.refresh_maintenance(ui.ctx());
        }
        if app.maintenance.summary.loading {
            ui.spinner();
        }
    });
    stale_note(ui, &app.maintenance.summary, "Storage summary");
    let summary = app.maintenance.summary.value.clone();
    ui.columns(4, |columns| {
        metric(
            &mut columns[0],
            "Total",
            &format_bytes(summary.total_bytes),
            theme::accent(),
            "AndroidForge state + artifacts",
        );
        metric(
            &mut columns[1],
            "Sessions",
            &summary.session_count.to_string(),
            if summary.active_session_count > 0 {
                theme::GREEN
            } else {
                theme::MUTED
            },
            &format!(
                "{} active · {} terminal",
                summary.active_session_count, summary.terminal_session_count
            ),
        );
        metric(
            &mut columns[2],
            "Runs",
            &summary.run_count.to_string(),
            if summary.running_run_count > 0 {
                theme::YELLOW
            } else {
                theme::GREEN
            },
            &format!(
                "{} running · {} terminal",
                summary.running_run_count, summary.terminal_run_count
            ),
        );
        metric(
            &mut columns[3],
            "Stale sessions",
            &summary.stale_session_count.to_string(),
            if summary.stale_session_count > 0 {
                theme::YELLOW
            } else {
                theme::GREEN
            },
            "ownership records needing review",
        );
    });
    ui.add_space(theme::PAGE_GAP);
    card(ui, |ui| {
        section_label(ui, "Managed paths");
        path_row(ui, &summary.state_dir);
        path_row(ui, &summary.artifacts_dir);
        small(ui, "Nothing outside these managed directories is removed.");
    });
    ui.add_space(theme::PAGE_GAP);
    ui.columns(2, |columns| {
        cleanup_controls(app, &mut columns[0]);
        cleanup_preview(app, &mut columns[1]);
    });
    ui.add_space(theme::PAGE_GAP);
    ui.columns(2, |columns| {
        sessions_table(app, &mut columns[0], &summary);
        runs_table(app, &mut columns[1], &summary);
    });
}

fn cleanup_controls(app: &mut App, ui: &mut egui::Ui) {
    card(ui, |ui| {
        section_title(ui, "Safe cleanup");
        muted(
            ui,
            "Preview first. Active sessions and running runs are never removed; tombstones and terminal runs are eligible.",
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Older than").size(10.0).color(theme::MUTED));
            ui.add(
                egui::DragValue::new(&mut app.maintenance.older_days)
                    .range(1..=3650)
                    .suffix(" days"),
            );
            ui.label(RichText::new("Keep latest").size(10.0).color(theme::MUTED));
            ui.add(
                egui::DragValue::new(&mut app.maintenance.keep_last)
                    .range(0..=500)
                    .suffix(" runs"),
            );
        });
        ui.horizontal(|ui| {
            ui.checkbox(
                &mut app.maintenance.include_sessions,
                "Released / failed session tombstones",
            );
            ui.checkbox(&mut app.maintenance.include_runs, "Historical runs");
        });
        ui.horizontal(|ui| {
            if primary_button(ui, "Preview cleanup").clicked() {
                app.preview_cleanup(ui.ctx());
            }
            if app.maintenance.preview_loading {
                ui.spinner();
                ui.label("Calculating...");
            }
            let has_work = app.maintenance.preview.as_ref().is_some_and(|preview| {
                let sessions = preview
                    .get("sessions")
                    .and_then(|value| value.get("pruned"))
                    .and_then(|value| value.as_array())
                    .is_some_and(|value| !value.is_empty());
                let runs = preview
                    .get("runs")
                    .and_then(|value| value.get("deleted_runs"))
                    .and_then(|value| value.as_array())
                    .is_some_and(|value| !value.is_empty());
                sessions || runs
            });
            let can_clean = has_work && !app.maintenance.cleaning && !app.active_operation();
            if action_button(
                ui,
                "Clean now...",
                can_clean,
                if has_work {
                    "Preview first, then confirm deletion"
                } else {
                    "Nothing eligible to remove in the current preview"
                },
            )
            .clicked()
            {
                app.request_cleanup_confirmation();
            }
        });
        if let Some(error) = &app.maintenance.preview_error {
            error_banner(ui, error);
        }
        if let Some(result) = &app.maintenance.last_result {
            info_banner(ui, result);
        }
    });
}

fn cleanup_preview(app: &mut App, ui: &mut egui::Ui) {
    card(ui, |ui| {
        section_title(ui, "Preview");
        let Some(preview) = app.maintenance.preview.as_ref() else {
            muted(
                ui,
                "Run Preview cleanup to see exactly what would be removed.",
            );
            return;
        };
        let session_entries = preview
            .get("sessions")
            .and_then(|value| value.get("pruned"))
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let run_entries = preview
            .get("runs")
            .and_then(|value| value.get("deleted_runs"))
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let removed_bytes = preview
            .get("runs")
            .and_then(|value| value.get("removed_bytes"))
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        kv_grid(
            ui,
            "cleanup-preview",
            &[
                ("Sessions", session_entries.len().to_string()),
                ("Runs", run_entries.len().to_string()),
                ("Run bytes", format_bytes(removed_bytes)),
                (
                    "Mode",
                    if preview
                        .get("dry_run")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                    {
                        "dry run".to_string()
                    } else {
                        "live".to_string()
                    },
                ),
            ],
        );
        if session_entries.is_empty() && run_entries.is_empty() {
            ui.add_space(4.0);
            small(ui, "No session tombstones or historical runs match.");
        } else {
            if !session_entries.is_empty() {
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("cleanup-session-list")
                    .max_height(110.0)
                    .show(ui, |ui| {
                        for entry in &session_entries {
                            let id = entry
                                .get("id")
                                .and_then(|value| value.as_str())
                                .unwrap_or("-");
                            let state = entry
                                .get("state")
                                .and_then(|value| value.as_str())
                                .unwrap_or("-");
                            ui.horizontal(|ui| {
                                status_pill(ui, state, theme::status_color(state));
                                ui.label(RichText::new(id).size(9.5).monospace());
                            });
                        }
                    });
            }
            if !run_entries.is_empty() {
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("cleanup-run-list")
                    .max_height(110.0)
                    .show(ui, |ui| {
                        for run in &run_entries {
                            if let Some(run) = run.as_str() {
                                ui.label(RichText::new(run).size(9.5).monospace());
                            }
                        }
                    });
            }
        }
    });
}

fn sessions_table(_app: &mut App, ui: &mut egui::Ui, summary: &crate::models::MaintenanceSummary) {
    card_fixed_height(ui, 310.0, "maintenance-sessions", |ui| {
        section_title(ui, "Terminal sessions");
        if summary.terminal_sessions.is_empty() {
            muted(ui, "No released, failed or abandoned sessions.");
            return;
        }
        for session in summary.terminal_sessions.iter().take(8) {
            selectable_row(ui, false, theme::ROW_HEIGHT, |ui| {
                row_cell(ui, 82.0, |ui| {
                    status_pill(ui, &session.state, theme::status_color(&session.state));
                });
                row_cell(ui, 150.0, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&session.id).size(9.5).monospace())
                            .truncate(),
                    );
                });
                small(
                    ui,
                    format!(
                        "{} · {}",
                        session.serial.as_deref().unwrap_or("-"),
                        format_bytes(session.bytes)
                    ),
                );
            });
        }
    });
}

fn runs_table(app: &mut App, ui: &mut egui::Ui, summary: &crate::models::MaintenanceSummary) {
    card_fixed_height(ui, 310.0, "maintenance-runs", |ui| {
        section_title(ui, "Largest runs");
        if summary.largest_runs.is_empty() {
            muted(ui, "No run artifacts are recorded.");
            return;
        }
        for run in summary.largest_runs.iter().take(8) {
            selectable_row(ui, false, theme::ROW_HEIGHT, |ui| {
                row_cell(ui, 82.0, |ui| {
                    status_pill(ui, &run.status, theme::status_color(&run.status));
                });
                row_cell(ui, 210.0, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&run.run_id).size(9.5).monospace())
                            .truncate(),
                    );
                });
                small(ui, format_bytes(run.bytes));
            });
        }
        if secondary_button(ui, "Open runs").clicked() {
            app.set_page(Page::Runs);
        }
    });
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
