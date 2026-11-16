use std::time::Duration;

use eframe::egui;
use egui::{RichText, Stroke};

use crate::app::App;
use crate::components::*;
use crate::state::*;
use crate::theme;

pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    settings_dialog(app, ctx);
    install_dialog(app, ctx);
    session_start_dialog(app, ctx);
    attach_dialog(app, ctx);
    run_dialog(app, ctx);
    confirm_dialog(app, ctx);
    error_dialog(app, ctx);
}

fn settings_dialog(app: &mut App, ctx: &egui::Context) {
    if !app.settings_open {
        return;
    }
    let mut open = true;
    let mut apply = false;
    let mut reconnect = false;
    let mut choose_cli = false;
    let mut choose_workspace = false;
    let mut cli_path = app.settings.cli_path.clone();
    let mut workspace = app.settings.workspace.clone();
    let mut owner = app.settings.owner.clone();
    let mut theme_mode = app.settings.theme;
    egui::Window::new("Settings")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(620.0)
        .min_width(540.0)
        .max_width(760.0)
        .max_height(680.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                section_label(ui, "Connection");
                ui.horizontal(|ui| {
                    ui.label(RichText::new("CLI").size(10.0).color(theme::MUTED));
                    let mut text = cli_path.clone().unwrap_or_default();
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(360.0)
                            .hint_text("androidforge.exe"),
                    );
                    if response.changed() {
                        cli_path = if text.trim().is_empty() {
                            None
                        } else {
                            Some(text.trim().to_string())
                        };
                    }
                    if secondary_button(ui, "Browse...").clicked() {
                        choose_cli = true;
                    }
                    if cli_path.is_some() && ui.small_button("Clear").clicked() {
                        cli_path = None;
                        apply = true;
                    }
                });
                if let Some(info) = &app.cli.info {
                    success_banner(
                        ui,
                        &format!(
                            "Connected · CLI {} · report schema {}",
                            info.version, info.report_schema
                        ),
                    );
                } else if let Some(error) = &app.cli.error {
                    error_banner(ui, error);
                }
                if let Some(resolution) = &app.cli.resolution {
                    path_row(ui, &resolution.path.display().to_string());
                    small(ui, resolution.source.label());
                }
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Workspace").size(10.0).color(theme::MUTED));
                    let mut text = workspace.clone().unwrap_or_default();
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(360.0)
                            .hint_text("workspace directory"),
                    );
                    if response.changed() {
                        workspace = if text.trim().is_empty() {
                            None
                        } else {
                            Some(text.trim().to_string())
                        };
                    }
                    if secondary_button(ui, "Browse...").clicked() {
                        choose_workspace = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Owner").size(10.0).color(theme::MUTED));
                    ui.add(egui::TextEdit::singleline(&mut owner).desired_width(220.0));
                    ui.label(RichText::new("Theme").size(10.0).color(theme::MUTED));
                    egui::ComboBox::from_id_salt("settings-theme")
                        .selected_text(theme_mode.label())
                        .show_ui(ui, |ui| {
                            for mode in [ThemeMode::System, ThemeMode::Dark, ThemeMode::Light] {
                                ui.selectable_value(&mut theme_mode, mode, mode.label());
                            }
                        });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if primary_button(ui, "Apply and reconnect").clicked() {
                        apply = true;
                    }
                    if secondary_button(ui, "Reconnect now").clicked() {
                        reconnect = true;
                    }
                    if ghost_button(ui, "Restart setup").clicked() {
                        app.restart_onboarding();
                    }
                });
                divider(ui);
                section_label(ui, "Updates");
                kv_grid(
                    ui,
                    "settings-versions",
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
                if let Some(update) = &app.update.value
                    && update.update_available
                {
                    warning_banner(
                        ui,
                        &format!(
                            "AndroidForge {} is available; current CLI is {}.",
                            update.latest_version, update.current_version
                        ),
                    );
                    if secondary_button(ui, "Download verified CLI").clicked() {
                        app.stage_cli_update(ui.ctx());
                    }
                }
                if let Some(message) = &app.update.repair_message {
                    info_banner(ui, message);
                }
                divider(ui);
                section_label(ui, "About");
                small(
                    ui,
                    "AndroidForge Desktop is a native egui/eframe client. The CLI remains authoritative; this app never reimplements Android operations.",
                );
                small(ui, format!("Desktop version {}", env!("CARGO_PKG_VERSION")));
            });
        });
    if choose_cli
        && let Some(path) = rfd::FileDialog::new()
            .add_filter("executable", &["exe"])
            .pick_file()
    {
        cli_path = Some(path.display().to_string());
        apply = true;
    }
    if choose_workspace && let Some(path) = rfd::FileDialog::new().pick_folder() {
        workspace = Some(path.display().to_string());
        apply = true;
    }
    if apply {
        app.settings.cli_path = cli_path;
        app.settings.workspace = workspace;
        app.settings.owner = if owner.trim().is_empty() {
            "Desktop User".to_string()
        } else {
            owner.trim().to_string()
        };
        app.settings.theme = theme_mode;
        app.reconnect(ctx);
        app.refresh_projects(ctx);
        app.settings_open = false;
    } else if reconnect {
        app.reconnect(ctx);
    }
    if !open {
        app.settings_open = false;
    }
}

