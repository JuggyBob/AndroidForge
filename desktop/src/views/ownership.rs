use eframe::egui;
use egui::RichText;
use egui_extras::{Column, TableBuilder};

use crate::app::App;
use crate::components::*;
use crate::models::SessionInfo;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    page_header(
        ui,
        "Ownership",
        "Who owns each device and session, which leases are stale, and safe commands when a device is occupied.",
    );
    ui.horizontal(|ui| {
        if primary_button(ui, "Refresh ownership").clicked() {
            app.manual_refresh(ui.ctx());
        }
        status_pill(ui, "read-only dashboard", theme::MUTED);
        if app.sessions.loading || app.devices.loading {
            ui.spinner();
        }
        if let Some(updated) = app.sessions.updated {
            muted(
                ui,
                format!(
                    "Session snapshot age: {:.1}s",
                    updated.elapsed().as_secs_f64()
                ),
            );
        }
    });
    if let Some(error) = &app.sessions.error {
        error_banner(ui, error);
    }
    if let Some(error) = &app.devices.error {
        error_banner(ui, error);
    }
    let sessions = app.sessions.value.sessions.clone();
    let devices = app.devices.value.devices.clone();
    let is_active =
        |session: &SessionInfo| matches!(session.state.as_str(), "ready" | "attached" | "starting");
    let active = sessions.iter().filter(|session| is_active(session)).count();
    let stale = sessions
        .iter()
        .filter(|session| is_active(session) && session.stale)
        .count();
    let tombstones = sessions.len().saturating_sub(active);
    let _operations = sessions
        .iter()
        .filter(|session| session.operation_active)
        .count();
    ui.add_space(8.0);
    ui.columns(4, |columns| {
        metric_card(&mut columns[0], "Active sessions", active, theme::GREEN);
        metric_card(&mut columns[1], "Device owners", active, theme::BLUE);
        metric_card(&mut columns[2], "Stale active", stale, theme::YELLOW);
        metric_card(&mut columns[3], "Tombstones", tombstones, theme::MUTED);
    });
    ui.add_space(10.0);
    card(ui, |ui| {
        section_title(ui, "Sessions and leases");
        if sessions.is_empty() {
            muted(ui, "No sessions are recorded.");
            return;
        }
        let rows = sessions.clone();
        TableBuilder::new(ui)
            .striped(false)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(150.0).at_least(120.0))
            .column(Column::initial(110.0))
            .column(Column::initial(110.0))
            .column(Column::initial(90.0))
            .column(Column::remainder().at_least(140.0))
            .column(Column::initial(150.0))
            .header(24.0, |mut header| {
                for name in ["Session", "Owner", "Device", "State", "Lease", "Project"] {
                    header.col(|ui| {
                        ui.label(RichText::new(name).size(9.0).strong().color(theme::MUTED));
                    });
                }
            })
            .body(|body| {
                body.rows(30.0, rows.len(), |mut row| {
                    let session = &rows[row.index()];
                    row.col(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&session.id).size(9.5).monospace());
                            if session.stale && is_active(session) {
                                status_pill(ui, "stale", theme::YELLOW);
                            }
                            if session.operation_active {
                                status_pill(ui, "busy", theme::ACCENT);
                            }
                        });
                    });
                    row.col(|ui| {
                        ui.label(RichText::new(&session.owner).size(10.0));
                    });
                    row.col(|ui| {
                        ui.label(
                            RichText::new(session.serial.as_deref().unwrap_or("-"))
                                .size(9.5)
                                .monospace(),
                        );
                    });
                    row.col(|ui| {
                        status_pill(ui, &session.state, theme::status_color(&session.state));
                    });
                    row.col(|ui| {
                        let lease = session
                            .lease_expires_at
                            .as_deref()
                            .map(|expires| {
                                if session.lease_expired {
                                    format!("expired ({expires})")
                                } else {
                                    expires.to_string()
                                }
                            })
                            .unwrap_or_else(|| "no lease".to_string());
                        ui.label(
                            RichText::new(lease)
                                .size(9.5)
                                .color(if session.lease_expired {
                                    theme::RED
                                } else {
                                    theme::MUTED
                                }),
                        );
                    });
                    row.col(|ui| {
                        ui.label(
                            RichText::new(
                                session.project_root.as_deref().unwrap_or("unknown project"),
                            )
                            .size(9.0)
                            .color(theme::MUTED),
                        )
                        .on_hover_text(session.project_root.as_deref().unwrap_or("-"));
                    });
                });
            });
    });
    ui.add_space(10.0);
    card(ui, |ui| {
        section_title(ui, "Devices");
        if devices.is_empty() {
            muted(ui, "No devices are connected.");
            return;
        }
        for device in &devices {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&device.serial).size(10.0).monospace());
                status_pill(ui, &device.state, theme::status_color(&device.state));
                ui.label(
                    RichText::new(format!(
                        "{} {} · API {}",
                        device.manufacturer.as_deref().unwrap_or(""),
                        device.model.as_deref().unwrap_or("unknown model"),
                        device
                            .api_level
                            .map(|level| level.to_string())
                            .unwrap_or_else(|| "?".to_string())
                    ))
                    .size(9.5)
                    .color(theme::MUTED),
                );
                if let Some(owner) = &device.owner {
                    status_pill(
                        ui,
                        &format!("{}: {}", owner.owner, owner.session_id),
                        theme::BLUE,
                    );
                } else {
                    status_pill(ui, "unowned", theme::MUTED);
                }
            });
        }
    });
    ui.add_space(10.0);
    safe_alternatives(ui, &sessions);
    ui.add_space(10.0);
    card(ui, |ui| {
        section_title(ui, "Ownership safety");
        small(
            ui,
            "This page never stops or releases a session. That is deliberate: a session may belong to another agent. Use the copied command only after confirming that you own the session.",
        );
        if let Some(error) = sessions
            .iter()
            .find_map(|session| session.last_error.as_deref())
        {
            ui.add_space(4.0);
            warning_banner(ui, &format!("Last reported session error: {error}"));
        }
    });
}

