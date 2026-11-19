use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, StrokeKind,
};

use crate::state::ThemeMode;

const DEFAULT_ACCENT: Color32 = Color32::from_rgb(240, 166, 50);
static ACCENT_RGB: AtomicU32 = AtomicU32::new(0x00F0_A632);

/// The current accent. Settings can change it at runtime; all UI should use
/// this instead of a hard-coded constant.
pub fn accent() -> Color32 {
    let packed = ACCENT_RGB.load(Ordering::Relaxed);
    Color32::from_rgb(
        ((packed >> 16) & 0xFF) as u8,
        ((packed >> 8) & 0xFF) as u8,
        (packed & 0xFF) as u8,
    )
}

pub fn set_accent(color: Color32) {
    let packed = (u32::from(color.r()) << 16) | (u32::from(color.g()) << 8) | u32::from(color.b());
    ACCENT_RGB.store(packed, Ordering::Relaxed);
}

pub fn accent_soft() -> Color32 {
    let color = accent();
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 28)
}

/// Kept for compatibility with older call sites that only need the default.
pub const ACCENT: Color32 = DEFAULT_ACCENT;
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(60, 45, 22);

pub const CARD_RADIUS: u8 = 12;
pub const ROW_RADIUS: u8 = 8;
pub const CONTROL_HEIGHT: f32 = 30.0;
pub const PAGE_GAP: f32 = 14.0;
pub const ROW_HEIGHT: f32 = 38.0;
pub const BLUE: Color32 = Color32::from_rgb(104, 168, 255);
pub const GREEN: Color32 = Color32::from_rgb(74, 201, 112);
pub const RED: Color32 = Color32::from_rgb(255, 107, 107);
pub const YELLOW: Color32 = Color32::from_rgb(229, 180, 84);
pub const PURPLE: Color32 = Color32::from_rgb(177, 140, 255);
pub const MUTED: Color32 = Color32::from_rgb(139, 153, 171);
pub const TEXT: Color32 = Color32::from_rgb(231, 237, 245);

// GitHub-dark inspired palette. The canvas is close to true black, while
// panels and surfaces stay distinguishable without a blue cast.
pub const DARK_BG: Color32 = Color32::from_rgb(8, 11, 15);
pub const DARK_PANEL: Color32 = Color32::from_rgb(13, 17, 23);
pub const DARK_SURFACE: Color32 = Color32::from_rgb(22, 27, 34);
pub const DARK_SURFACE_ALT: Color32 = Color32::from_rgb(33, 38, 45);
pub const DARK_BORDER: Color32 = Color32::from_rgb(48, 54, 61);
pub const DARK_BORDER_SOFT: Color32 = Color32::from_rgb(30, 36, 44);

pub const LIGHT_BG: Color32 = Color32::from_rgb(242, 245, 249);
pub const LIGHT_PANEL: Color32 = Color32::from_rgb(255, 255, 255);
pub const LIGHT_SURFACE: Color32 = Color32::from_rgb(247, 249, 252);
pub const LIGHT_BORDER: Color32 = Color32::from_rgb(215, 224, 234);

pub const PAGE_TITLE_SIZE: f32 = 24.0;
pub const SECTION_SIZE: f32 = 10.5;
pub const BODY_SIZE: f32 = 13.0;
pub const SMALL_SIZE: f32 = 11.0;
pub const MONO_SIZE: f32 = 11.5;

pub fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "inter".to_string(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-Variable.ttf"
        ))),
    );
    fonts.font_data.insert(
        "jetbrains-mono".to_string(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/JetBrainsMono-Variable.ttf"
        ))),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".to_string());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "jetbrains-mono".to_string());
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context, mode: ThemeMode) {
    ctx.set_theme(mode.preference());
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        apply_style(style, dark_visuals())
    });
    ctx.style_mut_of(egui::Theme::Light, |style| {
        apply_style(style, light_visuals())
    });
}

