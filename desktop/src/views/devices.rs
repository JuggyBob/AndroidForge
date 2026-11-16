use eframe::egui;
use egui::RichText;
use egui_extras::{Column, TableBuilder};

use crate::app::App;
use crate::components::*;
use crate::state::*;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    page_header(
        ui,
        "Devices",
        "Connected Android devices, current ownership and physical capabilities.",
    );
    toolbar(ui, |ui| {
        if secondary_button(ui, "Refresh").clicked() {
            app.manual_refresh(ui.ctx());
        }
        muted(
            ui,
            format!(
                "{} device(s) · automatic refresh while this page is visible",
                app.devices.value.count
            ),
        );
    });
    stale_note(ui, &app.devices, "Device list");
    ui.add_space(4.0);
    if app.devices.value.devices.is_empty() {
        empty_state(
            ui,
            "No devices connected",
            "Start an emulator session or connect a USB-debugging device.",
        );
        return;
    }
    let devices = app.devices.value.devices.clone();
    let mut select = None;
    TableBuilder::new(ui)
        .sense(egui::Sense::click())
        .striped(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(150.0).at_least(110.0))
        .column(Column::remainder().at_least(120.0))
        .column(Column::initial(85.0))
        .column(Column::initial(75.0))
        .column(Column::initial(180.0))
        .column(Column::initial(58.0))
        .header(24.0, |mut header| {
            for name in ["Serial", "Model", "State", "Android", "Owner", ""] {
                header.col(|ui| {
                    ui.label(RichText::new(name).size(9.0).strong().color(theme::MUTED));
                });
            }
        })
        .body(|body| {
            body.rows(28.0, devices.len(), |mut row| {
                let device = &devices[row.index()];
                row.set_selected(app.selected_device.as_deref() == Some(device.serial.as_str()));
                row.col(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&device.serial).size(10.0).monospace())
                            .truncate(),
                    )
                    .on_hover_text(&device.serial);
                });
                row.col(|ui| {
                    ui.label(RichText::new(device.model.as_deref().unwrap_or("-")).size(10.5));
                });
                row.col(|ui| {
                    status_pill(ui, &device.state, theme::status_color(&device.state));
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(device.android_version.as_deref().unwrap_or("-")).size(10.0),
                    );
                });
                row.col(|ui| {
                    let owner = device
                        .owner
                        .as_ref()
                        .map(|owner| format!("{} · {}", owner.owner, owner.session_id))
                        .unwrap_or_else(|| "unowned".to_string());
                    ui.add(egui::Label::new(RichText::new(owner).size(10.0)).truncate());
                });
                row.col(|ui| {
                    if ui.small_button("Select").clicked() {
                        select = Some(device.serial.clone());
                    }
                });
                if row.response().clicked() {
                    select = Some(device.serial.clone());
                }
            });
        });
    if let Some(serial) = select {
        app.selected_device = Some(serial);
    }
    if let Some(device) = app.selected_device_info() {
        ui.add_space(8.0);
        details(app, ui, &device);
    }
}

fn details(app: &mut App, ui: &mut egui::Ui, device: &crate::models::Device) {
    section_title(ui, "Device details");
    card(ui, |ui| {
        ui.columns(2, |columns| {
            kv_grid(
                &mut columns[0],
                "device-main",
                &[
                    ("Serial", device.serial.clone()),
                    (
                        "Model",
                        device.model.clone().unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Manufacturer",
                        device
                            .manufacturer
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Android",
                        device
                            .android_version
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "API",
                        device
                            .api_level
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            );
            kv_grid(
                &mut columns[1],
                "device-more",
                &[
                    (
                        "Resolution",
                        device.resolution.clone().unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Architecture",
                        device
                            .architecture
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Battery",
                        device
                            .battery_level
                            .map(|value| format!("{value}%"))
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    ("AVD", device.avd.clone().unwrap_or_else(|| "-".to_string())),
                    (
                        "Port",
                        device
                            .port
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            );
        });
        ui.add_space(6.0);
        if let Some(owner) = &device.owner {
            info_banner(
                ui,
                &format!(
                    "Owned by {} · {} · {}",
                    owner.owner, owner.session_id, owner.state
                ),
            );
        } else {
            muted(ui, "Not currently owned by an AndroidForge session.");
        }
        ui.horizontal(|ui| {
            copy_button(ui, &device.serial, "Copy serial");
            copy_button(
                ui,
                device.resolution.as_deref().unwrap_or("-"),
                "Copy resolution",
            );
            if let Some(session_id) = app
                .sessions
                .value
                .sessions
                .iter()
                .find(|session| session.serial.as_deref() == Some(device.serial.as_str()))
                .map(|session| session.id.clone())
            {
                if secondary_button(ui, "Open inspector").clicked() {
                    app.selected_session = Some(session_id.clone());
                    app.inspector_select_session(Some(session_id.clone()));
                    app.set_page(Page::Inspector);
                    app.start_inspector(ui.ctx());
                }
                if secondary_button(ui, "Use as target").clicked() {
                    app.selected_session = Some(session_id.clone());
                }
            }
        });
    });
}
