use std::sync::Arc;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke};

use crate::state::ThemeMode;

pub const ACCENT: Color32 = Color32::from_rgb(245, 165, 36);
pub const BLUE: Color32 = Color32::from_rgb(88, 166, 255);
pub const GREEN: Color32 = Color32::from_rgb(63, 185, 80);
pub const RED: Color32 = Color32::from_rgb(248, 81, 73);
pub const YELLOW: Color32 = Color32::from_rgb(210, 153, 34);
pub const MUTED: Color32 = Color32::from_rgb(139, 148, 167);

pub const DARK_BG: Color32 = Color32::from_rgb(11, 14, 19);
pub const DARK_PANEL: Color32 = Color32::from_rgb(17, 21, 29);
pub const DARK_SURFACE: Color32 = Color32::from_rgb(22, 27, 36);
pub const DARK_BORDER: Color32 = Color32::from_rgb(37, 44, 57);

pub const LIGHT_BG: Color32 = Color32::from_rgb(246, 247, 249);
pub const LIGHT_PANEL: Color32 = Color32::from_rgb(255, 255, 255);
pub const LIGHT_SURFACE: Color32 = Color32::from_rgb(240, 242, 245);
pub const LIGHT_BORDER: Color32 = Color32::from_rgb(208, 215, 222);

pub fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut loaded = false;

    #[cfg(windows)]
    {
        if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
            fonts.font_data.insert(
                "segoe-ui".to_string(),
                Arc::new(FontData::from_owned(bytes)),
            );
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "segoe-ui".to_string());
            loaded = true;
        }
        if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\consola.ttf") {
            fonts.font_data.insert(
                "consolas".to_string(),
                Arc::new(FontData::from_owned(bytes)),
            );
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .insert(0, "consolas".to_string());
            loaded = true;
        }
    }

    if loaded {
        ctx.set_fonts(fonts);
    }
}

pub fn apply(ctx: &egui::Context, mode: ThemeMode) {
    ctx.set_theme(mode.preference());
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.visuals = dark_visuals();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(9.0, 4.0);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.interact_size.y = 24.0;
        style.text_styles.insert(
            egui::TextStyle::Heading,
            FontId::new(18.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            FontId::new(13.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            FontId::new(12.0, FontFamily::Monospace),
        );
    });
    ctx.style_mut_of(egui::Theme::Light, |style| {
        style.visuals = light_visuals();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(9.0, 4.0);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.interact_size.y = 24.0;
        style.text_styles.insert(
            egui::TextStyle::Heading,
            FontId::new(18.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            FontId::new(13.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            FontId::new(12.0, FontFamily::Monospace),
        );
    });
}

fn dark_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = DARK_BG;
    visuals.window_fill = DARK_PANEL;
    visuals.faint_bg_color = DARK_SURFACE;
    visuals.extreme_bg_color = Color32::from_rgb(8, 11, 15);
    visuals.override_text_color = Some(Color32::from_rgb(230, 234, 242));
    visuals.hyperlink_color = BLUE;
    visuals.selection.bg_fill = Color32::from_rgb(48, 63, 85);
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(200, 220, 255));
    visuals.widgets.noninteractive.bg_fill = DARK_PANEL;
    visuals.widgets.noninteractive.weak_bg_fill = DARK_PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, DARK_BORDER);
    visuals.widgets.inactive.bg_fill = DARK_SURFACE;
    visuals.widgets.inactive.weak_bg_fill = DARK_SURFACE;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, DARK_BORDER);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(31, 38, 50);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(31, 38, 50);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(60, 72, 92));
    visuals.widgets.active.bg_fill = Color32::from_rgb(38, 47, 62);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(38, 47, 62);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.open.bg_fill = DARK_SURFACE;
    visuals.window_stroke = Stroke::new(1.0, DARK_BORDER);
    visuals
}

fn light_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = LIGHT_BG;
    visuals.window_fill = LIGHT_PANEL;
    visuals.faint_bg_color = LIGHT_SURFACE;
    visuals.extreme_bg_color = Color32::WHITE;
    visuals.override_text_color = Some(Color32::from_rgb(31, 35, 40));
    visuals.hyperlink_color = Color32::from_rgb(9, 105, 218);
    visuals.selection.bg_fill = Color32::from_rgb(194, 222, 255);
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(20, 60, 110));
    visuals.widgets.noninteractive.bg_fill = LIGHT_PANEL;
    visuals.widgets.noninteractive.weak_bg_fill = LIGHT_PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LIGHT_BORDER);
    visuals.widgets.inactive.bg_fill = LIGHT_SURFACE;
    visuals.widgets.inactive.weak_bg_fill = LIGHT_SURFACE;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, LIGHT_BORDER);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(228, 233, 240);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(228, 233, 240);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(160, 172, 188));
    visuals.widgets.active.bg_fill = Color32::from_rgb(214, 222, 232);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(214, 222, 232);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.window_stroke = Stroke::new(1.0, LIGHT_BORDER);
    visuals
}

pub fn status_color(status: &str) -> Color32 {
    match status.to_ascii_lowercase().as_str() {
        "ok" | "success" | "passed" | "ready" | "device" | "completed" | "active" => GREEN,
        "warn" | "warning" | "degraded" | "running" | "pending" | "attached" => YELLOW,
        "error" | "failed" | "failure" | "offline" | "unauthorized" | "aborted" => RED,
        "released" | "interrupted" | "partial" | "cancelled" => MUTED,
        _ => MUTED,
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

#[derive(Debug, Clone, Copy)]
pub enum NavIcon {
    Overview,
    Devices,
    Sessions,
    Runs,
}

pub fn paint_nav_icon(painter: &egui::Painter, rect: egui::Rect, icon: NavIcon, color: Color32) {
    let r = rect.shrink(2.0);
    match icon {
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
            let phone = r.shrink2(egui::vec2(2.0, 0.0));
            let stroke = Stroke::new(1.5, color);
            painter.line_segment([phone.left_top(), phone.right_top()], stroke);
            painter.line_segment([phone.right_top(), phone.right_bottom()], stroke);
            painter.line_segment([phone.right_bottom(), phone.left_bottom()], stroke);
            painter.line_segment([phone.left_bottom(), phone.left_top()], stroke);
            painter.circle_filled(
                egui::pos2(phone.center().x, phone.bottom() - 2.0),
                1.0,
                color,
            );
        }
        NavIcon::Sessions => {
            painter.circle_filled(egui::pos2(r.left() + 4.0, r.center().y), 3.0, color);
            painter.circle_filled(egui::pos2(r.right() - 4.0, r.center().y), 3.0, color);
            painter.line_segment(
                [
                    egui::pos2(r.left() + 6.0, r.center().y),
                    egui::pos2(r.right() - 6.0, r.center().y),
                ],
                Stroke::new(1.2, color),
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
        }
    }
}
