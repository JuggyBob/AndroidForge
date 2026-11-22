pub mod compare;
pub mod context;
pub mod devices;
pub mod inspector;
pub mod logcat;
pub mod maintenance;
pub mod matrix;
pub mod operations;
pub mod overview;
pub mod ownership;
pub mod run_details;
pub mod runs;
pub mod sessions;
pub mod settings;
pub mod setup;
pub mod tools;
pub mod triage;

use eframe::egui;

use crate::app::App;
use crate::state::Page;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    // A short slide-up and fade makes page changes feel deliberate instead of
    // snapping. The animation only runs for a quarter of a second.
    let elapsed = app.page_animation_started.elapsed().as_secs_f32();
    let progress = (elapsed / 0.26).clamp(0.0, 1.0);
    if progress < 1.0 {
        let eased = 1.0 - (1.0 - progress).powi(3);
        ui.add_space((1.0 - eased) * 22.0);
        ui.set_opacity(0.2 + 0.8 * eased);
        ui.ctx().request_repaint();
    }
    match app.page {
        Page::Setup => setup::show(app, ui),
        Page::Tools => tools::show(app, ui),
        Page::Overview => overview::show(app, ui),
        Page::Context => context::show(app, ui),
        Page::Matrix => matrix::show(app, ui),
        Page::Devices => devices::show(app, ui),
        Page::Sessions => sessions::show(app, ui),
        Page::Ownership => ownership::show(app, ui),
        Page::Runs => runs::show(app, ui),
        Page::Triage => triage::show(app, ui),
        Page::Compare => compare::show(app, ui),
        Page::Logcat => logcat::show(app, ui),
        Page::Inspector => inspector::show(app, ui),
        Page::Maintenance => maintenance::show(app, ui),
        Page::Settings => settings::show(app, ui),
    }
    if progress < 1.0 {
        ui.set_opacity(1.0);
    }
}

pub fn format_duration(milliseconds: u64) -> String {
    if milliseconds < 1000 {
        format!("{milliseconds}ms")
    } else if milliseconds < 60_000 {
        format!("{:.1}s", milliseconds as f64 / 1000.0)
    } else {
        format!(
            "{}m {}s",
            milliseconds / 60_000,
            (milliseconds % 60_000) / 1000
        )
    }
}
