use eframe::egui;
use egui::{RichText, Vec2};

use crate::app::App;
use crate::components::*;
use crate::state::*;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    page_header(
        ui,
        "Setup",
        "Connect the CLI, verify the Android toolchain, choose a project and prove a device works.",
    );
    let sections = SetupSection::ALL;
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(180.0, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| step_rail(app, ui, &sections),
        );
        ui.separator();
        ui.add_space(14.0);
        ui.vertical(|ui| match app.setup.section {
            SetupSection::Welcome => welcome(app, ui),
            SetupSection::Cli => cli(app, ui),
            SetupSection::Environment => environment(app, ui),
            SetupSection::Project => project(app, ui),
            SetupSection::Device => device(app, ui),
            SetupSection::Updates => updates(app, ui),
            SetupSection::Finish => finish(app, ui),
        });
    });
}

/// Sticky wizard footer. It lives outside the page scroll area so Back,
/// Continue and Finish stay reachable at every window size.
pub fn footer(app: &mut App, ui: &mut egui::Ui) {
    let sections = SetupSection::ALL;
    let position = app.setup.section.position();
    ui.horizontal_centered(|ui| {
        if position > 0 && secondary_button(ui, "Back").clicked() {
            app.setup.section = sections[position - 1];
        }
        ui.label(
            RichText::new(format!("Step {} of {}", position + 1, sections.len()))
                .size(9.5)
                .color(theme::MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if position + 1 < sections.len() {
                if primary_button(ui, "Continue").clicked() {
                    app.setup.section = sections[position + 1];
                }
            } else if primary_button(ui, "Finish setup").clicked() {
                app.finish_onboarding(ui.ctx());
            }
        });
    });
}

fn step_rail(app: &mut App, ui: &mut egui::Ui, sections: &[SetupSection]) {
    ui.label(
        RichText::new("SETUP")
            .size(9.0)
            .strong()
            .color(theme::MUTED),
    );
    ui.add_space(6.0);
    for section in sections {
        let active = app.setup.section == *section;
        let done = section.position() < app.setup.section.position();
        let label = if done { "✓" } else { "•" };
        let response = ui.add(
            egui::Button::new(
                RichText::new(format!("{label}  {}", section.label()))
                    .size(11.0)
                    .color(if active { theme::TEXT } else { theme::MUTED }),
            )
            .fill(if active {
                if ui.style().visuals.dark_mode {
                    theme::DARK_SURFACE_ALT
                } else {
                    theme::LIGHT_SURFACE
                }
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(if active {
                egui::Stroke::new(1.0, theme::ACCENT)
            } else {
                egui::Stroke::NONE
            })
            .min_size(Vec2::new(160.0, 30.0)),
        );
        if response.clicked() {
            app.setup.section = *section;
        }
    }
    ui.add_space(10.0);
    divider(ui);
    let cli_ok = app.cli.info.is_some();
    let env_ok = app.doctor.value.checks.iter().all(|check| !check.blocking);
    let project_ok = app.project_path.is_some();
    for (label, ok) in [
        ("CLI", cli_ok),
        ("Environment", env_ok),
        ("Project", project_ok),
    ] {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if ok { "●" } else { "○" })
                    .size(9.0)
                    .color(if ok { theme::GREEN } else { theme::MUTED }),
            );
            ui.label(RichText::new(label).size(9.5).color(theme::MUTED));
        });
    }
}

fn welcome(app: &mut App, ui: &mut egui::Ui) {
    ui.label(
        RichText::new("A cockpit for the AndroidForge CLI")
            .size(20.0)
            .strong(),
    );
    ui.add_space(4.0);
    muted(
        ui,
        "This wizard checks the real environment, connects the CLI sidecar and proves a device. It never installs an APK or resets user data.",
    );
    ui.add_space(14.0);
    let environment_label = if app.doctor.value.status.is_empty() {
        "Unknown".to_string()
    } else {
        theme::status_label(&app.doctor.value.status)
    };
    ui.columns(4, |columns| {
        metric(
            &mut columns[0],
            "CLI",
            if app.cli.info.is_some() {
                "Connected"
            } else {
                "Missing"
            },
            if app.cli.info.is_some() {
                theme::GREEN
            } else {
                theme::RED
            },
            app.cli
                .info
                .as_ref()
                .map(|info| info.version.as_str())
                .unwrap_or("run the CLI step"),
        );
        metric(
            &mut columns[1],
            "Environment",
            &environment_label,
            theme::status_color(&app.doctor.value.status),
            "SDK, ADB, emulator, Java",
        );
        metric(
            &mut columns[2],
            "Projects",
            &app.projects.len().to_string(),
            theme::BLUE,
            "discovered adapters",
        );
        metric(
            &mut columns[3],
            "Devices",
            &app.devices.value.devices.len().to_string(),
            theme::BLUE,
            "connected now",
        );
    });
    ui.add_space(14.0);
    card(ui, |ui| {
        section_title(ui, "What setup will do");
        for line in [
            "1. Locate and validate androidforge.exe and its report schema.",
            "2. Convert doctor output into blocking, optional and recommended fixes.",
            "3. Choose or create a project adapter and workspace.",
            "4. Start or attach a session, then verify it is usable.",
            "5. Optionally check GitHub for a newer verified release.",
        ] {
            ui.label(RichText::new(line).size(10.5).color(theme::MUTED));
        }
    });
    ui.add_space(10.0);
    if primary_button(ui, "Start setup").clicked() {
        app.setup.section = SetupSection::Cli;
    }
}

