#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use androidforge_desktop::app::App;
use androidforge_desktop::theme;
use eframe::egui;

fn main() -> eframe::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "AndroidForge desktop {}\n\nUsage:\n  androidforge-desktop.exe\n\nOptions:\n  -h, --help       Print this help and exit\n  -V, --version    Print the desktop version and exit\n\nThe desktop is a GUI; all AndroidForge operations go through the adjacent\nandroidforge.exe CLI sidecar.",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("androidforge-desktop {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if let Some(argument) = args.first() {
        eprintln!("androidforge-desktop: unknown argument '{argument}'");
        eprintln!("Run `androidforge-desktop.exe --help` for usage.");
        std::process::exit(2);
    }
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
