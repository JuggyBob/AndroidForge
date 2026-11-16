use std::path::PathBuf;

use eframe::egui;
use egui::{Align, Color32, Layout, RichText, Sense, Stroke, Vec2};

use crate::app::App;
use crate::backend::evidence::truncate_middle;
use crate::state::*;
use crate::theme;

pub fn topbar(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let icon_size = Vec2::splat(20.0);
        ui.add(egui::Image::new((app.icon.id(), icon_size)));
        ui.add_space(2.0);
        ui.label(RichText::new("AndroidForge").size(15.0).strong());
        ui.label(RichText::new("Desktop").size(11.0).color(theme::MUTED));
        ui.add_space(8.0);
        let mut project_choice = app.project_path.clone();
        let project_label = app
            .selected_project()
            .map(|project| project.display_name())
            .unwrap_or_else(|| "No project".to_string());
        egui::ComboBox::from_id_salt("topbar-project")
            .width(190.0)
            .selected_text(project_label)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(app.project_path.is_none(), "No project")
                    .clicked()
                {
                    project_choice = None;
                }
                for project in &app.projects {
                    if ui
                        .selectable_label(
                            app.project_path.as_deref() == Some(project.root.as_path()),
                            project.display_name(),
                        )
                        .on_hover_text(project.root.display().to_string())
                        .clicked()
                    {
                        project_choice = Some(project.root.clone());
                    }
                }
            });
        if project_choice != app.project_path {
            app.select_project(project_choice, ui.ctx());
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Settings").clicked() {
                app.open_settings();
            }
            let theme_label = match app.settings.theme {
                ThemeMode::System => "System",
                ThemeMode::Dark => "Dark",
                ThemeMode::Light => "Light",
            };
            if ui
                .button(theme_label)
                .on_hover_text("Cycle system, dark and light theme")
                .clicked()
            {
                app.settings.theme = match app.settings.theme {
                    ThemeMode::System => ThemeMode::Dark,
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::System,
                };
            }
            if ui.button("Refresh").clicked() {
                app.manual_refresh(ui.ctx());
            }
            let (text, color) = if app.cli.info.is_some() {
                (
                    format!(
                        "CLI {}",
                        app.cli
                            .info
                            .as_ref()
                            .map(|info| info.version.as_str())
                            .unwrap_or("-")
                    ),
                    theme::GREEN,
                )
            } else {
                ("CLI offline".to_string(), theme::RED)
            };
            status_badge(ui, &text, color).on_hover_text(
                app.cli
                    .resolution
                    .as_ref()
                    .map(|resolution| {
                        format!(
                            "{}: {}",
                            resolution.source.label(),
                            resolution.path.display()
                        )
                    })
                    .unwrap_or_else(|| "No CLI connection".to_string()),
            );
        });
    });
}

pub fn sidebar(app: &mut App, ui: &mut egui::Ui) {
    ui.add_space(2.0);
    ui.label(RichText::new("WORKSPACE").size(10.0).color(theme::MUTED));
    ui.add_space(4.0);
    let selected = app.page;
    if nav_row(
        ui,
        selected == Page::Overview,
        theme::NavIcon::Overview,
        "Overview",
        None,
    )
    .clicked()
    {
        app.set_page(Page::Overview);
    }
    if nav_row(
        ui,
        selected == Page::Devices,
        theme::NavIcon::Devices,
        "Devices",
        Some(app.devices.value.devices.len()),
    )
    .clicked()
    {
        app.set_page(Page::Devices);
    }
    if nav_row(
        ui,
        selected == Page::Sessions,
        theme::NavIcon::Sessions,
        "Sessions",
        Some(app.sessions.value.sessions.len()),
    )
    .clicked()
    {
        app.set_page(Page::Sessions);
    }
    if nav_row(
        ui,
        selected == Page::Runs,
        theme::NavIcon::Runs,
        "Runs",
        Some(app.runs.value.runs.len()),
    )
    .clicked()
    {
        app.set_page(Page::Runs);
    }

    ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!(
                "CLI {}",
                app.cli
                    .info
                    .as_ref()
                    .map(|info| info.version.as_str())
                    .unwrap_or("not connected")
            ))
            .size(10.0)
            .color(theme::MUTED),
        );
        ui.label(
            RichText::new(format!("Desktop {}", env!("CARGO_PKG_VERSION")))
                .size(10.0)
                .color(theme::MUTED),
        );
        ui.horizontal(|ui| {
            let color = if app.cli.info.is_some() {
                theme::GREEN
            } else {
                theme::RED
            };
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 3.0, color);
            ui.label(
                RichText::new(if app.cli.info.is_some() {
                    "CLI connected"
                } else {
                    "CLI disconnected"
                })
                .size(10.0)
                .color(color),
            );
        });
        ui.separator();
    });
}

