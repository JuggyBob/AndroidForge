use std::path::PathBuf;

use eframe::egui;
use egui::{Align, Color32, Layout, RichText, Sense, Stroke, StrokeKind, Vec2};

use crate::app::App;
use crate::backend::evidence::truncate_middle;
use crate::state::*;
use crate::theme;

pub fn sidebar(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical()
        .id_salt("sidebar-nav")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.add(egui::Image::new((app.icon.id(), Vec2::splat(28.0))));
                ui.add_space(7.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("AndroidForge").size(14.5).strong());
                    ui.label(
                        RichText::new(format!("Desktop {}", env!("CARGO_PKG_VERSION")))
                            .size(9.5)
                            .color(theme::MUTED),
                    );
                });
            });
            ui.add_space(12.0);

            // The highlight glides between nav rows. Rects are recorded from
            // the previous frame; the 0.28 easing hides the one-frame lag.
            if let Some(target) = app.nav_rects.get(&app.page).map(|rect| rect.center().y) {
                let current = app.nav_highlight_y.unwrap_or(target);
                let next = current + (target - current) * 0.28;
                app.nav_highlight_y = Some(next);
                if (target - next).abs() > 0.4 {
                    ui.ctx().request_repaint();
                }
                let x = ui.max_rect().left() + 4.0;
                let width = (ui.available_width() - 8.0).max(40.0);
                let rect = egui::Rect::from_min_max(
                    egui::pos2(x, next - 15.0),
                    egui::pos2(x + width, next + 15.0),
                );
                let fill = if ui.style().visuals.dark_mode {
                    Color32::from_rgb(31, 38, 47)
                } else {
                    Color32::from_rgb(234, 239, 246)
                };
                ui.painter().rect_filled(rect, theme::ROW_RADIUS, fill);
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(rect.left() + 2.0, rect.top() + 7.0),
                        Vec2::new(3.0, rect.height() - 14.0),
                    ),
                    2.0,
                    theme::accent(),
                );
            }
            nav_group(ui, "WORKSPACE", &mut |ui| {
                nav(app, ui, Page::Setup, theme::NavIcon::Setup, "Setup", None);
                nav(
                    app,
                    ui,
                    Page::Tools,
                    theme::NavIcon::Help,
                    "Tool database",
                    None,
                );
                nav(
                    app,
                    ui,
                    Page::Overview,
                    theme::NavIcon::Overview,
                    "Overview",
                    None,
                );
                nav(
                    app,
                    ui,
                    Page::Context,
                    theme::NavIcon::Context,
                    "Context",
                    None,
                );
            });
            nav_group(ui, "TEST & DEBUG", &mut |ui| {
                nav(
                    app,
                    ui,
                    Page::Devices,
                    theme::NavIcon::Devices,
                    "Devices",
                    Some(app.devices.value.devices.len().to_string()),
                );
                nav(
                    app,
                    ui,
                    Page::Sessions,
                    theme::NavIcon::Sessions,
                    "Sessions",
                    Some(app.sessions.value.sessions.len().to_string()),
                );
                nav(
                    app,
                    ui,
                    Page::Ownership,
                    theme::NavIcon::Ownership,
                    "Ownership",
                    None,
                );
                nav(
                    app,
                    ui,
                    Page::Runs,
                    theme::NavIcon::Runs,
                    "Runs",
                    Some(app.runs.value.runs.len().to_string()),
                );
            });
            nav_group(ui, "ANALYZE", &mut |ui| {
                nav(
                    app,
                    ui,
                    Page::Triage,
                    theme::NavIcon::Triage,
                    "Triage",
                    None,
                );
                nav(
                    app,
                    ui,
                    Page::Compare,
                    theme::NavIcon::Compare,
                    "Compare",
                    None,
                );
            });
            nav_group(ui, "LIVE", &mut |ui| {
                nav(
                    app,
                    ui,
                    Page::Logcat,
                    theme::NavIcon::Logcat,
                    "Logcat",
                    None,
                );
                nav(
                    app,
                    ui,
                    Page::Inspector,
                    theme::NavIcon::Inspector,
                    "Inspector",
                    None,
                );
            });
            nav_group(ui, "SYSTEM", &mut |ui| {
                nav(
                    app,
                    ui,
                    Page::Maintenance,
                    theme::NavIcon::Maintenance,
                    "Maintenance",
                    None,
                );
                nav(
                    app,
                    ui,
                    Page::Settings,
                    theme::NavIcon::Settings,
                    "Settings",
                    None,
                );
            });
            ui.add_space(10.0);
            let dark = ui.style().visuals.dark_mode;
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
            let fill = if response.hovered() {
                if dark {
                    theme::DARK_SURFACE_ALT
                } else {
                    Color32::from_rgb(235, 239, 244)
                }
            } else if dark {
                theme::DARK_SURFACE
            } else {
                theme::LIGHT_SURFACE
            };
            ui.painter().rect_filled(rect, theme::ROW_RADIUS, fill);
            ui.painter().rect_stroke(
                rect,
                theme::ROW_RADIUS,
                Stroke::new(1.0, theme::border_color(ui)),
                StrokeKind::Inside,
            );
            let connected = app.cli.info.is_some();
            let color = if connected { theme::GREEN } else { theme::RED };
            let header_y = rect.top() + 13.0;
            let dot_center = egui::pos2(rect.left() + 13.0, header_y);
            ui.painter().circle_filled(dot_center, 3.0, color);
            ui.painter().text(
                egui::pos2(rect.left() + 22.0, header_y),
                egui::Align2::LEFT_CENTER,
                if connected {
                    "CLI connected"
                } else {
                    "CLI disconnected"
                },
                egui::FontId::proportional(10.5),
                color,
            );
            let version = app
                .cli
                .info
                .as_ref()
                .map(|info| info.version.as_str())
                .unwrap_or("-");
            ui.painter().text(
                egui::pos2(rect.right() - 10.0, header_y),
                egui::Align2::RIGHT_CENTER,
                version,
                egui::FontId::monospace(9.5),
                theme::MUTED,
            );
            let detail = app
                .cli
                .resolution
                .as_ref()
                .map(|resolution| resolution.path.display().to_string())
                .unwrap_or_else(|| "Run setup to connect the CLI".to_string());
            let detail_rect = egui::Rect::from_min_max(
                egui::pos2(rect.left() + 10.0, rect.top() + 22.0),
                egui::pos2(rect.right() - 10.0, rect.top() + 36.0),
            );
            let max_chars = ((detail_rect.width() / 5.0).floor() as usize).clamp(8, 60);
            ui.painter().with_clip_rect(detail_rect).text(
                detail_rect.left_center(),
                egui::Align2::LEFT_CENTER,
                truncate_middle(&detail, max_chars),
                egui::FontId::monospace(8.5),
                theme::MUTED,
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Button,
                    ui.is_enabled(),
                    "Open the AndroidForge CLI",
                )
            });
            if response
                .on_hover_text(format!("Open the AndroidForge CLI\n{detail}"))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                app.open_cli();
            }
        });
}