fn confirm_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(action) = app.confirm.clone() else {
        return;
    };
    let mut open = true;
    let mut confirmed = false;
    let mut cancelled = false;
    egui::Window::new(action.title())
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(520.0)
        .show(ctx, |ui| {
            match &action {
                ConfirmAction::ReleaseSession(session) => {
                    ui.label("Release AndroidForge ownership without stopping the emulator.");
                    session_summary(ui, session);
                }
                ConfirmAction::StopSession(session) => {
                    error_banner(
                        ui,
                        "This stops the emulator and releases the session. Unsaved state may be lost.",
                    );
                    session_summary(ui, session);
                }
                ConfirmAction::RestartSession(session) => {
                    warning_banner(ui, "This restarts the emulator. Runtime state is lost.");
                    session_summary(ui, session);
                }
                ConfirmAction::Install { apk_path, fresh } => {
                    ui.label("Install this APK on the selected session?");
                    ui.label(RichText::new(apk_path.display().to_string()).monospace().size(9.5));
                    if let Some(session) = app.selected_session_info() {
                        kv_grid(
                            ui,
                            "install-target",
                            &[
                                ("Target", session.serial.unwrap_or_else(|| "-".to_string())),
                                ("Session", session.id),
                                ("Owner", session.owner),
                            ],
                        );
                    }
                    if *fresh {
                        error_banner(ui, "Fresh install replaces the app and deletes its data.");
                    }
                }
                ConfirmAction::Fixture { name, yes } => {
                    ui.label(format!("Run fixture '{name}' on the selected session?"));
                    project_target_summary(app, ui);
                    if *yes {
                        warning_banner(
                            ui,
                            "This flow may modify app or device state. Destructive steps were confirmed.",
                        );
                    }
                }
                ConfirmAction::Conformance { yes } => {
                    ui.label("Run the configured conformance flow on the selected session?");
                    project_target_summary(app, ui);
                    if *yes {
                        warning_banner(
                            ui,
                            "The flow may run a setup fixture or modify app/device state.",
                        );
                    }
                }
                ConfirmAction::Hook { name } => {
                    ui.label(format!("Run hook '{name}' on the selected session?"));
                    project_target_summary(app, ui);
                    warning_banner(
                        ui,
                        "Hooks are project-authored commands or device triggers. Review the adapter before running.",
                    );
                }
                ConfirmAction::Cleanup(request) => {
                    error_banner(
                        ui,
                        "This permanently removes matching session tombstones and run artifacts.",
                    );
                    kv_grid(
                        ui,
                        "cleanup-confirm",
                        &[
                            ("Older than", format!("{} days", request.older_days)),
                            ("Keep latest", format!("{} runs", request.keep_last)),
                            (
                                "Sessions",
                                if request.sessions { "included" } else { "not included" }.to_string(),
                            ),
                            (
                                "Runs",
                                if request.runs { "included" } else { "not included" }.to_string(),
                            ),
                        ],
                    );
                    muted(ui, "The preview above is the authoritative list; inspect it before confirming.");
                }
                ConfirmAction::InstallCli {
                    staged,
                    target,
                    version,
                } => {
                    warning_banner(
                        ui,
                        &format!("Replace the current CLI with verified AndroidForge {version}?"),
                    );
                    kv_grid(
                        ui,
                        "cli-repair",
                        &[
                            ("Staged", staged.display().to_string()),
                            ("Target", target.display().to_string()),
                            ("Version", version.clone()),
                        ],
                    );
                    muted(
                        ui,
                        "The previous executable is kept beside the target and restored if validation fails.",
                    );
                }
                ConfirmAction::CancelOperation => {
                    error_banner(
                        ui,
                        "This terminates the AndroidForge child process tree, not the emulator session.",
                    );
                    ui.label("The final CLI report may be incomplete. The device and session remain intact.");
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let label = action.confirm_label();
                let button = if action.destructive() {
                    danger_button(ui, label)
                } else {
                    primary_button(ui, label)
                };
                if button.clicked() {
                    confirmed = true;
                }
                if secondary_button(ui, "Cancel").clicked() {
                    cancelled = true;
                }
            });
        });
    if confirmed {
        app.execute_confirmation(ctx);
        app.confirm = None;
    } else if cancelled || !open {
        app.confirm = None;
    }
}