fn nav_row(
    ui: &mut egui::Ui,
    selected: bool,
    icon: theme::NavIcon,
    label: &str,
    count: Option<usize>,
) -> egui::Response {
    let desired = Vec2::new(ui.available_width(), 30.0);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let dark = ui.style().visuals.dark_mode;
    if selected {
        let fill = if dark {
            Color32::from_rgb(31, 38, 50)
        } else {
            Color32::from_rgb(232, 236, 242)
        };
        ui.painter()
            .rect_filled(rect.shrink2(Vec2::new(2.0, 1.0)), 4.0, fill);
    } else if response.hovered() {
        let fill = if dark {
            Color32::from_rgb(24, 30, 39)
        } else {
            Color32::from_rgb(240, 242, 245)
        };
        ui.painter()
            .rect_filled(rect.shrink2(Vec2::new(2.0, 1.0)), 4.0, fill);
    }
    let color = if selected {
        ui.style().visuals.text_color()
    } else {
        theme::MUTED
    };
    let icon_rect = egui::Rect::from_min_size(rect.min + Vec2::new(10.0, 7.0), Vec2::splat(16.0));
    theme::paint_nav_icon(ui.painter(), icon_rect, icon, color);
    ui.painter().text(
        rect.min + Vec2::new(38.0, 15.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        color,
    );
    if let Some(count) = count {
        ui.painter().text(
            rect.right_center() - Vec2::new(10.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::monospace(11.0),
            theme::MUTED,
        );
    }
    response
}

pub fn status_badge(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let text = RichText::new(format!("● {text}")).size(11.0).color(color);
    ui.label(text)
}

pub fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(title).size(12.0).strong());
    ui.add_space(2.0);
}

pub fn muted(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).size(11.0).color(theme::MUTED));
}

pub fn empty_state(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.add_space(20.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).size(14.0).strong());
        ui.label(RichText::new(detail).size(12.0).color(theme::MUTED));
    });
    ui.add_space(20.0);
}

pub fn error_banner(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(58, 24, 24))
        .stroke(Stroke::new(1.0, Color32::from_rgb(140, 50, 50)))
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(11.0)
                    .color(Color32::from_rgb(255, 190, 190)),
            );
        });
}

pub fn stale_note(ui: &mut egui::Ui, state: &DataState<impl Default>, label: &str) {
    if let Some(error) = &state.error {
        error_banner(ui, &format!("{label} is stale: {error}"));
    } else if let Some(age) = state.age() {
        if age > std::time::Duration::from_secs(15) {
            muted(ui, format!("{label} updated {}s ago", age.as_secs()));
        }
    }
}

pub fn copy_button(ui: &mut egui::Ui, value: &str, label: &str) {
    if ui.small_button(label).clicked() {
        ui.ctx().copy_text(value.to_string());
    }
}

pub fn truncated_text(ui: &mut egui::Ui, value: &str, max: usize) {
    let short = truncate_middle(value, max);
    ui.add(egui::Label::new(RichText::new(short).size(11.0)).truncate())
        .on_hover_text(value);
}

