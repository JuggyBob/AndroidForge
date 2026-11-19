use eframe::egui;
use egui::RichText;

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
    let width = ui.available_width();
    let widths = proportional_widths(width, &[0.15, 0.28, 0.09, 0.08, 0.32, 0.08], 10.0);
    let serial_width = widths[0];
    let model_width = widths[1];
    let state_width = widths[2];
    let android_width = widths[3];
    let owner_width = widths[4];
    let select_width = widths[5];
    table_header(
        ui,
        &[
            ("Serial", serial_width),
            ("Model", model_width),
            ("State", state_width),
            ("Android", android_width),
            ("Owner", owner_width),
            ("", select_width),
        ],
    );
    for device in &devices {
        let selected = app.selected_device.as_deref() == Some(device.serial.as_str());
        let response = selectable_row(ui, selected, theme::ROW_HEIGHT, |ui| {
            row_cell(ui, serial_width, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(&device.serial).size(10.0).monospace())
                        .truncate(),
                )
                .on_hover_text(&device.serial);
            });
            row_cell(ui, model_width, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(device.model.as_deref().unwrap_or("-")).size(10.5),
                    )
                    .truncate(),
                );
            });
            row_cell(ui, state_width, |ui| {
                status_pill(ui, &device.state, theme::status_color(&device.state));
            });
            row_cell(ui, android_width, |ui| {
                ui.label(
                    RichText::new(device.android_version.as_deref().unwrap_or("-")).size(10.0),
                );
            });
            row_cell(ui, owner_width, |ui| {
                let owner = device
                    .owner
                    .as_ref()
                    .map(|owner| format!("{} · {}", owner.owner, owner.session_id))
                    .unwrap_or_else(|| "unowned".to_string());
                ui.add(egui::Label::new(RichText::new(owner).size(10.0)).truncate());
            });
            row_cell(ui, select_width, |ui| {
                if secondary_button(ui, if selected { "Clear" } else { "Select" }).clicked() {
                    select = Some(if selected {
                        None
                    } else {
                        Some(device.serial.clone())
                    });
                }
            });
        });
        if response.clicked() {
            select = Some(if selected {
                None
            } else {
                Some(device.serial.clone())
            });
        }
        ui.add_space(2.0);
    }
    if let Some(serial) = select {
        app.selected_device = serial;
    }
    if let Some(device) = app.selected_device_info() {
        ui.add_space(theme::PAGE_GAP);
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
    card(ui, |ui| {
        section_title(ui, "Device health");
        ui.horizontal(|ui| {
            if secondary_button(ui, "Read health").clicked() {
                app.refresh_health(&device.serial, ui.ctx());
            }
            if app.health_loading && app.health_serial.as_deref() == Some(device.serial.as_str()) {
                ui.spinner();
            }
        });
        if let Some(error) = &app.health_error {
            error_banner(ui, error);
        }
        if app.health_serial.as_deref() == Some(device.serial.as_str())
            && let Some(health) = &app.health
        {
            kv_grid(
                ui,
                "device-health",
                &[
                    (
                        "ADB state",
                        health["adb_state"].as_str().unwrap_or("-").to_string(),
                    ),
                    (
                        "Battery",
                        format!(
                            "{}% {}",
                            health["battery"]["level_percent"]
                                .as_u64()
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "?".to_string()),
                            health["battery"]["status"].as_str().unwrap_or("unknown")
                        ),
                    ),
                    (
                        "Battery temperature",
                        health["battery"]["temperature_c"]
                            .as_f64()
                            .map(|value| format!("{value:.1} C"))
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                    (
                        "Thermal",
                        health["thermal"]["status"]
                            .as_str()
                            .unwrap_or("unknown")
                            .to_string(),
                    ),
                    (
                        "Data free",
                        health["storage"]["available_bytes"]
                            .as_u64()
                            .map(|value| format!("{} MiB", value / (1024 * 1024)))
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                    (
                        "Memory available",
                        health["memory"]["available_bytes"]
                            .as_u64()
                            .map(|value| format!("{} MiB", value / (1024 * 1024)))
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                    (
                        "Network",
                        format!(
                            "wifi={} airplane={} interface={}",
                            health["network"]["wifi_enabled"]
                                .as_bool()
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "unknown".to_string()),
                            health["network"]["airplane_mode"]
                                .as_bool()
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "unknown".to_string()),
                            health["network"]["active_interface"]
                                .as_str()
                                .unwrap_or("unknown")
                        ),
                    ),
                    (
                        "Screen",
                        format!(
                            "interactive={} locked={}",
                            health["screen"]["interactive"]
                                .as_bool()
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "unknown".to_string()),
                            health["screen"]["locked"]
                                .as_bool()
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "unknown".to_string())
                        ),
                    ),
                    (
                        "Uptime",
                        health["uptime_seconds"]
                            .as_u64()
                            .map(|value| format!("{value}s"))
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                ],
            );
            if let Some(warnings) = health["warnings"].as_array() {
                for warning in warnings.iter().filter_map(|value| value.as_str()) {
                    warning_banner(ui, warning);
                }
            }
        }
    });
    ui.add_space(theme::PAGE_GAP);
    card(ui, |ui| {
        section_title(ui, "Wireless ADB");
        ui.horizontal(|ui| {
            ui.label(RichText::new("Endpoint").size(9.5).color(theme::MUTED));
            ui.add(
                egui::TextEdit::singleline(&mut app.wireless_endpoint)
                    .desired_width(220.0)
                    .hint_text("192.168.1.20:37521"),
            );
            ui.label(RichText::new("Pairing code").size(9.5).color(theme::MUTED));
            ui.add(
                egui::TextEdit::singleline(&mut app.wireless_code)
                    .password(true)
                    .desired_width(90.0)
                    .hint_text("6 digits"),
            );
        });
        ui.horizontal(|ui| {
            if secondary_button(ui, "Pair").clicked() && !app.wireless_loading {
                let code = std::mem::take(&mut app.wireless_code);
                app.wireless_pair(app.wireless_endpoint.trim().to_string(), code, ui.ctx());
            }
            if secondary_button(ui, "Connect").clicked() && !app.wireless_loading {
                app.wireless_connect(app.wireless_endpoint.trim().to_string(), ui.ctx());
            }
            if secondary_button(ui, "Disconnect").clicked() && !app.wireless_loading {
                app.wireless_disconnect(app.wireless_endpoint.trim().to_string(), ui.ctx());
            }
            if app.wireless_loading {
                ui.spinner();
            }
        });
        small(
            ui,
            "The pairing code is held only in memory, sent to the CLI through stdin and never logged. The pairing port is not the connection port.",
        );
        if let Some(error) = &app.wireless_error {
            error_banner(ui, error);
        }
        if let Some(result) = &app.wireless_result {
            success_banner(
                ui,
                &format!(
                    "{} {} · serial {}",
                    result["action"].as_str().unwrap_or("wireless"),
                    result["endpoint"].as_str().unwrap_or("-"),
                    result["serial"].as_str().unwrap_or("-")
                ),
            );
        }
    });
}
