use eframe::egui;
use egui::RichText;

use crate::app::App;
use crate::components::*;
use crate::state::*;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    page_header(
        ui,
        "Sessions",
        "Device ownership, heartbeats and prepared state. Release keeps the emulator; stop is destructive.",
    );
    toolbar(ui, |ui| {
        if secondary_button(ui, "Refresh").clicked() {
            app.manual_refresh(ui.ctx());
        }
        if primary_button(ui, "Start emulator").clicked() {
            app.session_form = Some(SessionForm {
                owner: app.settings.owner.clone(),
                headless: true,
                ..Default::default()
            });
        }
        if secondary_button(ui, "Attach device").clicked() {
            app.attach_form = Some(AttachForm {
                owner: app.settings.owner.clone(),
                ..Default::default()
            });
        }
        if secondary_button(ui, "Install APK").clicked() {
            app.install.open = true;
        }
        if app.selected_session.is_some() && ghost_button(ui, "Clear selection").clicked() {
            app.selected_session = None;
        }
    });
    stale_note(ui, &app.sessions, "Session list");
    let mut details_rect = None;
    if let Some(session) = app.selected_session_info() {
        ui.add_space(6.0);
        let response = ui.scope(|ui| details(app, ui, &session));
        details_rect = Some(response.response.rect);
        ui.add_space(8.0);
    }
    ui.add_space(4.0);
    if app.sessions.value.sessions.is_empty() {
        if app.sessions.loading {
            ui.horizontal(|ui| {
                ui.spinner();
                muted(ui, "Loading sessions...");
            });
        } else {
            empty_state(
                ui,
                "No sessions",
                "Start an emulator or attach a device to create owned state.",
            );
        }
        return;
    }
    let sessions = app.sessions.value.sessions.clone();
    let mut select = None;
    let width = ui.available_width();
    let widths = proportional_widths(
        width,
        &[0.17, 0.12, 0.14, 0.20, 0.08, 0.14, 0.08, 0.07],
        10.0,
    );
    let session_width = widths[0];
    let owner_width = widths[1];
    let device_width = widths[2];
    let kind_width = widths[3];
    let heartbeat_width = widths[4];
    let lease_width = widths[5];
    let stale_width = widths[6];
    let select_width = widths[7];
    table_header(
        ui,
        &[
            ("Session", session_width),
            ("Owner", owner_width),
            ("Device", device_width),
            ("Kind / state", kind_width),
            ("Heartbeat", heartbeat_width),
            ("Lease", lease_width),
            ("Stale", stale_width),
            ("", select_width),
        ],
    );
    for session in &sessions {
        let selected = app.selected_session.as_deref() == Some(session.id.as_str());
        let response = selectable_row(ui, selected, theme::ROW_HEIGHT, |ui| {
            row_cell(ui, session_width, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(&session.id).size(10.0).monospace()).truncate(),
                )
                .on_hover_text(&session.id);
            });
            row_cell(ui, owner_width, |ui| {
                ui.add(egui::Label::new(RichText::new(&session.owner).size(10.0)).truncate());
            });
            row_cell(ui, device_width, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(session.serial.as_deref().unwrap_or("-"))
                            .size(10.0)
                            .monospace(),
                    )
                    .truncate(),
                );
            });
            row_cell(ui, kind_width, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&session.kind).size(9.0).color(theme::MUTED));
                    status_pill(ui, &session.state, theme::status_color(&session.state));
                });
            });
            row_cell(ui, heartbeat_width, |ui| {
                ui.label(RichText::new(format!("{}s", session.age_seconds)).size(10.0));
            });
            row_cell(ui, lease_width, |ui| {
                if let Some(lease) = &session.lease_expires_at {
                    if session.lease_expired {
                        status_pill(ui, "expired", theme::RED);
                    } else {
                        ui.add(
                            egui::Label::new(RichText::new(lease).size(9.0).monospace()).truncate(),
                        )
                        .on_hover_text(lease);
                    }
                } else {
                    ui.label(RichText::new("-").size(10.0).color(theme::MUTED));
                }
            });
            row_cell(ui, stale_width, |ui| {
                if session.stale {
                    status_pill(ui, "stale", theme::RED);
                } else {
                    ui.label(RichText::new("-").size(10.0).color(theme::MUTED));
                }
            });
            row_cell(ui, select_width, |ui| {
                if secondary_button(ui, if selected { "Clear" } else { "Select" }).clicked() {
                    select = Some(if selected {
                        None
                    } else {
                        Some(session.id.clone())
                    });
                }
            });
        });
        if response.clicked() {
            select = Some(if selected {
                None
            } else {
                Some(session.id.clone())
            });
        }
        ui.add_space(2.0);
    }
    if let Some(id) = select {
        app.selected_session = id;
    } else if ui.input(|input| {
        input.pointer.primary_clicked()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| !details_rect.is_some_and(|rect| rect.contains(pos)))
    }) {
        // Clicking outside a row or the details card clears the persistent
        // highlight. Clicking a details action must not hide its own panel.
        app.selected_session = None;
    }
}