fn cli(app: &mut App, ui: &mut egui::Ui) {
    section_title(ui, "CLI connection");
    if let Some(info) = &app.cli.info {
        success_banner(
            ui,
            &format!(
                "Connected to {} · report schema {}",
                info.version, info.report_schema
            ),
        );
        if let Some(resolution) = &app.cli.resolution {
            path_row(ui, &resolution.path.display().to_string());
            muted(ui, resolution.source.label());
        }
    } else {
        error_banner(
            ui,
            app.cli
                .error
                .as_deref()
                .unwrap_or("The AndroidForge CLI is not connected."),
        );
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if secondary_button(ui, "Choose CLI...").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("executable", &["exe"])
                .pick_file()
        {
            app.choose_cli_path(path, ui.ctx());
        }
        if secondary_button(ui, "Use bundled CLI").clicked() {
            app.clear_cli_path(ui.ctx());
        }
        if primary_button(ui, "Reconnect").clicked() {
            app.reconnect(ui.ctx());
        }
    });
    ui.add_space(8.0);
    card(ui, |ui| {
        section_title(ui, "Compatibility contract");
        muted(
            ui,
            "The desktop refuses a CLI whose version or report schema it does not understand. This prevents unsupported commands from running silently.",
        );
        ui.add_space(4.0);
        small(ui, "Desktop version");
        ui.label(
            RichText::new(env!("CARGO_PKG_VERSION"))
                .monospace()
                .size(11.0),
        );
    });
}

fn environment(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        section_title(ui, "Android development environment");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if secondary_button(ui, "Recheck").clicked() {
                app.manual_refresh(ui.ctx());
            }
            if secondary_button(ui, "Build repair plan").clicked() {
                app.refresh_repair_plan(ui.ctx());
            }
        });
    });
    if app.doctor.value.checks.is_empty() {
        empty_state(
            ui,
            "No doctor result yet",
            "Connect the CLI and recheck to inspect SDK, ADB, emulator and Java.",
        );
        return;
    }
    if app.setup.repair_plan.is_none()
        && !app.setup.repair_loading
        && app.setup.repair_error.is_none()
    {
        app.refresh_repair_plan(ui.ctx());
    }
    if !app.doctor.value.avds.is_empty() {
        ui.horizontal_wrapped(|ui| {
            small(ui, "AVDs:");
            for avd in &app.doctor.value.avds {
                status_pill(ui, avd, theme::MUTED);
            }
        });
        ui.add_space(4.0);
    }
    let checks = app.doctor.value.checks.clone();
    for check in checks {
        let color = theme::status_color(&check.status);
        egui::Frame::new()
            .fill(theme::surface_fill(ui))
            .stroke(egui::Stroke::new(1.0, theme::border_color(ui)))
            .corner_radius(7)
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    status_pill(ui, &check.status, color);
                    ui.label(RichText::new(&check.name).size(11.0).strong());
                    if check.blocking {
                        status_pill(ui, "blocking", theme::RED);
                    } else if check.required && check.status != "ok" {
                        status_pill(ui, "recommended", theme::YELLOW);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(path) = &check.path {
                            copy_button(ui, path, "Copy path");
                        }
                    });
                });
                ui.label(RichText::new(&check.detail).size(10.0).color(theme::MUTED));
                if let Some(impact) = &check.impact {
                    ui.label(RichText::new(impact).size(9.5).color(theme::MUTED));
                }
                if let Some(suggestion) = &check.suggestion {
                    ui.label(
                        RichText::new(format!("Fix: {suggestion}"))
                            .size(9.5)
                            .color(theme::BLUE),
                    );
                }
            });
        ui.add_space(4.0);
    }
    ui.add_space(6.0);
    repair_plan_panel(app, ui);
}

