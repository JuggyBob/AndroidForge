use eframe::egui;
use egui::RichText;
use egui_extras::{Column, TableBuilder};

use crate::app::App;
use crate::components::{
    action_button, empty_state, key_value, section_title, stale_note, title_row,
};
use crate::state::*;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    title_row(
        ui,
        "Sessions",
        "AndroidForge device ownership, heartbeat and prepared settings.",
    );
    ui.horizontal(|ui| {
        if ui.button("Refresh").clicked() {
            app.manual_refresh(ui.ctx());
        }
        if ui.button("Start emulator...").clicked() {
            let owner = app.settings.owner.clone();
            app.session_form = Some(SessionForm {
                owner,
                headless: true,
                prepare: false,
                ..Default::default()
            });
        }
        if ui.button("Attach device...").clicked() {
            let owner = app.settings.owner.clone();
            app.attach_form = Some(AttachForm {
                owner,
                prepare: false,
                ..Default::default()
            });
        }
        if ui.button("Install APK...").clicked() {
            app.install.open = true;
        }
    });
    stale_note(ui, &app.sessions, "Session list");
    ui.add_space(5.0);
    if app.sessions.value.sessions.is_empty() {
        empty_state(
            ui,
            "No sessions",
            "Start an emulator or attach to a device to create an owned session.",
        );
        return;
    }

    let sessions = app.sessions.value.sessions.clone();
    TableBuilder::new(ui)
        .sense(egui::Sense::click())
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(180.0).at_least(140.0))
        .column(Column::initial(100.0))
        .column(Column::initial(130.0))
        .column(Column::remainder().at_least(100.0))
        .column(Column::initial(80.0))
        .column(Column::initial(55.0))
        .column(Column::initial(58.0))
        .header(22.0, |mut header| {
            for name in [
                "Session",
                "Owner",
                "Device",
                "AVD / state",
                "Heartbeat",
                "Stale",
                "",
            ] {
                header.col(|ui| {
                    ui.label(RichText::new(name).size(10.0).strong().color(theme::MUTED));
                });
            }
        })
        .body(|body| {
            body.rows(24.0, sessions.len(), |mut row| {
                let session = &sessions[row.index()];
                let selected = app.selected_session.as_deref() == Some(session.id.as_str());
                row.set_selected(selected);
                row.col(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&session.id).size(10.0).monospace())
                            .truncate(),
                    )
                    .on_hover_text(&session.id);
                });
                row.col(|ui| {
                    ui.label(RichText::new(&session.owner).size(10.0));
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(session.serial.as_deref().unwrap_or("-"))
                            .size(10.0)
                            .monospace(),
                    );
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            session.avd.as_deref().unwrap_or("-"),
                            session.state
                        ))
                        .size(10.0)
                        .color(theme::status_color(&session.state)),
                    );
                });
                row.col(|ui| {
                    ui.label(RichText::new(format!("{}s", session.age_seconds)).size(10.0));
                });
                row.col(|ui| {
                    if session.stale {
                        ui.label(RichText::new("stale").size(10.0).color(theme::RED));
                    } else {
                        ui.label(RichText::new("-").size(10.0).color(theme::MUTED));
                    }
                });
                row.col(|ui| {
                    if ui.small_button("Select").clicked() {
                        app.selected_session = Some(session.id.clone());
                    }
                });
                if row.response().clicked() {
                    app.selected_session = Some(session.id.clone());
                }
            });
        });

    if let Some(session) = app.selected_session_info() {
        ui.add_space(8.0);
        details(app, ui, &session);
    }
}

fn details(app: &mut App, ui: &mut egui::Ui, session: &crate::models::SessionInfo) {
    section_title(ui, "Session details");
    egui::Frame::new()
        .fill(if ui.style().visuals.dark_mode {
            theme::DARK_SURFACE
        } else {
            theme::LIGHT_SURFACE
        })
        .stroke(egui::Stroke::new(1.0, theme::DARK_BORDER))
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            egui::Grid::new("session-details")
                .num_columns(4)
                .spacing([18.0, 4.0])
                .show(ui, |ui| {
                    key_value(ui, "Session", &session.id);
                    key_value(ui, "Owner", &session.owner);
                    ui.end_row();
                    key_value(ui, "Serial", session.serial.as_deref().unwrap_or("-"));
                    key_value(ui, "AVD", session.avd.as_deref().unwrap_or("-"));
                    ui.end_row();
                    key_value(ui, "Kind", &session.kind);
                    key_value(ui, "State", &session.state);
                    ui.end_row();
                    key_value(
                        ui,
                        "Port",
                        &session
                            .port
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    );
                    key_value(
                        ui,
                        "PID",
                        &session
                            .emulator_pid
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    );
                    ui.end_row();
                    key_value(ui, "Created", &session.created_at);
                    key_value(
                        ui,
                        "Heartbeat",
                        session.heartbeat_at.as_deref().unwrap_or("-"),
                    );
                    ui.end_row();
                });
            if let Some(error) = &session.last_error {
                ui.colored_label(theme::RED, error);
            }
            if !session.notes.is_empty() {
                ui.label(
                    RichText::new(format!("Notes: {}", session.notes.join(" · ")))
                        .size(10.0)
                        .color(theme::MUTED),
                );
            }
            if !session.device_settings.is_empty() {
                ui.collapsing("Prepared settings", |ui| {
                    for (key, value) in &session.device_settings {
                        ui.label(
                            RichText::new(format!("{key} = {}", value.as_deref().unwrap_or("-")))
                                .size(10.0)
                                .monospace(),
                        );
                    }
                });
            }
            let actionable = !matches!(session.state.as_str(), "released" | "abandoned");
            let busy = app.active_operation() || !actionable;
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                if action_button(
                    ui,
                    "Release...",
                    !busy,
                    "Release ownership; keep the emulator running",
                )
                .clicked()
                {
                    app.confirm(ConfirmAction::ReleaseSession(session.clone()));
                }
                if action_button(ui, "Stop...", !busy, "Stop the emulator; destructive").clicked() {
                    app.confirm(ConfirmAction::StopSession(session.clone()));
                }
                if action_button(ui, "Restart...", !busy, "Restart the emulator").clicked() {
                    app.confirm(ConfirmAction::RestartSession(session.clone()));
                }
                if action_button(
                    ui,
                    "Use as target",
                    actionable,
                    "Make this session the action target",
                )
                .clicked()
                {
                    app.selected_session = Some(session.id.clone());
                }
                if action_button(ui, "Install APK...", actionable, "Install an APK on this session")
                    .clicked()
                {
                    app.install.open = true;
                }
            });
            if !actionable {
                ui.colored_label(
                    theme::YELLOW,
                    "This session is a tombstone. Start or attach a session before running actions.",
                );
            }
            if session.kind == "attached" {
                ui.colored_label(
                    theme::YELLOW,
                    "Attached emulator: stopping requires --attached and is confirmed explicitly.",
                );
            }
        });
}
