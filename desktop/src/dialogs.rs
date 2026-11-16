use std::time::Duration;

use eframe::egui;
use egui::{RichText, Stroke};

use crate::app::App;
use crate::components::{
    error_banner, key_value, project_target_summary, session_summary, settings_path_row,
    truncated_text,
};
use crate::state::*;
use crate::theme;

pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    settings_dialog(app, ctx);
    install_dialog(app, ctx);
    session_start_dialog(app, ctx);
    attach_dialog(app, ctx);
    run_dialog(app, ctx);
    // Confirmations and errors must be drawn above every other dialog.
    confirm_dialog(app, ctx);
    error_dialog(app, ctx);
}

fn settings_dialog(app: &mut App, ctx: &egui::Context) {
    if !app.settings_open {
        return;
    }
    let mut open = true;
    let mut window_open = true;
    let mut cli_path = app.settings.cli_path.clone();
    let mut workspace = app.settings.workspace.clone();
    let mut owner = app.settings.owner.clone();
    let mut theme_mode = app.settings.theme;
    let mut apply = false;
    let mut reconnect = false;
    egui::Window::new("Settings")
        .open(&mut window_open)
        .collapsible(false)
        .resizable(false)
        .default_width(620.0)
        .show(ctx, |ui| {
            ui.label(RichText::new("CLI connection").strong());
            ui.add_space(4.0);
            settings_path_row(ui, "Executable", &mut cli_path, false);
            ui.add_space(3.0);
            settings_path_row(ui, "Workspace", &mut workspace, true);
            ui.add_space(6.0);
            if let Some(info) = &app.cli.info {
                ui.label(
                    RichText::new(format!(
                        "Connected: {} · report schema {}",
                        info.version, info.report_schema
                    ))
                    .size(11.0)
                    .color(theme::GREEN),
                );
                if let Some(resolution) = &app.cli.resolution {
                    ui.label(
                        RichText::new(format!(
                            "{}: {}",
                            resolution.source.label(),
                            resolution.path.display()
                        ))
                        .size(10.0)
                        .color(theme::MUTED),
                    );
                }
            } else if let Some(error) = &app.cli.error {
                error_banner(ui, error);
            }
            ui.add_space(8.0);
            ui.separator();
            ui.label(RichText::new("Desktop").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Owner").size(11.0).color(theme::MUTED));
                ui.add(egui::TextEdit::singleline(&mut owner).desired_width(220.0));
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Theme").size(11.0).color(theme::MUTED));
                egui::ComboBox::from_id_salt("settings-theme")
                    .selected_text(theme_mode.label())
                    .show_ui(ui, |ui| {
                        for mode in [ThemeMode::System, ThemeMode::Dark, ThemeMode::Light] {
                            ui.selectable_value(&mut theme_mode, mode, mode.label());
                        }
                    });
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "The desktop drives the existing CLI. It never replaces or reimplements core behavior.",
                )
                .size(10.0)
                .color(theme::MUTED),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Apply and reconnect").clicked() {
                    apply = true;
                }
                if ui.button("Reconnect now").clicked() {
                    reconnect = true;
                }
                if ui.button("Close").clicked() {
                    open = false;
                }
            });
        });
    if !window_open {
        open = false;
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
    let mut window_open = true;
    let mut confirmed = false;
    let mut cancelled = false;
    egui::Window::new(action.title())
        .open(&mut window_open)
        .collapsible(false)
        .resizable(false)
        .default_width(470.0)
        .show(ctx, |ui| {
            match &action {
                ConfirmAction::ReleaseSession(session) => {
                    ui.label("This releases AndroidForge ownership without stopping the emulator.");
                    session_summary(ui, session);
                }
                ConfirmAction::StopSession(session) => {
                    ui.colored_label(
                        theme::RED,
                        "This stops the emulator and releases the session. Unsaved state may be lost.",
                    );
                    session_summary(ui, session);
                }
                ConfirmAction::RestartSession(session) => {
                    ui.colored_label(
                        theme::YELLOW,
                        "This restarts the emulator. Runtime state is lost.",
                    );
                    session_summary(ui, session);
                }
                ConfirmAction::Install { apk_path, fresh } => {
                    ui.label("Install this APK on the selected session?");
                    ui.label(RichText::new(apk_path.display().to_string()).monospace().size(10.0));
                    if let Some(session) = app.selected_session_info() {
                        ui.label(
                            RichText::new(format!(
                                "Target: {} · {} · {}",
                                session.serial.as_deref().unwrap_or("-"),
                                session.id,
                                session.owner
                            ))
                            .size(11.0)
                            .color(theme::MUTED),
                        );
                    }
                    if *fresh {
                        ui.colored_label(
                            theme::RED,
                            "Fresh install replaces the app and deletes its data.",
                        );
                    }
                }
                ConfirmAction::Fixture { name, yes } => {
                    ui.label(format!("Run fixture '{name}' on the selected session?"));
                    project_target_summary(app, ui);
                    if *yes {
                        ui.colored_label(
                            theme::YELLOW,
                            "This flow may modify app or device state. You confirmed destructive steps.",
                        );
                    }
                }
                ConfirmAction::Conformance { yes } => {
                    ui.label("Run the configured conformance flow on the selected session?");
                    project_target_summary(app, ui);
                    if *yes {
                        ui.colored_label(
                            theme::YELLOW,
                            "The flow may run a setup fixture or modify app state.",
                        );
                    }
                }
                ConfirmAction::Hook { name } => {
                    ui.label(format!("Run hook '{name}' on the selected session?"));
                    project_target_summary(app, ui);
                    ui.colored_label(
                        theme::YELLOW,
                        "Hooks are project-authored commands or device triggers. Review the adapter before running.",
                    );
                }
                ConfirmAction::CancelOperation => {
                    ui.colored_label(
                        theme::RED,
                        "This terminates the AndroidForge child process tree, not the emulator session.",
                    );
                    ui.label("The final CLI report may be incomplete. The device and session remain intact.");
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let label = action.confirm_label();
                if ui
                    .add(egui::Button::new(RichText::new(label).color(if action.destructive() {
                        theme::RED
                    } else {
                        ui.style().visuals.text_color()
                    })))
                    .clicked()
                {
                    confirmed = true;
                }
                if ui.button("Cancel").clicked() {
                    cancelled = true;
                }
            });
        });
    if confirmed {
        app.execute_confirmation(ctx);
        app.confirm = None;
    } else if cancelled || !window_open {
        app.confirm = None;
    }
}