fn repair_plan_panel(app: &mut App, ui: &mut egui::Ui) {
    let plan = app.setup.repair_plan.clone();
    let loading = app.setup.repair_loading;
    let error = app.setup.repair_error.clone();
    if loading {
        ui.horizontal(|ui| {
            ui.spinner();
            muted(ui, "Building a read-only repair plan...");
        });
        return;
    }
    if let Some(error) = &error {
        error_banner(ui, error);
        return;
    }
    let Some(plan) = plan else {
        return;
    };
    card(ui, |ui| {
        ui.horizontal(|ui| {
            section_title(ui, "Repair plan");
            status_pill(ui, "read-only", theme::GREEN);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                copy_button(ui, &plan.generated_at, "Copy timestamp");
            });
        });
        small(
            ui,
            "AndroidForge detected these conditions but made no changes. Commands are suggestions; review them before running.",
        );
        if plan.issues.is_empty() {
            muted(ui, "No repairs are needed.");
            return;
        }
        ui.add_space(4.0);
        for issue in &plan.issues {
            ui.horizontal_wrapped(|ui| {
                let color = match issue.severity.as_str() {
                    "blocking" => theme::RED,
                    "recommended" => theme::YELLOW,
                    _ => theme::MUTED,
                };
                status_pill(ui, &issue.severity, color);
                ui.label(RichText::new(&issue.description).size(10.5).strong());
            });
            ui.label(
                RichText::new(format!("{} - {}", issue.detected, issue.reason))
                    .size(9.5)
                    .color(theme::MUTED),
            );
            if let Some(command) = issue.repair.command_display() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&command).size(9.5).monospace());
                    copy_button(ui, &command, "Copy command");
                });
            }
            if let Some(notes) = &issue.notes {
                ui.label(RichText::new(notes).size(9.0).color(theme::BLUE));
            }
            ui.add_space(4.0);
        }
    });
}

fn project(app: &mut App, ui: &mut egui::Ui) {
    section_title(ui, "Workspace and project adapter");
    ui.horizontal(|ui| {
        let mut workspace = app
            .settings
            .workspace
            .clone()
            .unwrap_or_else(|| app.workspace().display().to_string());
        ui.label(RichText::new("Workspace").size(10.0).color(theme::MUTED));
        ui.add(egui::TextEdit::singleline(&mut workspace).desired_width(300.0));
        if secondary_button(ui, "Browse...").clicked()
            && let Some(path) = rfd::FileDialog::new().pick_folder()
        {
            app.set_workspace(path, ui.ctx());
        }
    });
    ui.add_space(6.0);
    if app.projects.is_empty() {
        warning_banner(ui, "No project adapter found in the workspace.");
    } else {
        for project in app.projects.clone() {
            let selected = app.project_path.as_deref() == Some(project.root.as_path());
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(selected, RichText::new(project.display_name()).size(11.0))
                    .clicked()
                {
                    app.select_project(Some(project.root.clone()), ui.ctx());
                }
                ui.label(
                    RichText::new(project.package_label())
                        .size(10.0)
                        .color(theme::MUTED),
                );
                small(ui, project.root.display().to_string());
            });
        }
    }
    ui.add_space(10.0);
    card(ui, |ui| {
        section_title(ui, "Create a project adapter");
        ui.horizontal(|ui| {
            ui.label(RichText::new("Name").size(10.0).color(theme::MUTED));
            ui.add(egui::TextEdit::singleline(&mut app.setup.create_name).desired_width(160.0));
            ui.label(RichText::new("Package").size(10.0).color(theme::MUTED));
            ui.add(egui::TextEdit::singleline(&mut app.setup.create_package).desired_width(200.0));
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("AVD").size(10.0).color(theme::MUTED));
            ui.add(egui::TextEdit::singleline(&mut app.setup.create_avd).desired_width(200.0));
            if primary_button(ui, "Create adapter").clicked() {
                app.setup_create_project(ui.ctx());
            }
            if app.setup.create_busy {
                ui.spinner();
            }
        });
        if let Some(error) = &app.setup.create_error {
            error_banner(ui, error);
        }
    });
}