fn apply_style(style: &mut egui::Style, visuals: egui::Visuals) {
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(10.0, 7.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.interact_size.y = CONTROL_HEIGHT;
    // Solid, non-floating scrollbars reserve their own column. This keeps the
    // scrollbar from covering cards or buttons on every page.
    style.spacing.scroll = egui::style::ScrollStyle {
        floating: false,
        bar_width: 10.0,
        handle_min_length: 28.0,
        bar_inner_margin: 6.0,
        bar_outer_margin: 2.0,
        ..Default::default()
    };
    style.text_styles.insert(
        egui::TextStyle::Heading,
        FontId::new(18.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        FontId::new(BODY_SIZE, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        FontId::new(12.5, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        FontId::new(SMALL_SIZE, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        FontId::new(MONO_SIZE, FontFamily::Monospace),
    );
}

fn dark_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = DARK_BG;
    visuals.window_fill = DARK_PANEL;
    visuals.faint_bg_color = DARK_SURFACE;
    visuals.extreme_bg_color = Color32::from_rgb(6, 9, 13);
    visuals.override_text_color = Some(TEXT);
    visuals.hyperlink_color = BLUE;
    visuals.selection.bg_fill = accent_soft();
    visuals.selection.stroke = Stroke::new(1.0, accent());
    visuals.widgets.noninteractive.bg_fill = DARK_PANEL;
    visuals.widgets.noninteractive.weak_bg_fill = DARK_PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, DARK_BORDER);
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(ROW_RADIUS);
    visuals.widgets.inactive.bg_fill = DARK_SURFACE;
    visuals.widgets.inactive.weak_bg_fill = DARK_SURFACE;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, DARK_BORDER);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(ROW_RADIUS);
    visuals.widgets.hovered.bg_fill = DARK_SURFACE_ALT;
    visuals.widgets.hovered.weak_bg_fill = DARK_SURFACE_ALT;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(72, 79, 88));
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(ROW_RADIUS);
    visuals.widgets.active.bg_fill = Color32::from_rgb(38, 44, 52);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(38, 44, 52);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, accent());
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(ROW_RADIUS);
    visuals.widgets.open.bg_fill = DARK_SURFACE_ALT;
    visuals.window_stroke = Stroke::new(1.0, DARK_BORDER);
    visuals.window_corner_radius = egui::CornerRadius::same(CARD_RADIUS);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    visuals
}

fn light_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = LIGHT_BG;
    visuals.window_fill = LIGHT_PANEL;
    visuals.faint_bg_color = LIGHT_SURFACE;
    visuals.extreme_bg_color = Color32::WHITE;
    visuals.override_text_color = Some(Color32::from_rgb(22, 32, 45));
    visuals.hyperlink_color = Color32::from_rgb(20, 100, 200);
    visuals.selection.bg_fill = accent_soft();
    visuals.selection.stroke = Stroke::new(1.0, accent());
    visuals.widgets.noninteractive.bg_fill = LIGHT_PANEL;
    visuals.widgets.noninteractive.weak_bg_fill = LIGHT_PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LIGHT_BORDER);
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(ROW_RADIUS);
    visuals.widgets.inactive.bg_fill = LIGHT_SURFACE;
    visuals.widgets.inactive.weak_bg_fill = LIGHT_SURFACE;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, LIGHT_BORDER);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(ROW_RADIUS);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(228, 235, 244);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(228, 235, 244);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(170, 184, 200));
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(ROW_RADIUS);
    visuals.widgets.active.bg_fill = Color32::from_rgb(214, 224, 236);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(214, 224, 236);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, accent());
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(ROW_RADIUS);
    visuals.window_stroke = Stroke::new(1.0, LIGHT_BORDER);
    visuals.window_corner_radius = egui::CornerRadius::same(CARD_RADIUS);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(40),
    };
    visuals
}

pub fn panel_fill(ui: &egui::Ui) -> Color32 {
    if ui.style().visuals.dark_mode {
        DARK_PANEL
    } else {
        LIGHT_PANEL
    }
}

pub fn surface_fill(ui: &egui::Ui) -> Color32 {
    if ui.style().visuals.dark_mode {
        DARK_SURFACE
    } else {
        LIGHT_SURFACE
    }
}

