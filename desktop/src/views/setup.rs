use eframe::egui;
use egui::{Color32, RichText, Vec2};

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
        ui.vertical(|ui| {
            let elapsed = app
                .setup
                .section_animation_started
                .map(|started| started.elapsed().as_secs_f32())
                .unwrap_or(1.0);
            let progress = (elapsed / 0.24).clamp(0.0, 1.0);
            if progress < 1.0 {
                let eased = 1.0 - (1.0 - progress).powi(3);
                ui.add_space((1.0 - eased) * 16.0);
                ui.set_opacity(0.2 + 0.8 * eased);
                ui.ctx().request_repaint();
            }
            match app.setup.section {
                SetupSection::Welcome => welcome(app, ui),
                SetupSection::Cli => cli(app, ui),
                SetupSection::Environment => environment(app, ui),
                SetupSection::Project => project(app, ui),
                SetupSection::Device => device(app, ui),
                SetupSection::Updates => updates(app, ui),
                SetupSection::Finish => finish(app, ui),
                SetupSection::Agent => agent_handoff(app, ui),
            }
            if progress < 1.0 {
                ui.set_opacity(1.0);
            }
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
            app.set_setup_section(sections[position - 1]);
        }
        ui.label(
            RichText::new(format!("Step {} of {}", position + 1, sections.len()))
                .size(9.5)
                .color(theme::MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if position + 1 < sections.len() {
                if primary_button(ui, "Continue").clicked() {
                    app.set_setup_section(sections[position + 1]);
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

    let dark = ui.style().visuals.dark_mode;
    let active_position = app.setup.section.position();
    let first_top = ui.cursor().top();
    let pitch = 30.0 + ui.spacing().item_spacing.y;
    let target_rect = app
        .setup
        .step_rects
        .iter()
        .find(|(position, _)| *position == active_position)
        .map(|(_, rect)| *rect)
        .unwrap_or_else(|| {
            egui::Rect::from_min_size(
                egui::pos2(
                    ui.cursor().left(),
                    first_top + 15.0 + active_position as f32 * pitch,
                ),
                Vec2::new(160.0, 30.0),
            )
        });
    let current = app.setup.highlight_y.unwrap_or(target_rect.center().y);
    let next = current + (target_rect.center().y - current) * 0.28;
    app.setup.highlight_y = Some(next);
    if (target_rect.center().y - next).abs() > 0.4 {
        ui.ctx().request_repaint();
    }
    let highlight = egui::Rect::from_min_max(
        egui::pos2(target_rect.left(), next - 15.0),
        egui::pos2(target_rect.right(), next + 15.0),
    );
    ui.painter().rect_filled(
        highlight,
        theme::ROW_RADIUS,
        if dark {
            theme::DARK_SURFACE_ALT
        } else {
            theme::LIGHT_SURFACE
        },
    );
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(highlight.left() + 2.0, highlight.top() + 7.0),
            Vec2::new(3.0, highlight.height() - 14.0),
        ),
        2.0,
        theme::accent(),
    );

    let mut rects = Vec::with_capacity(sections.len());
    for section in sections {
        let active = app.setup.section == *section;
        let done = section.position() < app.setup.section.position();
        let label = if done { "✓" } else { "•" };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(160.0, 30.0), egui::Sense::click());
        if !active && response.hovered() {
            let fill = if dark {
                Color32::from_rgb(24, 31, 39)
            } else {
                Color32::from_rgb(242, 245, 249)
            };
            ui.painter().rect_filled(rect, theme::ROW_RADIUS, fill);
        }
        ui.painter().text(
            rect.left_center() + Vec2::new(12.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{label}  {}", section.label()),
            egui::FontId::proportional(11.0),
            if active {
                theme::text_color(ui)
            } else {
                theme::nav_muted(ui)
            },
        );
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            app.set_setup_section(*section);
        }
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::Button,
                ui.is_enabled(),
                format!("Setup section: {}", section.label()),
            )
        });
        rects.push((section.position(), rect));
    }
    app.setup.step_rects = rects;

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
        app.set_setup_section(SetupSection::Cli);
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
            .corner_radius(theme::ROW_RADIUS)
            .inner_margin(egui::Margin::symmetric(12, 7))
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
            selectable_row(ui, false, theme::ROW_HEIGHT, |ui| {
                row_cell(ui, 80.0, |ui| {
                    status_pill(ui, &device.state, theme::status_color(&device.state));
                });
                row_cell(ui, 160.0, |ui| {
                    ui.label(RichText::new(&device.serial).monospace().size(10.5));
                });
                ui.label(
                    RichText::new(device.model.as_deref().unwrap_or("-"))
                        .size(10.0)
                        .color(theme::MUTED),
                );
            });
            ui.add_space(2.0);
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
    muted(
        ui,
        "Continue to the agent handoff page to copy a ready-to-send prompt with the full documentation paths.",
    );
    if secondary_button(ui, "Start over").clicked() {
        app.restart_onboarding();
    }
}

