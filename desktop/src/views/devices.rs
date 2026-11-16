use eframe::egui;
use egui::RichText;
use egui_extras::{Column, TableBuilder};

use crate::app::App;
use crate::components::{empty_state, key_value, muted, section_title, stale_note, title_row};
use crate::state::DataState;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    title_row(
        ui,
        "Devices",
        "Connected and discoverable Android devices. Ownership is reported by AndroidForge sessions.",
    );
    ui.horizontal(|ui| {
        if ui.button("Refresh").clicked() {
            app.manual_refresh(ui.ctx());
        }
        muted(
            ui,
            format!(
                "{} device(s) · automatic refresh every 3s",
                app.devices.value.count
            ),
        );
    });
    stale_note(ui, &app.devices, "Device list");
    ui.add_space(5.0);
    if app.devices.value.devices.is_empty() {
        empty_state(
            ui,
            "No devices connected",
            "Start an emulator session or connect a USB-debugging device.",
        );
        return;
    }

    let devices = app.devices.value.devices.clone();
    TableBuilder::new(ui)
        .sense(egui::Sense::click())
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(150.0).at_least(110.0))
        .column(Column::remainder().at_least(120.0))
        .column(Column::initial(75.0))
        .column(Column::initial(80.0))
        .column(Column::initial(150.0))
        .column(Column::initial(58.0))
        .header(22.0, |mut header| {
            for name in ["Serial", "Model", "State", "Android", "Owner", ""] {
                header.col(|ui| {
                    ui.label(RichText::new(name).size(10.0).strong().color(theme::MUTED));
                });
            }
        })
        .body(|body| {
            body.rows(24.0, devices.len(), |mut row| {
                let device = &devices[row.index()];
                let selected = app.selected_device.as_deref() == Some(device.serial.as_str());
                row.set_selected(selected);
                row.col(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&device.serial).size(10.0).monospace())
                            .truncate(),
                    )
                    .on_hover_text(&device.serial);
                });
                row.col(|ui| {
                    ui.label(RichText::new(device.model.as_deref().unwrap_or("-")).size(11.0));
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(&device.state)
                            .size(10.0)
                            .color(theme::status_color(&device.state)),
                    );
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
                    ui.label(RichText::new(owner).size(10.0));
                });
                row.col(|ui| {
                    if ui.small_button("Select").clicked() {
                        app.selected_device = Some(device.serial.clone());
                    }
                });
                if row.response().clicked() {
                    app.selected_device = Some(device.serial.clone());
                }
            });
        });

    if let Some(device) = app.selected_device_info() {
        ui.add_space(8.0);
        details(ui, &device, app);
    }
}

fn details(ui: &mut egui::Ui, device: &crate::models::Device, app: &App) {
    section_title(ui, "Device details");
    egui::Frame::new()
        .fill(if ui.style().visuals.dark_mode {
            theme::DARK_SURFACE
        } else {
            theme::LIGHT_SURFACE
        })
        .stroke(egui::Stroke::new(1.0, theme::DARK_BORDER))
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            egui::Grid::new("device-details")
                .num_columns(4)
                .spacing([18.0, 4.0])
                .show(ui, |ui| {
                    key_value(ui, "Serial", &device.serial);
                    key_value(ui, "Kind", device.kind.as_deref().unwrap_or("-"));
                    ui.end_row();
                    key_value(
                        ui,
                        "Manufacturer",
                        device.manufacturer.as_deref().unwrap_or("-"),
                    );
                    key_value(ui, "Model", device.model.as_deref().unwrap_or("-"));
                    ui.end_row();
                    key_value(
                        ui,
                        "Android",
                        device.android_version.as_deref().unwrap_or("-"),
                    );
                    key_value(
                        ui,
                        "API",
                        &device
                            .api_level
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    );
                    ui.end_row();
                    key_value(
                        ui,
                        "Architecture",
                        device.architecture.as_deref().unwrap_or("-"),
                    );
                    key_value(
                        ui,
                        "Resolution",
                        device.resolution.as_deref().unwrap_or("-"),
                    );
                    ui.end_row();
                    key_value(ui, "Boot", device.boot_state.as_deref().unwrap_or("-"));
                    key_value(ui, "AVD", device.avd.as_deref().unwrap_or("-"));
                    ui.end_row();
                    key_value(
                        ui,
                        "Port",
                        &device
                            .port
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    );
                    key_value(
                        ui,
                        "Battery",
                        &device
                            .battery_level
                            .map(|v| format!("{v}%"))
                            .unwrap_or_else(|| "-".to_string()),
                    );
                    ui.end_row();
                });
            if let Some(owner) = &device.owner {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Owned by {} · {} · {}",
                        owner.owner, owner.session_id, owner.state
                    ))
                    .size(11.0)
                    .color(theme::YELLOW),
                );
            } else {
                muted(ui, "Not currently owned by an AndroidForge session.");
            }
            if let Some(session) = app
                .sessions
                .value
                .sessions
                .iter()
                .find(|session| session.serial.as_deref() == Some(device.serial.as_str()))
            {
                ui.label(
                    RichText::new(format!(
                        "Current session: {} · {}",
                        session.id, session.owner
                    ))
                    .size(10.0)
                    .color(theme::MUTED),
                );
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                crate::components::copy_button(ui, &device.serial, "Copy serial");
                if let Some(path) = &device.owner {
                    let _ = path;
                }
            });
        });
}

#[allow(dead_code)]
fn _keep_data_state(_state: &DataState<crate::models::DeviceList>) {}