fn install_dialog(app: &mut App, ctx: &egui::Context) {
    if !app.install.open {
        return;
    }
    let mut open = true;
    let mut window_open = true;
    let mut close = false;
    egui::Window::new("Install APK")
        .open(&mut window_open)
        .collapsible(false)
        .resizable(true)
        .default_width(700.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if secondary_button(ui, "Choose APK...").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Android package", &["apk"])
                        .pick_file()
                {
                    app.choose_apk(path, ui.ctx());
                }
                if let Some(path) = &app.install.apk_path {
                    truncated_text(ui, &path.display().to_string(), 70);
                }
                if app.install.loading {
                    ui.spinner();
                }
            });
            if let Some(error) = &app.install.error {
                error_banner(ui, error);
            }
            if let Some(info) = &app.install.info {
                ui.add_space(4.0);
                card(ui, |ui| {
                    ui.columns(2, |columns| {
                        kv_grid(
                            &mut columns[0],
                            "apk-left",
                            &[
                                (
                                    "Package",
                                    info.package.clone().unwrap_or_else(|| "-".to_string()),
                                ),
                                (
                                    "Version",
                                    info.version_name.clone().unwrap_or_else(|| "-".to_string()),
                                ),
                                (
                                    "Version code",
                                    info.version_code
                                        .map(|value| value.to_string())
                                        .unwrap_or_else(|| "-".to_string()),
                                ),
                                (
                                    "Size",
                                    format!("{:.1} MB", info.size_bytes as f64 / 1_048_576.0),
                                ),
                            ],
                        );
                        kv_grid(
                            &mut columns[1],
                            "apk-right",
                            &[
                                ("SHA-256", info.sha256.clone()),
                                (
                                    "Certificate",
                                    info.signature
                                        .cert_sha256
                                        .clone()
                                        .unwrap_or_else(|| "-".to_string()),
                                ),
                                (
                                    "Signer",
                                    info.signature
                                        .signer_dn
                                        .clone()
                                        .unwrap_or_else(|| "-".to_string()),
                                ),
                            ],
                        );
                    });
                });
                if let Some(mismatch) = &app.install.mismatch {
                    error_banner(ui, mismatch);
                }
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Target").size(10.0).color(theme::MUTED));
                let mut selected = app.selected_session.clone();
                let label = app
                    .selected_session_info()
                    .map(|session| {
                        format!(
                            "{} · {}",
                            session.serial.as_deref().unwrap_or("-"),
                            session.id
                        )
                    })
                    .unwrap_or_else(|| "Select a session".to_string());
                egui::ComboBox::from_id_salt("install-session")
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for session in &app.sessions.value.sessions {
                            if matches!(session.state.as_str(), "ready" | "attached")
                                && ui
                                    .selectable_label(
                                        app.selected_session.as_deref()
                                            == Some(session.id.as_str()),
                                        format!(
                                            "{} · {} · {}",
                                            session.serial.as_deref().unwrap_or("-"),
                                            session.owner,
                                            session.id
                                        ),
                                    )
                                    .clicked()
                            {
                                selected = Some(session.id.clone());
                            }
                        }
                    });
                if selected != app.selected_session {
                    app.selected_session = selected;
                }
            });
            ui.checkbox(&mut app.install.fresh, "Fresh install (deletes app data)");
            if app.install.fresh {
                warning_banner(
                    ui,
                    "Fresh install is destructive and requires confirmation.",
                );
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let can_preview = app.install.info.is_some()
                    && app.selected_session.is_some()
                    && !app.install.preview_loading;
                if action_button(ui, "Dry run", can_preview, "Preview install constraints")
                    .clicked()
                {
                    app.preview_install(ui.ctx());
                }
                let can_install = app.install.info.is_some()
                    && app.selected_session.is_some()
                    && app.install.mismatch.is_none()
                    && !app.active_operation();
                if action_button(
                    ui,
                    "Install...",
                    can_install,
                    "Install and verify the APK identity",
                )
                .clicked()
                    && let Some(path) = app.install.apk_path.clone()
                {
                    app.install.open = false;
                    close = true;
                    app.confirm(ConfirmAction::Install {
                        apk_path: path,
                        fresh: app.install.fresh,
                    });
                }
                if app.install.preview_loading {
                    ui.spinner();
                }
                if secondary_button(ui, "Close").clicked() {
                    close = true;
                }
            });
            if let Some(error) = &app.install.preview_error {
                error_banner(ui, error);
            }
            if let Some(preview) = &app.install.preview {
                ui.collapsing("Dry-run preview", |ui| {
                    let mut text = serde_json::to_string_pretty(preview).unwrap_or_default();
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .font(egui::FontId::monospace(9.5))
                            .desired_rows(8)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
            }
            if let Some(build) = &app.install.result {
                success_banner(
                    ui,
                    &format!(
                        "Installed {} {} · verified={} · {}",
                        build.package.as_deref().unwrap_or("-"),
                        build.version_name.as_deref().unwrap_or("-"),
                        build.verified,
                        build.installed_apk_sha256.as_deref().unwrap_or("-")
                    ),
                );
            }
        });
    if close || !window_open {
        open = false;
    }
    if !open {
        app.install.open = false;
    }
}

