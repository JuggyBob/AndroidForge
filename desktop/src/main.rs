#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use androidforge_desktop::app::App;
use androidforge_desktop::theme;
use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([1024.0, 700.0])
            .with_icon(theme::icon_data())
            .with_title("AndroidForge"),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "AndroidForge",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
