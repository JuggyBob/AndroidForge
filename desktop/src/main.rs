#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use androidforge_desktop::app::App;
use androidforge_desktop::theme;
use eframe::egui;

fn main() -> eframe::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "AndroidForge desktop {}\n\nUsage:\n  androidforge-desktop.exe [--page <name>]\n\nOptions:\n  --page <name>    Open a page: setup, tools, overview, context, devices, sessions,\n                   matrix, ownership, runs, triage, compare, logcat, inspector, maintenance, settings\n  -h, --help       Print this help and exit\n  -V, --version    Print the desktop version and exit\n\nThe desktop is a GUI; all AndroidForge operations go through the adjacent\nandroidforge.exe CLI sidecar.",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("androidforge-desktop {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let mut initial_page = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--page" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("androidforge-desktop: --page requires a page name");
                    std::process::exit(2);
                };
                initial_page = Some(parse_page(value));
                index += 2;
            }
            argument => {
                eprintln!("androidforge-desktop: unknown argument '{argument}'");
                eprintln!("Run `androidforge-desktop.exe --help` for usage.");
                std::process::exit(2);
            }
        }
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
        Box::new(move |cc| Ok(Box::new(App::new_with_page(cc, initial_page)))),
    )
}

fn parse_page(value: &str) -> androidforge_desktop::state::Page {
    match value.to_ascii_lowercase().as_str() {
        "setup" => androidforge_desktop::state::Page::Setup,
        "tools" => androidforge_desktop::state::Page::Tools,
        "overview" => androidforge_desktop::state::Page::Overview,
        "context" => androidforge_desktop::state::Page::Context,
        "matrix" => androidforge_desktop::state::Page::Matrix,
        "devices" => androidforge_desktop::state::Page::Devices,
        "sessions" => androidforge_desktop::state::Page::Sessions,
        "ownership" => androidforge_desktop::state::Page::Ownership,
        "runs" => androidforge_desktop::state::Page::Runs,
        "triage" => androidforge_desktop::state::Page::Triage,
        "compare" => androidforge_desktop::state::Page::Compare,
        "logcat" => androidforge_desktop::state::Page::Logcat,
        "inspector" => androidforge_desktop::state::Page::Inspector,
        "maintenance" => androidforge_desktop::state::Page::Maintenance,
        "settings" => androidforge_desktop::state::Page::Settings,
        other => {
            eprintln!(
                "androidforge-desktop: unknown page '{other}'; use --help to list page names"
            );
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_database_page() {
        assert_eq!(
            parse_page("tools"),
            androidforge_desktop::state::Page::Tools
        );
    }
}