fn session_start_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(mut form) = app.session_form.take() else {
        return;
    };
    let mut open = true;
    let mut window_open = true;
    let mut close = false;
    let mut start = false;
    let avds = app.doctor.value.avds.clone();
    egui::Window::new("Start emulator session")
        .open(&mut window_open)
        .collapsible(false)
        .resizable(false)
        .default_width(500.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("AVD").size(10.0).color(theme::MUTED));
                egui::ComboBox::from_id_salt("start-avd")
                    .selected_text(if form.avd.is_empty() {
                        "Select an AVD".to_string()
                    } else {
                        form.avd.clone()
                    })
                    .show_ui(ui, |ui| {
                        for avd in &avds {
                            ui.selectable_value(&mut form.avd, avd.clone(), avd);
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Port").size(10.0).color(theme::MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut form.port)
                        .desired_width(90.0)
                        .hint_text("auto"),
                );
                ui.label(
                    RichText::new("even console port; ADB uses port + 1")
                        .size(9.0)
                        .color(theme::MUTED),
                );
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Owner").size(10.0).color(theme::MUTED));
                ui.add(egui::TextEdit::singleline(&mut form.owner).desired_width(220.0));
            });
            ui.checkbox(&mut form.headless, "Headless");
            ui.checkbox(&mut form.prepare, "Prepare device settings");
            ui.checkbox(&mut form.reuse, "Reuse a running emulator for this AVD");
            if form.avd.is_empty() {
                warning_banner(ui, "Choose an AVD to continue.");
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if action_button(
                    ui,
                    "Start session",
                    !form.avd.is_empty()
                        && !form.owner.trim().is_empty()
                        && !app.active_operation(),
                    "Start a managed emulator session",
                )
                .clicked()
                {
                    start = true;
                }
                if secondary_button(ui, "Cancel").clicked() {
                    close = true;
                }
            });
        });
    if close || !window_open {
        open = false;
    }
    if start {
        let mut args = vec![
            "session".to_string(),
            "start".to_string(),
            "--avd".to_string(),
            form.avd.clone(),
            "--owner".to_string(),
            form.owner.clone(),
        ];
        if !form.port.trim().is_empty() {
            args.push("--port".to_string());
            args.push(form.port.trim().to_string());
        }
        if form.headless {
            args.push("--headless".to_string());
        }
        if form.prepare {
            args.push("--prepare".to_string());
        }
        if form.reuse {
            args.push("--reuse".to_string());
        }
        app.start_operation(
            ctx,
            format!("Start {}", form.avd),
            OperationKind::SessionStart,
            app.invocation(args),
        );
        app.session_form = None;
    } else if open {
        app.session_form = Some(form);
    }
}