fn install_dialog(app: &mut App, ctx: &egui::Context) {
    if !app.install.open {
        return;
    }
    let mut open = true;
    let mut window_open = true;
    egui::Window::new("Install APK")
        .open(&mut window_open)
        .collapsible(false)
        .resizable(true)
        .default_width(680.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Choose APK...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Android package", &["apk"])
                        .pick_file()
                    {
                        app.choose_apk(path, ctx);
                    }
                }
                if let Some(path) = &app.install.apk_path {
                    truncated_text(ui, &path.display().to_string(), 70);
                }
            });
            if app.install.loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Inspecting APK...");
                });
            }
            if let Some(error) = &app.install.error {
                error_banner(ui, error);
            }
            if let Some(info) = &app.install.info {
                ui.add_space(6.0);
                egui::Grid::new("apk-identity")
                    .num_columns(2)
                    .spacing([14.0, 4.0])
                    .show(ui, |ui| {
                        key_value(ui, "Package", info.package.as_deref().unwrap_or("-"));
                        ui.end_row();
                        key_value(ui, "Version", info.version_name.as_deref().unwrap_or("-"));
                        ui.end_row();
                        key_value(
                            ui,
                            "Version code",
                            &info.version_code.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
                        );
                        ui.end_row();
                        key_value(ui, "Size", &format!("{:.1} MB", info.size_bytes as f64 / 1_048_576.0));
                        ui.end_row();
                        key_value(ui, "SHA-256", &info.sha256);
                        ui.end_row();
                        key_value(
                            ui,
                            "Certificate",
                            info.signature.cert_sha256.as_deref().unwrap_or("-"),
                        );
                        ui.end_row();
                        key_value(
                            ui,
                            "Signer",
                            info.signature.signer_dn.as_deref().unwrap_or("-"),
                        );
                        ui.end_row();
                    });
                if let Some(mismatch) = &app.install.mismatch {
                    ui.add_space(4.0);
                    error_banner(ui, mismatch);
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Target session").size(11.0).color(theme::MUTED));
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
                            if session.state == "ready" || session.state == "attached" {
                                if ui
                                    .selectable_label(
                                        app.selected_session.as_deref() == Some(session.id.as_str()),
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
                        }
                    });
                if selected != app.selected_session {
                    app.selected_session = selected;
                }
            });
            ui.checkbox(&mut app.install.fresh, "Fresh install (deletes app data)");
            if app.install.fresh {
                ui.colored_label(
                    theme::RED,
                    "Fresh install is destructive. The CLI will also receive --yes after confirmation.",
                );
            }
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                let can_preview = app.install.info.is_some()
                    && app.selected_session.is_some()
                    && !app.install.preview_loading;
                if ui
                    .add_enabled(can_preview, egui::Button::new("Dry run"))
                    .on_hover_text("Preview install constraints without changing the device")
                    .clicked()
                {
                    app.preview_install(ctx);
                }
                let can_install = app.install.info.is_some()
                    && app.selected_session.is_some()
                    && app.install.mismatch.is_none()
                    && !app.active_operation();
                if ui
                    .add_enabled(can_install, egui::Button::new("Install..."))
                    .clicked()
                {
                    if let Some(path) = app.install.apk_path.clone() {
                        app.install.open = false;
                        open = false;
                        app.confirm(ConfirmAction::Install {
                            apk_path: path,
                            fresh: app.install.fresh,
                        });
                    }
                }
                if app.install.preview_loading {
                    ui.spinner();
                }
                if ui.button("Close").clicked() {
                    open = false;
                }
            });
            if let Some(error) = &app.install.preview_error {
                error_banner(ui, error);
            }
            if let Some(preview) = &app.install.preview {
                ui.collapsing("Dry-run preview", |ui| {
                    let mut text = pretty_json(preview);
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .font(egui::FontId::monospace(10.0))
                            .desired_rows(8)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
            }
            if let Some(build) = &app.install.result {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Installed {} {}",
                        build.package.as_deref().unwrap_or("-"),
                        build.version_name.as_deref().unwrap_or("-")
                    ))
                    .color(theme::GREEN),
                );
                ui.label(
                    RichText::new(format!(
                        "verified={} sha256={}",
                        build.verified,
                        build.installed_apk_sha256.as_deref().unwrap_or("-")
                    ))
                    .size(10.0)
                    .color(theme::MUTED),
                );
            }
        });
    if !window_open {
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
    let mut start = false;
    let avds = app.doctor.value.avds.clone();
    egui::Window::new("Start emulator session")
        .open(&mut window_open)
        .collapsible(false)
        .resizable(false)
        .default_width(480.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("AVD").size(11.0).color(theme::MUTED));
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
                ui.label(RichText::new("Port").size(11.0).color(theme::MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut form.port)
                        .desired_width(90.0)
                        .hint_text("auto"),
                );
                ui.label(
                    RichText::new("Optional even console port; ADB uses port + 1")
                        .size(10.0)
                        .color(theme::MUTED),
                );
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Owner").size(11.0).color(theme::MUTED));
                ui.add(egui::TextEdit::singleline(&mut form.owner).desired_width(220.0));
            });
            ui.checkbox(&mut form.headless, "Headless");
            ui.checkbox(&mut form.prepare, "Prepare device settings");
            ui.checkbox(&mut form.reuse, "Reuse a running emulator for this AVD");
            if form.avd.is_empty() {
                ui.colored_label(theme::YELLOW, "Choose an AVD to continue.");
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !form.avd.is_empty()
                            && !form.owner.trim().is_empty()
                            && !app.active_operation(),
                        egui::Button::new("Start session"),
                    )
                    .clicked()
                {
                    start = true;
                }
                if ui.button("Cancel").clicked() {
                    open = false;
                }
            });
        });
    if !window_open {
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
    let mut attach = false;
    egui::Window::new("Attach to device")
        .open(&mut window_open)
        .collapsible(false)
        .resizable(false)
        .default_width(480.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Serial").size(11.0).color(theme::MUTED));
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
                ui.label(RichText::new("Owner").size(11.0).color(theme::MUTED));
                ui.add(egui::TextEdit::singleline(&mut form.owner).desired_width(220.0));
            });
            ui.checkbox(&mut form.prepare, "Prepare device settings");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !form.serial.is_empty()
                            && !form.owner.trim().is_empty()
                            && !app.active_operation(),
                        egui::Button::new("Attach"),
                    )
                    .clicked()
                {
                    attach = true;
                }
                if ui.button("Cancel").clicked() {
                    open = false;
                }
            });
        });
    if !window_open {
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
        .default_width(540.0)
        .show(ctx, |ui| {
            project_target_summary(app, ui);
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut form.conformance, false, "Fixture");
                ui.selectable_value(&mut form.conformance, true, "Conformance");
            });
            if form.conformance {
                match &conformance {
                    Some(flow) => {
                        ui.label(RichText::new(format!("Conformance flow: {flow}")).size(11.0));
                    }
                    None => {
                        ui.colored_label(theme::YELLOW, "No conformance flow is configured.");
                    }
                }
            } else if fixtures.is_empty() {
                ui.colored_label(
                    theme::YELLOW,
                    "No fixtures are configured for this project.",
                );
            } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Fixture").size(11.0).color(theme::MUTED));
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
                if let Some(name) = &form.fixture {
                    if let Some(fixture) = fixtures.iter().find(|fixture| &fixture.name == name) {
                        ui.label(
                            RichText::new(
                                fixture.description.as_deref().unwrap_or("No description"),
                            )
                            .size(10.0)
                            .color(theme::MUTED),
                        );
                    }
                }
            }
            ui.add_space(6.0);
            ui.checkbox(
                &mut form.yes,
                "I understand this may modify app or device state and confirm destructive steps",
            );
            ui.add_space(5.0);
            let target_ready = app.selected_session.is_some();
            let can_run = if form.conformance {
                conformance.is_some() && form.yes && target_ready && !app.active_operation()
            } else {
                form.fixture.is_some() && form.yes && target_ready && !app.active_operation()
            };
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(can_run, egui::Button::new("Review and run"))
                    .clicked()
                {
                    run = true;
                }
                if ui.button("Cancel").clicked() {
                    open = false;
                }
            });
        });
    if !window_open {
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
        .default_width(640.0)
        .show(ctx, |ui| {
            ui.label(
                RichText::new(format!("{}: {}", error.code, error.message))
                    .color(theme::RED)
                    .strong(),
            );
            if let Some(cause) = &error.cause {
                ui.label(cause);
            }
            if !error.suggestions.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new("Suggestions").strong());
                for suggestion in &error.suggestions {
                    ui.label(format!("• {suggestion}"));
                }
            }
            if !error.context.is_empty() {
                ui.collapsing("Context", |ui| {
                    let mut text = pretty_json(&serde_json::Value::Object(
                        error.context.clone().into_iter().collect(),
                    ));
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .font(egui::FontId::monospace(10.0))
                            .desired_rows(6)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
            }
            if !error.artifacts.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new("Artifacts").strong());
                for artifact in &error.artifacts {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{} · {}", artifact.kind, artifact.path))
                                .size(10.0)
                                .monospace(),
                        );
                    });
                }
            }
            if let Some(raw) = &error.raw_json {
                ui.collapsing("Raw JSON", |ui| {
                    let mut text = raw.clone();
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .font(egui::FontId::monospace(10.0))
                            .desired_rows(8)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                    if ui.small_button("Copy raw JSON").clicked() {
                        ui.ctx().copy_text(raw.clone());
                    }
                });
            }
            ui.add_space(4.0);
            if ui.button("Close").clicked() {
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
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-14.0, -14.0))
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
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.label(RichText::new(&toast.text).size(11.0).color(color));
                    });
            }
        });
    ctx.request_repaint_after(Duration::from_secs(1));
}

fn pretty_json(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}