fn nav_group(ui: &mut egui::Ui, title: &str, add: &mut dyn FnMut(&mut egui::Ui)) {
    ui.add_space(8.0);
    ui.add_space(6.0);
    ui.label(
        RichText::new(title)
            .size(9.0)
            .strong()
            .color(theme::nav_muted(ui)),
    );
    ui.add_space(2.0);
    add(ui);
}

fn nav(
    app: &mut App,
    ui: &mut egui::Ui,
    page: Page,
    icon: theme::NavIcon,
    label: &str,
    badge: Option<String>,
) {
    let (response, rect) = nav_row(ui, app.page == page, icon, label, badge);
    app.nav_rects.insert(page, rect);
    if response.clicked() {
        app.set_page(page);
    }
}

fn nav_row(
    ui: &mut egui::Ui,
    selected: bool,
    icon: theme::NavIcon,
    label: &str,
    badge: Option<String>,
) -> (egui::Response, egui::Rect) {
    let desired = Vec2::new(ui.available_width(), 30.0);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let dark = ui.style().visuals.dark_mode;
    if !selected && response.hovered() {
        let fill = if dark {
            Color32::from_rgb(24, 31, 39)
        } else {
            Color32::from_rgb(242, 245, 249)
        };
        ui.painter()
            .rect_filled(rect.shrink2(Vec2::new(4.0, 1.0)), theme::ROW_RADIUS, fill);
    }
    let color = if selected {
        theme::text_color(ui)
    } else {
        theme::nav_muted(ui)
    };
    let icon_rect = egui::Rect::from_min_size(rect.min + Vec2::new(16.0, 7.5), Vec2::splat(15.0));
    theme::paint_nav_icon(
        ui.painter(),
        icon_rect,
        icon,
        if selected { theme::accent() } else { color },
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(44.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.5),
        color,
    );
    if let Some(badge) = badge {
        ui.painter().text(
            rect.right_center() - Vec2::new(16.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            badge,
            egui::FontId::monospace(9.5),
            theme::nav_muted(ui),
        );
    }
    (response, rect)
}

pub fn topbar(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal_centered(|ui| {
        ui.add_space(4.0);
        let page = app.page.label();
        ui.label(RichText::new(page).size(14.0).strong());
        ui.label(RichText::new("/").size(12.0).color(theme::MUTED));
        let mut project_choice = app.project_path.clone();
        let project_label = app
            .selected_project()
            .map(|project| project.display_name())
            .unwrap_or_else(|| "Select project".to_string());
        egui::ComboBox::from_id_salt("topbar-project")
            .width(180.0)
            .selected_text(RichText::new(project_label).size(11.5))
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
            if app
                .update
                .value
                .as_ref()
                .is_some_and(|update| update.update_available)
            {
                let accent = theme::accent();
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("Update available").size(10.0).color(accent),
                        )
                        .fill(Color32::from_rgba_unmultiplied(
                            accent.r(),
                            accent.g(),
                            accent.b(),
                            28,
                        ))
                        .stroke(Stroke::new(
                            1.0,
                            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 95),
                        ))
                        .corner_radius(theme::ROW_RADIUS)
                        .min_size(Vec2::new(0.0, 26.0)),
                    )
                    .clicked()
                {
                    app.open_setup(SetupSection::Updates);
                }
            }
            if refresh_button(ui).clicked() {
                app.manual_refresh(ui.ctx());
            }
            let connected = app.cli.info.is_some();
            topbar_status_pill(
                ui,
                if connected { "Online" } else { "Offline" },
                if connected { theme::GREEN } else { theme::RED },
            )
            .on_hover_text(
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
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("Desktop {}", env!("CARGO_PKG_VERSION")))
                    .size(9.5)
                    .color(theme::MUTED),
            );
        });
    });
}

