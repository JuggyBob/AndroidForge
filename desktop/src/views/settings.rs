use eframe::egui;
use egui::{Align, Color32, Layout, RichText};

use crate::app::App;
use crate::components::*;
use crate::fonts;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    page_header(
        ui,
        "Settings",
        "Connection, appearance, typography and updates for the AndroidForge cockpit.",
    );

    let mut apply = false;
    let mut reconnect = false;
    let mut choose_cli = false;
    let mut choose_workspace = false;

    ui.columns(2, |columns| {
        connection_card(
            app,
            &mut columns[0],
            &mut apply,
            &mut reconnect,
            &mut choose_cli,
            &mut choose_workspace,
        );
        appearance_card(app, &mut columns[1]);
    });

    ui.add_space(theme::PAGE_GAP);
    ui.columns(2, |columns| {
        updates_card(app, &mut columns[0]);
        about_card(&mut columns[1]);
    });

    if choose_cli
        && let Some(path) = rfd::FileDialog::new()
            .add_filter("executable", &["exe"])
            .pick_file()
    {
        app.settings.cli_path = Some(path.display().to_string());
        apply = true;
    }
    if choose_workspace && let Some(path) = rfd::FileDialog::new().pick_folder() {
        app.settings.workspace = Some(path.display().to_string());
        apply = true;
    }
    if apply {
        app.reconnect(ui.ctx());
        app.refresh_projects(ui.ctx());
        app.toast("Settings applied", crate::state::ToastLevel::Success);
    } else if reconnect {
        app.reconnect(ui.ctx());
        app.toast("CLI reconnected", crate::state::ToastLevel::Success);
    }
}