pub fn text_color(ui: &egui::Ui) -> Color32 {
    if ui.style().visuals.dark_mode {
        TEXT
    } else {
        Color32::from_rgb(22, 32, 45)
    }
}

pub fn nav_muted(ui: &egui::Ui) -> Color32 {
    if ui.style().visuals.dark_mode {
        MUTED
    } else {
        Color32::from_rgb(82, 97, 117)
    }
}

/// Row tables sit on top of cards or the page background, so they need a
/// visibly raised surface rather than a fill that blends into the parent.
pub fn row_fill(ui: &egui::Ui) -> Color32 {
    if ui.style().visuals.dark_mode {
        Color32::from_rgb(33, 39, 47)
    } else {
        Color32::from_rgb(241, 246, 252)
    }
}

pub fn row_hover_fill(ui: &egui::Ui) -> Color32 {
    if ui.style().visuals.dark_mode {
        Color32::from_rgb(41, 48, 58)
    } else {
        Color32::from_rgb(231, 238, 247)
    }
}

pub fn border_color(ui: &egui::Ui) -> Color32 {
    if ui.style().visuals.dark_mode {
        DARK_BORDER
    } else {
        LIGHT_BORDER
    }
}

/// Make an egui slider clearly visible against both themes. The default rail
/// is only a couple of pixels tall and disappears on the dark surface.
pub fn configure_slider(ui: &mut egui::Ui, width: f32) {
    let dark = ui.style().visuals.dark_mode;
    ui.visuals_mut().widgets.inactive.bg_fill = if dark {
        DARK_SURFACE_ALT
    } else {
        Color32::from_rgb(220, 228, 238)
    };
    ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0, border_color(ui));
    ui.visuals_mut().slider_trailing_fill = true;
    ui.visuals_mut().selection.bg_fill = accent();
    ui.spacing_mut().slider_width = width;
    ui.spacing_mut().slider_rail_height = 6.0;
}

pub fn status_color(status: &str) -> Color32 {
    match status.to_ascii_lowercase().as_str() {
        "ok" | "success" | "passed" | "ready" | "device" | "completed" | "active" | "connected"
        | "stable_success" => GREEN,
        "warn"
        | "warning"
        | "degraded"
        | "running"
        | "pending"
        | "attached"
        | "insufficient_evidence" => YELLOW,
        "error" | "failed" | "failure" | "offline" | "unauthorized" | "aborted"
        | "stable_failure" | "regressed" => RED,
        "cancelled" | "canceled" => PURPLE,
        "interrupted" | "timed_out" | "timed-out" | "abandoned" | "partial" => BLUE,
        "released" | "skipped" | "unchanged" | "fixed" | "added" | "removed" => MUTED,
        "flaky" => YELLOW,
        _ => MUTED,
    }
}

pub fn status_label(status: &str) -> String {
    let normalized = status.replace(['_', '-'], " ");
    let mut chars = normalized.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

pub fn accent_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

pub fn load_icon_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let image = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .expect("embedded icon must be a valid PNG")
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    ctx.load_texture(
        "androidforge-icon",
        color_image,
        egui::TextureOptions::LINEAR,
    )
}