pub fn page_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(theme::PAGE_TITLE_SIZE).strong());
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).size(11.5).color(theme::MUTED));
    }
    ui.add_space(8.0);
}

pub fn toolbar(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        add(ui);
    });
    ui.add_space(4.0);
}

pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(text.to_uppercase())
            .size(theme::SECTION_SIZE)
            .strong()
            .color(theme::MUTED),
    );
    ui.add_space(2.0);
}

pub fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).size(13.0).strong());
    ui.add_space(2.0);
}

pub fn muted(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(
        RichText::new(text)
            .size(theme::SMALL_SIZE)
            .color(theme::MUTED),
    );
}

pub fn small(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).size(9.5).color(theme::MUTED));
}

pub fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    card_min_height(ui, 0.0, add);
}

pub fn card_min_height(ui: &mut egui::Ui, min_height: f32, add: impl FnOnce(&mut egui::Ui)) {
    let dark = ui.style().visuals.dark_mode;
    egui::Frame::new()
        .fill(if dark {
            theme::DARK_SURFACE
        } else {
            theme::LIGHT_PANEL
        })
        .stroke(Stroke::new(1.0, theme::border_color(ui)))
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(egui::Margin::symmetric(16, 14))
        .show(ui, |ui| {
            let width = (ui.available_width() - theme::PAGE_GAP).max(0.0);
            ui.set_min_width(width);
            ui.set_max_width(width);
            if min_height > 0.0 {
                ui.set_min_height(min_height);
            }
            add(ui);
        });
}