fn agent_handoff(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new("Setup complete").size(20.0).strong());
    ui.add_space(4.0);
    muted(
        ui,
        "Give the prompt below to your coding agent. It contains the full paths to the AndroidForge documentation, the CLI executable and the recommended agent loop.",
    );
    ui.add_space(12.0);

    let root = documentation_root(app);
    let docs = [
        ("Agent guide", "AGENT_GUIDE.md"),
        ("User guide and commands", "USER_GUIDE.md"),
        ("Capability coverage", "CAPABILITIES.md"),
        ("How it works", "README.md"),
        ("Exit codes and JSON", "EXIT_CODES.md"),
        ("Project adapters", "PROJECT_ADAPTER.md"),
    ];

    card(ui, |ui| {
        section_title(ui, "Documentation");
        muted(
            ui,
            "These are the files the agent should read before using AndroidForge.",
        );
        ui.add_space(4.0);
        for (label, file) in docs {
            let path = root.join(file);
            ui.horizontal(|ui| {
                status_pill(
                    ui,
                    if path.is_file() { "found" } else { "missing" },
                    if path.is_file() {
                        theme::GREEN
                    } else {
                        theme::YELLOW
                    },
                );
                ui.label(RichText::new(label).size(10.5).strong());
                ui.add(
                    egui::Label::new(
                        RichText::new(path.display().to_string())
                            .size(9.0)
                            .monospace()
                            .color(theme::MUTED),
                    )
                    .truncate(),
                )
                .on_hover_text(path.display().to_string());
            });
        }
        ui.add_space(4.0);
        if secondary_button(ui, "Open documentation folder").clicked() {
            let _ = open::that(&root);
        }
    });

    ui.add_space(theme::PAGE_GAP);
    let prompt = agent_prompt(app, &root);
    card(ui, |ui| {
        ui.horizontal(|ui| {
            section_title(ui, "Agent handoff prompt");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary_button(ui, "Copy agent prompt").clicked() {
                    ui.ctx().copy_text(prompt.clone());
                }
                if let Some(path) = cli_path(app)
                    && secondary_button(ui, "Copy CLI path").clicked()
                {
                    ui.ctx().copy_text(path);
                }
            });
        });
        muted(
            ui,
            "Paste this into your agent before asking it to test an app.",
        );
        ui.add_space(6.0);
        let dark = ui.style().visuals.dark_mode;
        egui::Frame::new()
            .fill(if dark {
                Color32::from_rgb(8, 11, 15)
            } else {
                Color32::from_rgb(246, 248, 251)
            })
            .stroke(egui::Stroke::new(1.0, theme::border_color(ui)))
            .corner_radius(theme::ROW_RADIUS)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                let mut display = prompt.clone();
                ui.add(
                    egui::TextEdit::multiline(&mut display)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(22)
                        .frame(egui::Frame::new())
                        .interactive(false),
                );
            });
    });

    ui.add_space(theme::PAGE_GAP);
    success_banner(
        ui,
        "You can finish setup now. The prompt will remain available from this page if you need it again.",
    );
}

fn cli_path(app: &App) -> Option<String> {
    app.cli
        .resolution
        .as_ref()
        .map(|resolution| resolution.path.display().to_string())
        .or_else(|| {
            app.cli
                .configured_path
                .as_ref()
                .map(|path| path.display().to_string())
        })
}

fn documentation_root(app: &App) -> std::path::PathBuf {
    let workspace = app.workspace();
    if workspace.join("AGENT_GUIDE.md").is_file() {
        return workspace;
    }
    if let Ok(executable) = std::env::current_exe() {
        for ancestor in executable.ancestors().skip(1).take(5) {
            if ancestor.join("AGENT_GUIDE.md").is_file() {
                return ancestor.to_path_buf();
            }
        }
    }
    workspace
}

fn agent_prompt(app: &App, root: &std::path::Path) -> String {
    let workspace = app.workspace().display().to_string();
    let cli = cli_path(app).unwrap_or_else(|| "not connected".to_string());
    let docs = |file: &str| root.join(file).display().to_string();
    format!(
        "AndroidForge is a CLI tool built for agents to test Android apps through an AVD emulator or a connected device. Use it instead of raw adb whenever it covers the operation.\n\n\
         Read these files first (full paths):\n\
         - Agent guide and safety rules: {agent_guide}\n\
         - Full user guide and command reference: {user_guide}\n\
         - Capability and command coverage: {capabilities}\n\
         - Overview and how it works: {readme}\n\
         - Exit codes and JSON envelopes: {exit_codes}\n\
         - Project adapters and configuration: {project_adapter}\n\n\
         CLI executable:\n{cli}\n\n\
         Workspace:\n{workspace}\n\n\
         The complete command list, every flag and the safety classes are in USER_GUIDE.md. CAPABILITIES.md lists every command area and its status.\n\n\
         Start with this loop:\n\
         androidforge doctor --json\n\
         androidforge agent brief --project \"{workspace}\" --json\n\
         androidforge session ensure --project \"{workspace}\" --avd <AVD_NAME> --owner <AGENT_NAME> --ttl 1h --json\n\
         androidforge ui observe --session <SESSION_ID> --json\n\
         androidforge ui tap --session <SESSION_ID> --text \"<LABEL>\" --wait-for-text \"<EXPECTED_TEXT>\" --receipt --json\n\
         androidforge report explain latest-failure --json\n\
         androidforge session release --session <SESSION_ID> --owner <AGENT_NAME> --json\n\n\
         Use emulators for destructive tests. Never pass --allow-physical-mutation for a physical device unless the user explicitly approves that exact operation. Keep the session owner so cleanup is possible.",
        agent_guide = docs("AGENT_GUIDE.md"),
        user_guide = docs("USER_GUIDE.md"),
        capabilities = docs("CAPABILITIES.md"),
        readme = docs("README.md"),
        exit_codes = docs("EXIT_CODES.md"),
        project_adapter = docs("PROJECT_ADAPTER.md"),
    )
}