fn connection_card(
    app: &mut App,
    ui: &mut egui::Ui,
    apply: &mut bool,
    reconnect: &mut bool,
    choose_cli: &mut bool,
    choose_workspace: &mut bool,
) {
    card(ui, |ui| {
        section_title(ui, "Connection");
        ui.label(RichText::new("CLI").size(10.0).color(theme::MUTED));
        ui.horizontal(|ui| {
            let mut cli_path = app.settings.cli_path.clone().unwrap_or_default();
            let width = (ui.available_width() - 190.0).max(220.0);
            let response = ui.add(
                egui::TextEdit::singleline(&mut cli_path)
                    .desired_width(width)
                    .hint_text("Path to androidforge.exe"),
            );
            if response.changed() {
                app.settings.cli_path = if cli_path.trim().is_empty() {
                    None
                } else {
                    Some(cli_path.trim().to_string())
                };
            }
            if secondary_button(ui, "Browse...").clicked() {
                *choose_cli = true;
            }
            if app.settings.cli_path.is_some() && ghost_button(ui, "Clear").clicked() {
                app.settings.cli_path = None;
                *apply = true;
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Workspace").size(10.0).color(theme::MUTED));
        ui.horizontal(|ui| {
            let mut workspace = app.settings.workspace.clone().unwrap_or_default();
            let width = (ui.available_width() - 120.0).max(220.0);
            let response = ui.add(
                egui::TextEdit::singleline(&mut workspace)
                    .desired_width(width)
                    .hint_text("Workspace directory"),
            );
            if response.changed() {
                app.settings.workspace = if workspace.trim().is_empty() {
                    None
                } else {
                    Some(workspace.trim().to_string())
                };
            }
            if secondary_button(ui, "Browse...").clicked() {
                *choose_workspace = true;
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Owner").size(10.0).color(theme::MUTED));
        let owner_width = (ui.available_width() - 20.0).max(220.0);
        ui.add(egui::TextEdit::singleline(&mut app.settings.owner).desired_width(owner_width));

        ui.add_space(8.0);
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
            path_block(ui, &resolution.path.display().to_string());
            small(ui, resolution.source.label());
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if primary_button(ui, "Apply and reconnect").clicked() {
                *apply = true;
            }
            if secondary_button(ui, "Reconnect now").clicked() {
                *reconnect = true;
            }
            if ghost_button(ui, "Restart setup").clicked() {
                app.restart_onboarding();
            }
        });
    });
}

fn appearance_card(app: &mut App, ui: &mut egui::Ui) {
    card(ui, |ui| {
        section_title(ui, "Appearance");
        egui::Grid::new("settings-appearance-grid")
            .num_columns(2)
            .spacing([14.0, 12.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Theme").size(10.0).color(theme::MUTED));
                egui::ComboBox::from_id_salt("settings-theme")
                    .width(180.0)
                    .selected_text(app.settings.theme.label())
                    .show_ui(ui, |ui| {
                        for mode in [
                            crate::state::ThemeMode::System,
                            crate::state::ThemeMode::Dark,
                            crate::state::ThemeMode::Light,
                        ] {
                            if ui
                                .selectable_value(&mut app.settings.theme, mode, mode.label())
                                .clicked()
                            {
                                theme::apply(ui.ctx(), mode);
                            }
                        }
                    });
                ui.end_row();

                ui.label(RichText::new("Accent").size(10.0).color(theme::MUTED));
                ui.horizontal(|ui| {
                    let mut accent = app.settings.accent;
                    if egui::color_picker::color_edit_button_srgb(ui, &mut accent).changed() {
                        app.settings.accent = accent;
                        theme::set_accent(Color32::from_rgb(accent[0], accent[1], accent[2]));
                        theme::apply(ui.ctx(), app.settings.theme);
                    }
                    ui.label(
                        RichText::new(format!(
                            "#{:02X}{:02X}{:02X}",
                            accent[0], accent[1], accent[2]
                        ))
                        .size(10.0)
                        .monospace()
                        .color(theme::MUTED),
                    );
                    if ghost_button(ui, "Reset").clicked() {
                        app.settings.accent = [240, 166, 50];
                        theme::set_accent(Color32::from_rgb(240, 166, 50));
                        theme::apply(ui.ctx(), app.settings.theme);
                    }
                });
                ui.end_row();

                ui.label(RichText::new("Font").size(10.0).color(theme::MUTED));
                font_picker(app, ui);
                ui.end_row();
            });
        ui.add_space(8.0);
        ui.label(
            RichText::new("Accent color wheel")
                .size(10.0)
                .color(theme::MUTED),
        );
        accent_wheel(app, ui);
        small(ui, "Click or drag the wheel, then adjust brightness.");
        ui.add_space(6.0);
        small(
            ui,
            "The accent is applied immediately. Fonts are loaded locally and never uploaded.",
        );
    });
}

fn accent_wheel(app: &mut App, ui: &mut egui::Ui) {
    use egui::ecolor::Hsva;
    const SIZE: f32 = 154.0;
    let texture = if let Some(texture) = &app.accent_wheel {
        texture.clone()
    } else {
        let n = 180usize;
        let mut pixels = Vec::with_capacity(n * n);
        for y in 0..n {
            for x in 0..n {
                let dx = (x as f32 + 0.5) / n as f32 * 2.0 - 1.0;
                let dy = (y as f32 + 0.5) / n as f32 * 2.0 - 1.0;
                let radius = (dx * dx + dy * dy).sqrt();
                if radius > 1.0 {
                    pixels.push(Color32::TRANSPARENT);
                } else {
                    let hue = (dy.atan2(dx) / std::f32::consts::TAU + 1.0) % 1.0;
                    pixels.push(Color32::from(Hsva::new(hue, radius, 1.0, 1.0)));
                }
            }
        }
        let texture = ui.ctx().load_texture(
            "settings-accent-wheel",
            egui::ColorImage::new([n, n], pixels),
            egui::TextureOptions::LINEAR,
        );
        app.accent_wheel = Some(texture.clone());
        texture
    };
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::splat(SIZE), egui::Sense::click_and_drag());
    ui.painter().image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    ui.painter().circle_stroke(
        rect.center(),
        SIZE / 2.0,
        egui::Stroke::new(1.0, theme::border_color(ui)),
    );
    let accent = Color32::from_rgb(
        app.settings.accent[0],
        app.settings.accent[1],
        app.settings.accent[2],
    );
    let mut hsva = Hsva::from(accent);
    let angle = hsva.h * std::f32::consts::TAU;
    let radius = hsva.s * SIZE / 2.0;
    let marker = rect.center() + egui::Vec2::new(angle.cos() * radius, angle.sin() * radius);
    ui.painter()
        .circle_stroke(marker, 7.0, egui::Stroke::new(2.0, Color32::WHITE));
    ui.painter()
        .circle_stroke(marker, 8.5, egui::Stroke::new(1.0, Color32::BLACK));
    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let dx = (pos.x - rect.center().x) / (SIZE / 2.0);
        let dy = (pos.y - rect.center().y) / (SIZE / 2.0);
        let saturation = (dx * dx + dy * dy).sqrt().min(1.0);
        let hue = (dy.atan2(dx) / std::f32::consts::TAU + 1.0) % 1.0;
        hsva = Hsva::new(hue, saturation, hsva.v.max(0.4), 1.0);
        update_accent(app, ui, hsva);
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Brightness").size(10.0).color(theme::MUTED));
        ui.scope(|ui| {
            theme::configure_slider(ui, 150.0);
            let mut value = hsva.v;
            if ui
                .add_sized(
                    [240.0, 22.0],
                    egui::Slider::new(&mut value, 0.2..=1.0)
                        .trailing_fill(true)
                        .show_value(true),
                )
                .changed()
            {
                hsva.v = value;
                update_accent(app, ui, hsva);
            }
        });
    });
}

fn update_accent(app: &mut App, ui: &mut egui::Ui, hsva: egui::ecolor::Hsva) {
    let color = Color32::from(hsva);
    app.settings.accent = [color.r(), color.g(), color.b()];
    theme::set_accent(color);
    theme::apply(ui.ctx(), app.settings.theme);
    ui.ctx().request_repaint();
}

fn font_picker(app: &mut App, ui: &mut egui::Ui) {
    let current = fonts::label_for(app.settings.font.as_deref());
    let mut chosen = None;
    egui::ComboBox::from_id_salt("settings-font")
        .width(280.0)
        .selected_text(RichText::new(current).size(11.0))
        .show_ui(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut app.font_search)
                    .hint_text("Search fonts...")
                    .desired_width(250.0),
            );
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("settings-font-list")
                .max_height(280.0)
                .show(ui, |ui| {
                    let query = app.font_search.to_ascii_lowercase();
                    for choice in fonts::choices() {
                        if !query.is_empty() && !choice.label.to_ascii_lowercase().contains(&query)
                        {
                            continue;
                        }
                        let selected = app.settings.font.as_deref() == Some(choice.key.as_str());
                        let response = ui
                            .horizontal(|ui| {
                                ui.label(RichText::new(&choice.label).size(11.0).color(
                                    if selected {
                                        theme::accent()
                                    } else {
                                        theme::text_color(ui)
                                    },
                                ));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new("Lorem Ipsum")
                                            .font(egui::FontId::new(14.0, choice.family()))
                                            .color(theme::MUTED),
                                    );
                                });
                            })
                            .response
                            .interact(egui::Sense::click());
                        if selected {
                            ui.painter().rect_stroke(
                                response.rect,
                                theme::ROW_RADIUS,
                                egui::Stroke::new(1.0, theme::accent()),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if response.clicked() {
                            chosen = Some(choice.key.clone());
                            ui.close();
                        }
                    }
                    if let Some(path) = app.settings.font.as_deref()
                        && std::path::Path::new(path).is_file()
                    {
                        ui.separator();
                        small(ui, format!("Custom: {path}"));
                    }
                });
        });
    if secondary_button(ui, "Browse font file...").clicked()
        && let Some(path) = rfd::FileDialog::new()
            .add_filter("font", &["ttf", "otf"])
            .pick_file()
    {
        chosen = Some(path.display().to_string());
    }
    if let Some(chosen) = chosen {
        app.settings.font = Some(chosen);
        fonts::apply(ui.ctx(), app.settings.font.as_deref());
        ui.ctx().request_repaint();
    }
}

fn updates_card(app: &mut App, ui: &mut egui::Ui) {
    card(ui, |ui| {
        section_title(ui, "Updates");
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
        ui.add_space(8.0);
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
    });
}

fn about_card(ui: &mut egui::Ui) {
    card(ui, |ui| {
        section_title(ui, "About");
        small(
            ui,
            "AndroidForge Desktop is a native egui/eframe client. The CLI remains authoritative for every Android operation.",
        );
        ui.add_space(6.0);
        kv_grid(
            ui,
            "settings-about",
            &[
                ("Desktop version", env!("CARGO_PKG_VERSION").to_string()),
                ("Renderer", "egui / eframe / glow".to_string()),
                ("Network", "Local CLI only".to_string()),
            ],
        );
    });
}