/// A full-width rounded list row with a single hover/selection treatment.
/// It draws the background before the content, so child widgets stay readable.
pub fn selectable_row(
    ui: &mut egui::Ui,
    selected: bool,
    height: f32,
    add: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let full_width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(full_width, height), Sense::click());
    let hovered = response.hovered();
    let dark = ui.style().visuals.dark_mode;
    let (fill, stroke) = if selected {
        (theme::accent_soft(), theme::accent())
    } else if hovered {
        (
            theme::row_hover_fill(ui),
            if dark {
                Color32::from_rgb(72, 79, 88)
            } else {
                Color32::from_rgb(170, 184, 200)
            },
        )
    } else {
        (theme::row_fill(ui), theme::border_color(ui))
    };
    ui.painter().rect_filled(rect, theme::ROW_RADIUS, fill);
    ui.painter().rect_stroke(
        rect,
        theme::ROW_RADIUS,
        Stroke::new(1.0, stroke),
        StrokeKind::Inside,
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(12.0, 2.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    add(&mut child);
    response
}

pub fn row_cell(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, ui.available_height()),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_width(width);
            add(ui);
        },
    );
}

/// Split the usable row width into proportional columns. The row itself has
/// 12px inner padding on both sides and 10px between cells; headers use the
/// same values so they always line up.
pub fn proportional_widths(total: f32, ratios: &[f32], gap: f32) -> Vec<f32> {
    let gaps = gap * ratios.len().saturating_sub(1) as f32;
    let usable = (total - 24.0 - gaps).max(160.0);
    ratios.iter().map(|ratio| usable * ratio).collect()
}

pub fn table_header(ui: &mut egui::Ui, columns: &[(&str, f32)]) {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        for (label, width) in columns {
            ui.allocate_ui_with_layout(
                Vec2::new(*width, 20.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_min_width(*width);
                    ui.label(RichText::new(*label).size(9.5).strong().color(theme::MUTED));
                },
            );
        }
    });
    ui.add_space(3.0);
}

/// A card with a fixed outer height and an internal scroll area. Used for
/// side-by-side panels that must stay the same height regardless of content.
pub fn card_fixed_height(
    ui: &mut egui::Ui,
    height: f32,
    id: &str,
    add: impl FnOnce(&mut egui::Ui),
) {
    let dark = ui.style().visuals.dark_mode;
    egui::Frame::new()
        .fill(if dark {
            theme::DARK_SURFACE
        } else {
            theme::LIGHT_PANEL
        })
        .stroke(Stroke::new(1.0, theme::border_color(ui)))
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(egui::Margin::symmetric(16, 13))
        .show(ui, |ui| {
            let inner = egui::Vec2::new(
                (ui.available_width() - theme::PAGE_GAP).max(0.0),
                (height - 26.0).max(80.0),
            );
            ui.allocate_ui_with_layout(inner, egui::Layout::top_down(egui::Align::Min), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(id)
                    .max_height(inner.y)
                    .auto_shrink([false, false])
                    .show(ui, add);
            });
        });
}

pub fn banner(ui: &mut egui::Ui, text: &str, color: Color32) {
    let dark = ui.style().visuals.dark_mode;
    let bg = if dark {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 24)
    } else {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 18)
    };
    egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 110),
        ))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(9, 6))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.5).color(color));
        });
}

pub fn error_banner(ui: &mut egui::Ui, text: &str) {
    banner(ui, text, theme::RED);
}

pub fn success_banner(ui: &mut egui::Ui, text: &str) {
    banner(ui, text, theme::GREEN);
}

pub fn warning_banner(ui: &mut egui::Ui, text: &str) {
    banner(ui, text, theme::YELLOW);
}