fn metric_card(ui: &mut egui::Ui, label: &str, value: usize, color: egui::Color32) {
    card(ui, |ui| {
        ui.label(RichText::new(label).size(9.5).color(theme::MUTED));
        ui.label(
            RichText::new(value.to_string())
                .size(24.0)
                .strong()
                .color(color),
        );
    });
}

fn safe_alternatives(ui: &mut egui::Ui, sessions: &[SessionInfo]) {
    let busy = sessions
        .iter()
        .filter(|session| {
            matches!(session.state.as_str(), "ready" | "attached" | "starting")
                && session.serial.is_some()
        })
        .collect::<Vec<_>>();
    card(ui, |ui| {
        section_title(ui, "Safe alternatives when a device is occupied");
        if busy.is_empty() {
            muted(ui, "No occupied device requires an alternative right now.");
        }
        for session in busy.iter().take(6) {
            let serial = session.serial.as_deref().unwrap_or("<serial>");
            ui.add_space(3.0);
            ui.label(
                RichText::new(format!(
                    "{} is owned by {} in session {}",
                    serial, session.owner, session.id
                ))
                .size(9.5)
                .color(theme::MUTED),
            );
            for (label, command) in [
                (
                    "Inspect the owner",
                    format!(
                        "androidforge session inspect --session {} --json",
                        session.id
                    ),
                ),
                (
                    "Start an independent target",
                    "androidforge session ensure --avd <other-avd> --owner <your-owner> --json"
                        .to_string(),
                ),
                (
                    "See all devices first",
                    "androidforge devices list --json".to_string(),
                ),
            ] {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&command).size(9.5).monospace());
                    copy_button(ui, &command, label);
                });
            }
        }
    });
}