fn device(app: &mut App, ui: &mut egui::Ui) {
    section_title(ui, "Verify a device safely");
    muted(
        ui,
        "The test uses an existing ready session or starts a temporary one. It does not install an APK or reset data.",
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("AVD").size(10.0).color(theme::MUTED));
        egui::ComboBox::from_id_salt("setup-avd")
            .selected_text(if app.setup.create_avd.is_empty() {
                "Select an AVD".to_string()
            } else {
                app.setup.create_avd.clone()
            })
            .show_ui(ui, |ui| {
                for avd in &app.doctor.value.avds {
                    ui.selectable_value(&mut app.setup.create_avd, avd.clone(), avd);
                }
            });
        if primary_button(ui, "Test device").clicked() {
            app.setup_test_device(ui.ctx());
        }
        if app.setup.test_running {
            ui.spinner();
            ui.label("Testing...");
        }
    });
    ui.add_space(6.0);
    if let Some(ok) = app.setup.test_ok {
        if ok {
            success_banner(ui, "Device test passed.");
        } else {
            error_banner(ui, "Device test did not pass.");
        }
    }
    if let Some(error) = &app.setup.test_error {
        error_banner(ui, error);
    }
    for line in &app.setup.test_log {
        small(ui, line);
    }
    if let Some(session) = &app.setup.test_session {
        info_banner(ui, &format!("Test session: {session}"));
    }
    ui.add_space(8.0);
    section_label(ui, "Available devices");
    if app.devices.value.devices.is_empty() {
        muted(ui, "No device is currently connected.");
    } else {
        for device in &app.devices.value.devices {
            ui.horizontal(|ui| {
                status_pill(ui, &device.state, theme::status_color(&device.state));
                ui.label(RichText::new(&device.serial).monospace().size(10.5));
                ui.label(
                    RichText::new(device.model.as_deref().unwrap_or("-"))
                        .size(10.0)
                        .color(theme::MUTED),
                );
            });
        }
    }
}

fn updates(app: &mut App, ui: &mut egui::Ui) {
    section_title(ui, "Version and updates");
    kv_grid(
        ui,
        "setup-update-versions",
        &[
            ("Desktop", env!("CARGO_PKG_VERSION").to_string()),
            (
                "CLI",
                app.cli
                    .info
                    .as_ref()
                    .map(|info| info.version.clone())
                    .unwrap_or_else(|| "not connected".to_string()),
            ),
            (
                "Latest release",
                app.update
                    .value
                    .as_ref()
                    .map(|update| update.latest_version.clone())
                    .unwrap_or_else(|| "not checked".to_string()),
            ),
        ],
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if secondary_button(ui, "Check for updates").clicked() {
            app.check_updates(ui.ctx());
        }
        if secondary_button(ui, "Open releases").clicked() {
            app.open_releases_page();
        }
        if app.update.loading {
            ui.spinner();
        }
    });
    if let Some(error) = &app.update.error {
        error_banner(ui, error);
    }
    if let Some(update) = app.update.value.clone() {
        if update.update_available {
            warning_banner(
                ui,
                &format!(
                    "AndroidForge {} is available. You are running {}.",
                    update.latest_version, update.current_version
                ),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "Download verified CLI").clicked() {
                    app.stage_cli_update(ui.ctx());
                }
                if secondary_button(ui, "Open release page").clicked() {
                    let _ = open::that(&update.release.html_url);
                }
                if app.update.repair_busy {
                    ui.spinner();
                    ui.label("Downloading...");
                }
            });
            ui.collapsing("Release notes", |ui| {
                ui.label(RichText::new(&update.release.body).size(10.0));
            });
        } else {
            success_banner(ui, "The installed CLI is current.");
        }
    }
    if let Some(message) = &app.update.repair_message {
        info_banner(ui, message);
    }
    small(
        ui,
        "Downloads are explicit, verified against the published SHA-256 and never installed silently.",
    );
}

fn finish(app: &mut App, ui: &mut egui::Ui) {
    section_title(ui, "Setup summary");
    let cli_ok = app.cli.info.is_some();
    let project_ok = app.project_path.is_some();
    let device_ok = !app.devices.value.devices.is_empty()
        || app
            .sessions
            .value
            .sessions
            .iter()
            .any(|session| matches!(session.state.as_str(), "ready" | "attached"));
    for (label, ok, detail) in [
        (
            "CLI",
            cli_ok,
            app.cli
                .info
                .as_ref()
                .map(|info| info.version.clone())
                .unwrap_or_else(|| "not connected".to_string()),
        ),
        (
            "Project",
            project_ok,
            app.selected_project()
                .map(|project| project.display_name())
                .unwrap_or_else(|| "no adapter selected".to_string()),
        ),
        (
            "Device",
            device_ok,
            app.devices
                .value
                .devices
                .first()
                .map(|device| format!("{} · {}", device.serial, device.state))
                .unwrap_or_else(|| "no device connected".to_string()),
        ),
    ] {
        ui.horizontal(|ui| {
            status_pill(
                ui,
                if ok { "ready" } else { "missing" },
                if ok { theme::GREEN } else { theme::YELLOW },
            );
            ui.label(RichText::new(label).size(11.0).strong());
            ui.label(RichText::new(detail).size(10.0).color(theme::MUTED));
        });
    }
    ui.add_space(10.0);
    warning_banner(
        ui,
        "Setup does not pretend a missing device works. You can finish and configure the device later.",
    );
    ui.add_space(6.0);
    if primary_button(ui, "Finish setup").clicked() {
        app.finish_onboarding(ui.ctx());
    }
    if secondary_button(ui, "Start over").clicked() {
        app.restart_onboarding();
    }
}