pub fn info_banner(ui: &mut egui::Ui, text: &str) {
    banner(ui, text, theme::BLUE);
}

pub fn status_pill(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let dark = ui.style().visuals.dark_mode;
    let bg = if dark {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 24)
    } else {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 18)
    };
    egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 90),
        ))
        .corner_radius(9)
        .inner_margin(egui::Margin::symmetric(8, 1))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("● {}", theme::status_label(text)))
                    .size(10.5)
                    .color(color),
            );
        })
        .response
}

/// The topbar connection tag is explicitly sized to the same height as the
/// Refresh control, which keeps the two header actions visually balanced.
pub fn topbar_status_pill(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let dark = ui.style().visuals.dark_mode;
    let label = theme::status_label(text);
    let galley = ui
        .painter()
        .layout_no_wrap(label, egui::FontId::proportional(10.0), color);
    let dot = 6.0;
    let gap = 5.0;
    let padding = 10.0;
    let width = padding * 2.0 + dot + gap + galley.size().x;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, theme::CONTROL_HEIGHT), Sense::hover());
    let bg = if dark {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 24)
    } else {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 18)
    };
    let stroke = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 90);
    ui.painter()
        .rect(rect, 9, bg, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let dot_center = egui::pos2(rect.left() + padding + dot / 2.0, rect.center().y);
    ui.painter().circle_filled(dot_center, 2.5, color);
    let text_pos = egui::pos2(
        dot_center.x + dot / 2.0 + gap,
        rect.center().y - galley.size().y / 2.0,
    );
    ui.painter().galley(text_pos, galley, color);
    response
}

/// A rounded secondary button with a hand-painted refresh glyph. The glyph is
/// drawn with shapes because the bundled UI fonts do not contain the Unicode
/// refresh symbols. The label and icon are laid out explicitly so the button
/// stays compact instead of inheriting awkward leading spaces.
pub fn refresh_button(ui: &mut egui::Ui) -> egui::Response {
    let font = egui::FontId::proportional(11.0);
    let label = "Refresh";
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, theme::text_color(ui));
    let padding = 8.0;
    let icon = 13.0;
    let gap = 5.0;
    let width = padding * 2.0 + icon + gap + galley.size().x;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, theme::CONTROL_HEIGHT), Sense::click());
    let visuals = ui.style().interact(&response);
    ui.painter().rect(
        rect,
        theme::ROW_RADIUS,
        visuals.weak_bg_fill,
        visuals.bg_stroke,
        StrokeKind::Inside,
    );
    let icon_center = egui::pos2(rect.left() + padding + icon / 2.0, rect.center().y);
    paint_refresh_icon(ui.painter(), icon_center, 5.0, visuals.fg_stroke.color);
    let text_pos = egui::pos2(
        icon_center.x + icon / 2.0 + gap,
        rect.center().y - galley.size().y / 2.0,
    );
    ui.painter()
        .galley(text_pos, galley, visuals.fg_stroke.color);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Refresh"));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn paint_refresh_icon(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: Color32) {
    use std::f32::consts::{PI, TAU};
    let stroke = Stroke::new(1.4, color);
    let start = 5.0 * PI / 3.0;
    let end = 3.0 * PI / 2.0 + TAU;
    let steps = 20;
    let point = |angle: f32| center + egui::vec2(angle.cos() * radius, angle.sin() * radius);
    let mut previous = point(start);
    for step in 1..=steps {
        let angle = start + (end - start) * step as f32 / steps as f32;
        let next = point(angle);
        painter.line_segment([previous, next], stroke);
        previous = next;
    }
    let tangent = egui::vec2(-end.sin(), end.cos());
    let normal = egui::vec2(end.cos(), end.sin());
    let head = 4.0;
    let spread = 2.2;
    painter.add(egui::Shape::convex_polygon(
        vec![
            previous,
            previous - tangent * head + normal * spread,
            previous - tangent * head - normal * spread,
        ],
        color,
        Stroke::NONE,
    ));
}