fn details(app: &mut App, ui: &mut egui::Ui, session: &crate::models::SessionInfo) {
    section_title(ui, "Session details");
    card(ui, |ui| {
        ui.columns(2, |columns| {
            kv_grid(
                &mut columns[0],
                "session-main",
                &[
                    ("Session", session.id.clone()),
                    ("Owner", session.owner.clone()),
                    (
                        "Serial",
                        session.serial.clone().unwrap_or_else(|| "-".to_string()),
                    ),
                    ("Kind", session.kind.clone()),
                    ("State", theme::status_label(&session.state)),
                ],
            );
            kv_grid(
                &mut columns[1],
                "session-more",
                &[
                    (
                        "AVD",
                        session.avd.clone().unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Port",
                        session
                            .port
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Emulator PID",
                        session
                            .emulator_pid
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    ("Created", session.created_at.clone()),
                    (
                        "Heartbeat",
                        session
                            .heartbeat_at
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Lease",
                        session
                            .lease_expires_at
                            .clone()
                            .map(|lease| {
                                if session.lease_expired {
                                    format!("{lease} (expired)")
                                } else {
                                    lease
                                }
                            })
                            .unwrap_or_else(|| "none".to_string()),
                    ),
                    (
                        "Project",
                        session
                            .project_root
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            );
        });
        if let Some(error) = &session.last_error {
            error_banner(ui, error);
        }
        if !session.notes.is_empty() {
            muted(ui, session.notes.join(" · "));
        }
        if !session.device_settings.is_empty() {
            ui.collapsing("Prepared device settings", |ui| {
                for (key, value) in &session.device_settings {
                    ui.label(
                        RichText::new(format!("{key} = {}", value.as_deref().unwrap_or("-")))
                            .size(9.5)
                            .monospace(),
                    );
                }
            });
        }
        let actionable = !matches!(session.state.as_str(), "released" | "abandoned" | "failed");
        let busy = app.active_operation() || !actionable;
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if action_button(
                ui,
                "Release...",
                !busy,
                "Release ownership and keep the emulator running",
            )
            .clicked()
            {
                app.confirm(ConfirmAction::ReleaseSession(session.clone()));
            }
            if action_button(ui, "Stop...", !busy, "Stop the emulator; destructive").clicked() {
                app.confirm(ConfirmAction::StopSession(session.clone()));
            }
            if action_button(ui, "Restart...", !busy, "Restart an owned emulator").clicked() {
                app.confirm(ConfirmAction::RestartSession(session.clone()));
            }
            if action_button(
                ui,
                "Use as target",
                actionable,
                "Make this the action target",
            )
            .clicked()
            {
                app.selected_session = Some(session.id.clone());
            }
            if action_button(
                ui,
                "Install APK",
                actionable,
                "Install an APK on this session",
            )
            .clicked()
            {
                app.install.open = true;
            }
            if action_button(
                ui,
                "Inspector",
                actionable,
                "Open the semantic device inspector",
            )
            .clicked()
            {
                app.inspector_select_session(Some(session.id.clone()));
                app.set_page(Page::Inspector);
                app.start_inspector(ui.ctx());
            }
            if action_button(ui, "Logcat", actionable, "Open live logcat").clicked() {
                app.logcat_select_session(Some(session.id.clone()));
                app.set_page(Page::Logcat);
                app.start_logcat(ui.ctx());
            }
        });
        if !actionable {
            warning_banner(
                ui,
                "This session is a tombstone. Start or attach a session before running actions.",
            );
        }
        if session.kind == "attached" {
            warning_banner(
                ui,
                "Attached emulator: stopping requires --attached and is confirmed explicitly.",
            );
        }
    });
}