pub fn key_value(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.label(RichText::new(key).size(11.0).color(theme::MUTED));
    ui.label(RichText::new(value).size(11.0));
}

pub fn project_strip(app: &App, ui: &mut egui::Ui) {
    let Some(project) = app.selected_project() else {
        ui.horizontal(|ui| {
            muted(
                ui,
                "No project selected. Choose one in the top bar or open Settings.",
            );
        });
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(project.display_name()).size(12.0).strong());
        ui.label(RichText::new("·").color(theme::MUTED));
        ui.label(
            RichText::new(project.package_label())
                .size(11.0)
                .color(theme::MUTED),
        );
        ui.label(RichText::new("·").color(theme::MUTED));
        ui.label(
            RichText::new(format!("adapter v{}", project.adapter_version))
                .size(11.0)
                .color(theme::GREEN),
        );
        if let Some(flow) = project.conformance_flow() {
            ui.label(RichText::new("·").color(theme::MUTED));
            ui.label(RichText::new(flow).size(11.0).color(theme::MUTED));
        }
    });
}

pub fn title_row(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(18.0).strong());
            if !subtitle.is_empty() {
                ui.label(RichText::new(subtitle).size(11.0).color(theme::MUTED));
            }
        });
    });
    ui.add_space(4.0);
}

pub fn action_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    tooltip: &str,
) -> egui::Response {
    ui.add_enabled(enabled, egui::Button::new(label))
        .on_hover_text(tooltip)
}

pub fn path_button(ui: &mut egui::Ui, path: &str) {
    if ui.small_button("Open").on_hover_text(path).clicked() {
        let _ = open::that(path);
    }
}

pub fn settings_path_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<String>,
    directory: bool,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(11.0).color(theme::MUTED));
        let mut text = value.clone().unwrap_or_default();
        let response = ui.add(
            egui::TextEdit::singleline(&mut text)
                .desired_width(360.0)
                .hint_text(if directory {
                    "workspace directory"
                } else {
                    "androidforge.exe"
                }),
        );
        if response.changed() {
            *value = if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            };
            changed = true;
        }
        if ui.button("Browse").clicked() {
            let picked = if directory {
                rfd::FileDialog::new().pick_folder()
            } else {
                rfd::FileDialog::new()
                    .add_filter("executable", &["exe"])
                    .pick_file()
            };
            if let Some(path) = picked {
                *value = Some(path.display().to_string());
                changed = true;
            }
        }
        if value.is_some() && ui.small_button("Clear").clicked() {
            *value = None;
            changed = true;
        }
    });
    changed
}

pub fn parse_path(value: &Option<String>) -> Option<PathBuf> {
    value.as_ref().map(PathBuf::from)
}

pub fn session_summary(ui: &mut egui::Ui, session: &crate::models::SessionInfo) {
    ui.add_space(4.0);
    egui::Grid::new("confirm-session")
        .num_columns(2)
        .spacing([12.0, 3.0])
        .show(ui, |ui| {
            key_value(ui, "Session", &session.id);
            ui.end_row();
            key_value(ui, "Owner", &session.owner);
            ui.end_row();
            key_value(ui, "Device", session.serial.as_deref().unwrap_or("-"));
            ui.end_row();
            key_value(ui, "AVD", session.avd.as_deref().unwrap_or("-"));
            ui.end_row();
        });
}

pub fn project_target_summary(app: &App, ui: &mut egui::Ui) {
    if let Some(project) = app.selected_project() {
        ui.label(
            RichText::new(format!(
                "Project: {} ({})",
                project.display_name(),
                project.package_label()
            ))
            .size(11.0)
            .color(theme::MUTED),
        );
    }
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
    } else {
        ui.colored_label(theme::RED, "No session selected.");
    }
}