/// A compact secondary action with the same rounded shape as the larger
/// buttons, used for Copy/Open and other inline row actions.
pub fn compact_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    // Let egui use the themed inactive/hovered widget visuals so the button
    // highlights on hover instead of being pinned to one fill color.
    ui.add(
        egui::Button::new(RichText::new(text).size(10.5))
            .corner_radius(theme::ROW_RADIUS)
            .min_size(Vec2::new(0.0, 24.0)),
    )
}

/// A left-aligned path with a real background and compact Copy/Open actions.
pub fn path_block(ui: &mut egui::Ui, path: &str) {
    let dark = ui.style().visuals.dark_mode;
    egui::Frame::new()
        .fill(if dark {
            Color32::from_rgb(8, 11, 15)
        } else {
            Color32::from_rgb(246, 248, 251)
        })
        .stroke(Stroke::new(1.0, theme::border_color(ui)))
        .corner_radius(theme::ROW_RADIUS)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 220.0).max(120.0);
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(width, 20.0), Sense::hover());
                let display = truncate_middle(path, 90);
                ui.painter().with_clip_rect(rect).text(
                    rect.left_center(),
                    egui::Align2::LEFT_CENTER,
                    display,
                    egui::FontId::monospace(9.5),
                    theme::MUTED,
                );
                response.on_hover_text(path);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if secondary_button(ui, "Open").clicked() {
                        let _ = open::that(path);
                    }
                    if secondary_button(ui, "Copy").clicked() {
                        ui.ctx().copy_text(path.to_string());
                    }
                });
            });
        });
}

pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let dark = ui.style().visuals.dark_mode;
    let text_color = if dark {
        Color32::from_rgb(20, 16, 8)
    } else {
        Color32::from_rgb(25, 20, 10)
    };
    ui.add(
        egui::Button::new(RichText::new(text).size(11.5).strong().color(text_color))
            .fill(theme::accent())
            .stroke(Stroke::new(1.0, theme::accent()))
            .corner_radius(theme::ROW_RADIUS)
            .min_size(Vec2::new(0.0, theme::CONTROL_HEIGHT)),
    )
}

pub fn secondary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    // Do not pin the fill/stroke here: the theme already defines inactive,
    // hovered and active widget visuals, and an explicit fill would suppress
    // the hover highlight on Refresh, Copy/Open and every other secondary
    // action.
    ui.add(
        egui::Button::new(RichText::new(text).size(11.5))
            .corner_radius(theme::ROW_RADIUS)
            .min_size(Vec2::new(0.0, theme::CONTROL_HEIGHT)),
    )
}

pub fn ghost_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(11.0).color(theme::MUTED))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .corner_radius(theme::ROW_RADIUS)
            .min_size(Vec2::new(0.0, theme::CONTROL_HEIGHT - 2.0)),
    )
}

pub fn danger_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(11.5).color(theme::RED))
            .fill(Color32::from_rgba_unmultiplied(255, 107, 107, 18))
            .stroke(Stroke::new(
                1.0,
                Color32::from_rgba_unmultiplied(255, 107, 107, 90),
            ))
            .corner_radius(theme::ROW_RADIUS)
            .min_size(Vec2::new(0.0, theme::CONTROL_HEIGHT)),
    )
}

pub fn icon_text_button(ui: &mut egui::Ui, label: &str, glyph: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(format!("{} {}", glyph, label))
                .size(10.0)
                .color(theme::MUTED),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE),
    )
}

pub fn settings_button(ui: &mut egui::Ui) -> egui::Response {
    let desired = Vec2::new(82.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    if response.hovered() {
        let fill = if ui.style().visuals.dark_mode {
            theme::DARK_SURFACE_ALT
        } else {
            theme::LIGHT_SURFACE
        };
        ui.painter().rect_filled(rect, 5.0, fill);
    }
    let icon_rect = egui::Rect::from_min_size(rect.min + Vec2::new(8.0, 6.0), Vec2::splat(14.0));
    theme::paint_gear_icon(ui.painter(), icon_rect, theme::MUTED);
    ui.painter().text(
        rect.left_center() + Vec2::new(28.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "Settings",
        egui::FontId::proportional(10.0),
        theme::MUTED,
    );
    response.on_hover_text("Open settings")
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

pub fn empty_state(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.add_space(26.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).size(14.0).strong());
        ui.label(RichText::new(detail).size(11.0).color(theme::MUTED));
    });
    ui.add_space(26.0);
}