fn attach_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(mut form) = app.attach_form.take() else {
        return;
    };
    let mut open = true;
    let mut window_open = true;
    let mut close = false;
    let mut attach = false;
    egui::Window::new("Attach to device")
        .open(&mut window_open)
        .collapsible(false)
        .resizable(false)
        .default_width(500.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Serial").size(10.0).color(theme::MUTED));
                egui::ComboBox::from_id_salt("attach-serial")
                    .selected_text(if form.serial.is_empty() {
                        "Select a device".to_string()
                    } else {
                        form.serial.clone()
                    })
                    .show_ui(ui, |ui| {
                        for device in &app.devices.value.devices {
                            ui.selectable_value(
                                &mut form.serial,
                                device.serial.clone(),
                                format!(
                                    "{} · {} · {}",
                                    device.serial,
                                    device.model.as_deref().unwrap_or("-"),
                                    device.state
                                ),
                            );
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Owner").size(10.0).color(theme::MUTED));
                ui.add(egui::TextEdit::singleline(&mut form.owner).desired_width(220.0));
            });
            ui.checkbox(&mut form.prepare, "Prepare device settings");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if action_button(
                    ui,
                    "Attach",
                    !form.serial.is_empty()
                        && !form.owner.trim().is_empty()
                        && !app.active_operation(),
                    "Attach to the selected device",
                )
                .clicked()
                {
                    attach = true;
                }
                if secondary_button(ui, "Cancel").clicked() {
                    close = true;
                }
            });
        });
    if close || !window_open {
        open = false;
    }
    if attach {
        let mut args = vec![
            "session".to_string(),
            "attach".to_string(),
            "--serial".to_string(),
            form.serial.clone(),
            "--owner".to_string(),
            form.owner.clone(),
        ];
        if form.prepare {
            args.push("--prepare".to_string());
        }
        app.start_operation(
            ctx,
            format!("Attach {}", form.serial),
            OperationKind::SessionAttach,
            app.invocation(args),
        );
        app.attach_form = None;
    } else if open {
        app.attach_form = Some(form);
    }
}

fn run_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(mut form) = app.run_form.take() else {
        return;
    };
    let mut open = true;
    let mut window_open = true;
    let mut close = false;
    let mut run = false;
    let fixtures = app
        .project
        .as_ref()
        .map(|bundle| bundle.fixtures.clone())
        .unwrap_or_default();
    let conformance = app
        .project
        .as_ref()
        .and_then(|bundle| bundle.config.conformance.flow.clone());
    egui::Window::new("Run project flow")
        .open(&mut window_open)
        .collapsible(false)
        .resizable(false)
        .default_width(560.0)
        .show(ctx, |ui| {
            project_target_summary(app, ui);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut form.conformance, false, "Fixture");
                ui.selectable_value(&mut form.conformance, true, "Conformance");
            });
            if form.conformance {
                match &conformance {
                    Some(flow) => {
                        ui.label(RichText::new(format!("Flow: {flow}")).size(10.0));
                    }
                    None => {
                        warning_banner(ui, "No conformance flow is configured.");
                    }
                }
            } else if fixtures.is_empty() {
                warning_banner(ui, "No fixtures are configured for this project.");
            } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Fixture").size(10.0).color(theme::MUTED));
                    egui::ComboBox::from_id_salt("run-fixture")
                        .selected_text(
                            form.fixture
                                .clone()
                                .unwrap_or_else(|| "Select a fixture".to_string()),
                        )
                        .show_ui(ui, |ui| {
                            for fixture in &fixtures {
                                if ui
                                    .selectable_label(
                                        form.fixture.as_deref() == Some(fixture.name.as_str()),
                                        &fixture.name,
                                    )
                                    .on_hover_text(
                                        fixture.description.as_deref().unwrap_or("No description"),
                                    )
                                    .clicked()
                                {
                                    form.fixture = Some(fixture.name.clone());
                                }
                            }
                        });
                });
                if let Some(name) = &form.fixture
                    && let Some(fixture) = fixtures.iter().find(|fixture| &fixture.name == name)
                {
                    muted(
                        ui,
                        fixture.description.as_deref().unwrap_or("No description"),
                    );
                }
            }
            ui.add_space(4.0);
            ui.checkbox(
                &mut form.yes,
                "I understand this may modify app or device state and confirm destructive steps",
            );
            ui.add_space(4.0);
            let target_ready = app.selected_session.is_some();
            let can_run = if form.conformance {
                conformance.is_some() && form.yes && target_ready && !app.active_operation()
            } else {
                form.fixture.is_some() && form.yes && target_ready && !app.active_operation()
            };
            ui.horizontal(|ui| {
                if action_button(ui, "Review and run", can_run, "Run the selected flow").clicked() {
                    run = true;
                }
                if secondary_button(ui, "Cancel").clicked() {
                    close = true;
                }
            });
        });
    if close || !window_open {
        open = false;
    }
    if run {
        if form.conformance {
            app.confirm(ConfirmAction::Conformance { yes: form.yes });
        } else if let Some(name) = form.fixture.clone() {
            app.confirm(ConfirmAction::Fixture {
                name,
                yes: form.yes,
            });
        }
        app.run_form = None;
    } else if open {
        app.run_form = Some(form);
    }
}