pub fn icon_data() -> Arc<egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .expect("embedded icon must be a valid PNG")
        .to_rgba8();
    let (width, height) = image.dimensions();
    Arc::new(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

/// Draw a settings gear without relying on a font glyph, so it renders the
/// same on every Windows font fallback configuration.
pub fn paint_gear_icon(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.36;
    let stroke = Stroke::new(1.4, color);
    painter.circle_stroke(center, radius, stroke);
    painter.circle_filled(center, radius * 0.28, color);
    for index in 0..8 {
        let angle = std::f32::consts::TAU * index as f32 / 8.0;
        let direction = egui::vec2(angle.cos(), angle.sin());
        painter.line_segment(
            [
                center + direction * (radius + 0.5),
                center + direction * (radius + 3.0),
            ],
            stroke,
        );
    }
}

#[derive(Debug, Clone, Copy)]
pub enum NavIcon {
    Setup,
    Overview,
    Context,
    Matrix,
    Devices,
    Sessions,
    Ownership,
    Runs,
    Triage,
    Compare,
    Logcat,
    Inspector,
    Maintenance,
    Settings,
}

pub fn paint_nav_icon(painter: &egui::Painter, rect: egui::Rect, icon: NavIcon, color: Color32) {
    let r = rect.shrink(3.0);
    let stroke = Stroke::new(1.5, color);
    let faint = Stroke::new(1.0, color);
    match icon {
        NavIcon::Setup => {
            painter.circle_stroke(r.center(), r.width().min(r.height()) / 2.0, stroke);
            painter.line_segment(
                [
                    egui::pos2(r.center().x, r.top() + 2.0),
                    egui::pos2(r.center().x + 2.5, r.center().y + 1.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(r.center().x + 2.5, r.center().y + 1.0),
                    egui::pos2(r.center().x - 2.0, r.bottom() - 2.0),
                ],
                stroke,
            );
        }
        NavIcon::Overview => {
            let gap = 2.0;
            let w = (r.width() - gap) / 2.0;
            let h = (r.height() - gap) / 2.0;
            for (x, y) in [
                (0.0, 0.0),
                (w + gap, 0.0),
                (0.0, h + gap),
                (w + gap, h + gap),
            ] {
                let cell = egui::Rect::from_min_size(r.min + egui::vec2(x, y), egui::vec2(w, h));
                painter.rect_filled(cell, 1.5, color);
            }
        }
        NavIcon::Devices => {
            let phone = r.shrink2(egui::vec2(2.5, 0.0));
            painter.rect_stroke(phone, 2.0, stroke, StrokeKind::Inside);
            painter.line_segment(
                [
                    egui::pos2(phone.center().x - 2.0, phone.bottom() - 1.5),
                    egui::pos2(phone.center().x + 2.0, phone.bottom() - 1.5),
                ],
                faint,
            );
        }
        NavIcon::Context => {
            painter.rect_stroke(
                r.shrink2(egui::vec2(1.5, 0.0)),
                2.0,
                stroke,
                StrokeKind::Inside,
            );
            for index in 0..3 {
                let y = r.top() + 4.0 + index as f32 * 3.5;
                painter.line_segment(
                    [
                        egui::pos2(r.left() + 4.0, y),
                        egui::pos2(r.right() - 3.0, y),
                    ],
                    faint,
                );
            }
        }
        NavIcon::Matrix => {
            for (x, y) in [
                (0.0, 0.0),
                (r.width() / 2.0 + 1.0, 0.0),
                (0.0, r.height() / 2.0 + 1.0),
                (r.width() / 2.0 + 1.0, r.height() / 2.0 + 1.0),
            ] {
                let rect = egui::Rect::from_min_size(
                    r.min + egui::vec2(x, y),
                    egui::vec2(r.width() / 2.0 - 1.0, r.height() / 2.0 - 1.0),
                );
                painter.rect_stroke(rect, 1.5, faint, StrokeKind::Inside);
            }
        }
        NavIcon::Ownership => {
            painter.circle_stroke(egui::pos2(r.left() + 5.0, r.top() + 5.0), 2.6, stroke);
            painter.circle_stroke(egui::pos2(r.right() - 5.0, r.top() + 5.0), 2.6, stroke);
            painter.line_segment(
                [
                    egui::pos2(r.left() + 2.0, r.bottom() - 2.0),
                    egui::pos2(r.left() + 5.0, r.center().y + 3.0),
                ],
                faint,
            );
            painter.line_segment(
                [
                    egui::pos2(r.right() - 2.0, r.bottom() - 2.0),
                    egui::pos2(r.right() - 5.0, r.center().y + 3.0),
                ],
                faint,
            );
        }
        NavIcon::Sessions => {
            painter.circle_stroke(egui::pos2(r.left() + 4.0, r.center().y), 3.0, stroke);
            painter.circle_stroke(egui::pos2(r.right() - 4.0, r.center().y), 3.0, stroke);
            painter.line_segment(
                [
                    egui::pos2(r.left() + 7.0, r.center().y),
                    egui::pos2(r.right() - 7.0, r.center().y),
                ],
                faint,
            );
        }
        NavIcon::Runs => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(r.left() + 3.0, r.top() + 1.0),
                    egui::pos2(r.right() - 1.0, r.center().y),
                    egui::pos2(r.left() + 3.0, r.bottom() - 1.0),
                ],
                color,
                Stroke::NONE,
            ));
            painter.line_segment(
                [
                    egui::pos2(r.right() - 5.0, r.top() + 2.0),
                    egui::pos2(r.right() - 5.0, r.bottom() - 2.0),
                ],
                faint,
            );
        }
        NavIcon::Triage => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(r.center().x, r.top()),
                    egui::pos2(r.right(), r.bottom()),
                    egui::pos2(r.left(), r.bottom()),
                ],
                Color32::TRANSPARENT,
                stroke,
            ));
            painter.line_segment(
                [
                    egui::pos2(r.center().x, r.top() + 4.0),
                    egui::pos2(r.center().x, r.center().y + 1.0),
                ],
                stroke,
            );
            painter.circle_filled(egui::pos2(r.center().x, r.bottom() - 2.5), 1.2, color);
        }
        NavIcon::Compare => {
            painter.rect_stroke(
                egui::Rect::from_min_max(r.left_top(), egui::pos2(r.center().x - 2.0, r.bottom())),
                2.0,
                stroke,
                StrokeKind::Inside,
            );
            painter.rect_stroke(
                egui::Rect::from_min_max(egui::pos2(r.center().x + 2.0, r.top()), r.right_bottom()),
                2.0,
                stroke,
                StrokeKind::Inside,
            );
            painter.line_segment(
                [
                    egui::pos2(r.center().x - 1.0, r.center().y - 2.0),
                    egui::pos2(r.center().x + 1.0, r.center().y + 2.0),
                ],
                faint,
            );
        }
        NavIcon::Logcat => {
            painter.line_segment(
                [
                    egui::pos2(r.left() + 1.0, r.top() + 3.0),
                    egui::pos2(r.center().x - 1.0, r.center().y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(r.center().x - 1.0, r.center().y),
                    egui::pos2(r.left() + 1.0, r.bottom() - 3.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(r.center().x + 3.0, r.bottom() - 3.0),
                    egui::pos2(r.right() - 1.0, r.bottom() - 3.0),
                ],
                stroke,
            );
        }
        NavIcon::Inspector => {
            painter.rect_stroke(r.shrink(1.0), 2.0, stroke, StrokeKind::Inside);
            painter.circle_stroke(r.center(), r.width() / 5.0, faint);
            painter.line_segment(
                [
                    egui::pos2(r.center().x, r.top() - 1.0),
                    egui::pos2(r.center().x, r.bottom() + 1.0),
                ],
                faint,
            );
            painter.line_segment(
                [
                    egui::pos2(r.left() - 1.0, r.center().y),
                    egui::pos2(r.right() + 1.0, r.center().y),
                ],
                faint,
            );
        }
        NavIcon::Maintenance => {
            painter.line_segment(
                [
                    egui::pos2(r.left() + 3.0, r.bottom() - 2.0),
                    egui::pos2(r.right() - 3.0, r.top() + 2.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(r.left() + 2.0, r.bottom() - 5.0),
                    egui::pos2(r.left() + 5.0, r.bottom() - 2.0),
                ],
                faint,
            );
            painter.line_segment(
                [
                    egui::pos2(r.right() - 6.0, r.top() + 1.0),
                    egui::pos2(r.right() - 1.0, r.top() + 4.0),
                ],
                faint,
            );
        }
        NavIcon::Settings => {
            painter.circle_stroke(r.center(), r.width() / 3.6, stroke);
            painter.circle_filled(r.center(), 1.6, color);
            for angle in [0.0_f32, 60.0, 120.0, 180.0, 240.0, 300.0] {
                let radians = angle.to_radians();
                let dir = egui::vec2(radians.cos(), radians.sin());
                painter.line_segment(
                    [
                        r.center() + dir * (r.width() / 3.6),
                        r.center() + dir * (r.width() / 2.2),
                    ],
                    faint,
                );
            }
        }
    }
}