pub fn key_value(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.label(RichText::new(key).size(10.0).color(theme::MUTED));
    ui.label(RichText::new(value).size(10.5));
}

pub fn kv_grid(ui: &mut egui::Ui, id: &str, rows: &[(&str, String)]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for (key, value) in rows {
                key_value(ui, key, value);
                ui.end_row();
            }
        });
}

pub fn copy_button(ui: &mut egui::Ui, value: &str, label: &str) {
    if compact_button(ui, label).clicked() {
        ui.ctx().copy_text(value.to_string());
    }
}

pub fn truncated_text(ui: &mut egui::Ui, value: &str, max: usize) {
    let short = truncate_middle(value, max);
    ui.add(egui::Label::new(RichText::new(short).size(10.5)).truncate())
        .on_hover_text(value);
}

pub fn path_row(ui: &mut egui::Ui, path: &str) {
    let reserve = 142.0;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - reserve).max(60.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::hover());
        let display = truncate_middle(path, 90);
        ui.painter().with_clip_rect(rect).text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            display,
            egui::FontId::monospace(9.5),
            theme::MUTED,
        );
        response.on_hover_text(path);
        if compact_button(ui, "Copy").clicked() {
            ui.ctx().copy_text(path.to_string());
        }
        if compact_button(ui, "Open").clicked() {
            let _ = open::that(path);
        }
    });
}

pub fn path_button(ui: &mut egui::Ui, path: &str) {
    if compact_button(ui, "Open").on_hover_text(path).clicked() {
        let _ = open::that(path);
    }
}

pub fn stale_note(ui: &mut egui::Ui, state: &DataState<impl Default>, label: &str) {
    if let Some(error) = &state.error {
        error_banner(ui, &format!("{label} is stale: {error}"));
    } else if let Some(age) = state.age()
        && age > std::time::Duration::from_secs(15)
    {
        muted(ui, format!("{label} updated {}s ago", age.as_secs()));
    }
}

pub fn project_strip(app: &App, ui: &mut egui::Ui) {
    let Some(project) = app.selected_project() else {
        muted(
            ui,
            "No project selected. Choose one in the top bar or open Setup.",
        );
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(project.display_name()).size(11.5).strong());
        ui.label(RichText::new("·").color(theme::MUTED));
        ui.label(
            RichText::new(project.package_label())
                .size(10.0)
                .color(theme::MUTED),
        );
        ui.label(RichText::new("·").color(theme::MUTED));
        status_pill(
            ui,
            &format!("adapter v{}", project.adapter_version),
            theme::GREEN,
        );
        if let Some(flow) = project.conformance_flow() {
            ui.label(RichText::new("·").color(theme::MUTED));
            ui.label(RichText::new(flow).size(9.5).color(theme::MUTED));
        }
    });
}

pub fn title_row(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    page_header(ui, title, subtitle);
}

pub fn metric(ui: &mut egui::Ui, label: &str, value: &str, color: Color32, detail: &str) {
    let dark = ui.style().visuals.dark_mode;
    egui::Frame::new()
        .fill(if dark {
            theme::DARK_SURFACE
        } else {
            theme::LIGHT_PANEL
        })
        .stroke(Stroke::new(1.0, theme::border_color(ui)))
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(egui::Margin::symmetric(16, 13))
        .show(ui, |ui| {
            ui.set_min_width((ui.available_width() - theme::PAGE_GAP).max(140.0));
            ui.set_min_height(60.0);
            ui.label(RichText::new(label).size(9.5).color(theme::MUTED));
            ui.label(RichText::new(value).size(22.0).strong().color(color));
            ui.label(RichText::new(detail).size(9.0).color(theme::MUTED));
        });
}