fn error_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(error) = app.error_dialog.clone() else {
        return;
    };
    let mut open = true;
    let mut window_open = true;
    egui::Window::new("Operation details")
        .open(&mut window_open)
        .collapsible(false)
        .resizable(true)
        .default_width(660.0)
        .show(ctx, |ui| {
            error_banner(ui, &format!("{}: {}", error.code, error.message));
            if let Some(cause) = &error.cause {
                ui.label(RichText::new(cause).size(10.0).color(theme::MUTED));
            }
            if !error.suggestions.is_empty() {
                section_label(ui, "Next steps");
                for suggestion in &error.suggestions {
                    ui.label(RichText::new(format!("→ {suggestion}")).size(10.0));
                }
            }
            if !error.context.is_empty() {
                ui.collapsing("Context", |ui| {
                    let mut text = serde_json::to_string_pretty(&serde_json::Value::Object(
                        error.context.clone().into_iter().collect(),
                    ))
                    .unwrap_or_default();
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .font(egui::FontId::monospace(9.5))
                            .desired_rows(7)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
            }
            if let Some(raw) = &error.raw_json {
                ui.collapsing("Raw JSON", |ui| {
                    let text = raw.clone();
                    code_block(ui, &text);
                    if ui.small_button("Copy raw JSON").clicked() {
                        ui.ctx().copy_text(text.clone());
                    }
                });
            }
            if !error.artifacts.is_empty() {
                section_label(ui, "Artifacts");
                for artifact in &error.artifacts {
                    ui.horizontal(|ui| {
                        small(ui, &artifact.kind);
                        ui.label(RichText::new(&artifact.path).size(9.5).monospace());
                    });
                }
            }
            ui.add_space(4.0);
            if secondary_button(ui, "Close").clicked() {
                open = false;
            }
        });
    if !window_open {
        open = false;
    }
    if !open {
        app.error_dialog = None;
    }
}

pub fn toasts(app: &mut App, ctx: &egui::Context) {
    app.toasts.retain(|toast| !toast.expired());
    if app.toasts.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::unique("androidforge-toasts"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0))
        .interactable(false)
        .show(ctx, |ui| {
            for toast in &app.toasts {
                let color = match toast.level {
                    ToastLevel::Info => theme::BLUE,
                    ToastLevel::Success => theme::GREEN,
                    ToastLevel::Warning => theme::YELLOW,
                    ToastLevel::Error => theme::RED,
                };
                egui::Frame::new()
                    .fill(if ctx.style_of(ctx.theme()).visuals.dark_mode {
                        theme::DARK_SURFACE
                    } else {
                        theme::LIGHT_PANEL
                    })
                    .stroke(Stroke::new(1.0, color))
                    .corner_radius(7)
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.label(RichText::new(&toast.text).size(10.5).color(color));
                    });
                ui.add_space(4.0);
            }
        });
    ctx.request_repaint_after(Duration::from_secs(1));
}
