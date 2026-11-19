use eframe::egui;
use egui::{Align, Layout, RichText};

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
    ui.add_space(theme::PAGE_GAP);
    ui.columns(4, |columns| {
        metric_card(&mut columns[0], "Active sessions", active, theme::GREEN);
        metric_card(&mut columns[1], "Device owners", active, theme::BLUE);
        metric_card(&mut columns[2], "Stale active", stale, theme::YELLOW);
        metric_card(&mut columns[3], "Tombstones", tombstones, theme::MUTED);
    });
    ui.add_space(theme::PAGE_GAP);
    card(ui, |ui| {
        section_title(ui, "Sessions and leases");
        if sessions.is_empty() {
            muted(ui, "No sessions are recorded.");
            return;
        }
        let rows = sessions.clone();
        let width = ui.available_width();
        let widths = proportional_widths(width, &[0.18, 0.12, 0.14, 0.12, 0.22, 0.22], 10.0);
        let session_width = widths[0];
        let owner_width = widths[1];
        let device_width = widths[2];
        let state_width = widths[3];
        let lease_width = widths[4];
        let project_width = widths[5];
        table_header(
            ui,
            &[
                ("Session", session_width),
                ("Owner", owner_width),
                ("Device", device_width),
                ("State", state_width),
                ("Lease", lease_width),
                ("Project", project_width),
            ],
        );
        for session in &rows {
            selectable_row(ui, false, theme::ROW_HEIGHT, |ui| {
                row_cell(ui, session_width, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&session.id).size(9.5).monospace())
                            .truncate(),
                    );
                    if session.stale && is_active(session) {
                        status_pill(ui, "stale", theme::YELLOW);
                    }
                    if session.operation_active {
                        status_pill(ui, "busy", theme::accent());
                    }
                });
                row_cell(ui, owner_width, |ui| {
                    ui.add(egui::Label::new(RichText::new(&session.owner).size(10.0)).truncate());
                });
                row_cell(ui, device_width, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(session.serial.as_deref().unwrap_or("-"))
                                .size(9.5)
                                .monospace(),
                        )
                        .truncate(),
                    );
                });
                row_cell(ui, state_width, |ui| {
                    status_pill(ui, &session.state, theme::status_color(&session.state));
                });
                row_cell(ui, lease_width, |ui| {
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
                    ui.add(
                        egui::Label::new(RichText::new(lease).size(9.5).color(
                            if session.lease_expired {
                                theme::RED
                            } else {
                                theme::MUTED
                            },
                        ))
                        .truncate(),
                    );
                });
                row_cell(ui, project_width, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                session.project_root.as_deref().unwrap_or("unknown project"),
                            )
                            .size(9.0)
                            .color(theme::MUTED),
                        )
                        .truncate(),
                    )
                    .on_hover_text(session.project_root.as_deref().unwrap_or("-"));
                });
            });
            ui.add_space(2.0);
        }
    });
    ui.add_space(10.0);
    card(ui, |ui| {
        section_title(ui, "Devices");
        if devices.is_empty() {
            muted(ui, "No devices are connected.");
            return;
        }
        for device in &devices {
            selectable_row(ui, false, theme::ROW_HEIGHT, |ui| {
                row_cell(ui, 170.0, |ui| {
                    ui.label(RichText::new(&device.serial).size(10.0).monospace());
                });
                row_cell(ui, 80.0, |ui| {
                    status_pill(ui, &device.state, theme::status_color(&device.state));
                });
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
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
            });
            ui.add_space(2.0);
        }
    });
    ui.add_space(theme::PAGE_GAP);
    safe_alternatives(ui, &sessions);
    ui.add_space(theme::PAGE_GAP);
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
    card_min_height(ui, 78.0, |ui| {
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