pub fn sparkline(ui: &mut egui::Ui, values: &[f32], color: Color32, height: f32) {
    let desired = Vec2::new(ui.available_width().max(40.0), height);
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
    if values.len() < 2 {
        return;
    }
    let min = values.iter().copied().fold(f32::INFINITY, f32::min);
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let span = (max - min).max(1.0);
    let points: Vec<egui::Pos2> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let x = rect.left() + rect.width() * index as f32 / (values.len() - 1) as f32;
            let y = rect.bottom() - 2.0 - (rect.height() - 4.0) * ((value - min) / span);
            egui::pos2(x, y)
        })
        .collect();
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(1.5, color)));
}

pub fn settings_path_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<String>,
    directory: bool,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(10.0).color(theme::MUTED));
        let mut text = value.clone().unwrap_or_default();
        let response = ui.add(
            egui::TextEdit::singleline(&mut text)
                .desired_width(320.0)
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
    kv_grid(
        ui,
        "confirm-session",
        &[
            ("Session", session.id.clone()),
            ("Owner", session.owner.clone()),
            (
                "Device",
                session.serial.clone().unwrap_or_else(|| "-".to_string()),
            ),
            (
                "AVD",
                session.avd.clone().unwrap_or_else(|| "-".to_string()),
            ),
        ],
    );
}

pub fn project_target_summary(app: &App, ui: &mut egui::Ui) {
    if let Some(project) = app.selected_project() {
        ui.label(
            RichText::new(format!(
                "Project: {} ({})",
                project.display_name(),
                project.package_label()
            ))
            .size(10.5)
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
            .size(10.5)
            .color(theme::MUTED),
        );
    } else {
        ui.colored_label(theme::RED, "No session selected.");
    }
}

pub fn divider(ui: &mut egui::Ui) {
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::border_color(ui));
    ui.add_space(6.0);
}

pub fn surface_frame(ui: &mut egui::Ui) -> egui::Frame {
    let dark = ui.style().visuals.dark_mode;
    egui::Frame::new()
        .fill(if dark {
            theme::DARK_SURFACE
        } else {
            theme::LIGHT_PANEL
        })
        .stroke(Stroke::new(1.0, theme::border_color(ui)))
        .corner_radius(8)
}

pub fn code_frame(ui: &mut egui::Ui, text: &str) {
    let dark = ui.style().visuals.dark_mode;
    egui::Frame::new()
        .fill(if dark {
            Color32::from_rgb(7, 10, 14)
        } else {
            Color32::from_rgb(246, 248, 251)
        })
        .stroke(Stroke::new(1.0, theme::border_color(ui)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .font(egui::FontId::monospace(10.0))
                        .color(if dark {
                            theme::TEXT
                        } else {
                            Color32::from_rgb(30, 40, 52)
                        }),
                )
                .wrap(),
            );
        });
}

pub fn segmented(ui: &mut egui::Ui, options: &[(&str, bool)], mut on_click: impl FnMut(usize)) {
    egui::Frame::new()
        .fill(theme::surface_fill(ui))
        .stroke(Stroke::new(1.0, theme::border_color(ui)))
        .corner_radius(7)
        .inner_margin(egui::Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (index, (label, active)) in options.iter().enumerate() {
                    let response = ui.add(
                        egui::Button::new(RichText::new(*label).size(10.0).color(if *active {
                            theme::text_color(ui)
                        } else {
                            theme::MUTED
                        }))
                        .fill(if *active {
                            if ui.style().visuals.dark_mode {
                                theme::DARK_SURFACE_ALT
                            } else {
                                theme::LIGHT_PANEL
                            }
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(Stroke::NONE),
                    );
                    if response.clicked() {
                        on_click(index);
                    }
                }
            });
        });
}

#[allow(dead_code)]
fn _keep_stroke_kind(_kind: StrokeKind) {}

pub fn code_block(ui: &mut egui::Ui, text: &str) {
    code_frame(ui, text);
}
