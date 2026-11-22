use eframe::egui;
use egui::RichText;

use crate::app::App;
use crate::components::*;
use crate::models::{CatalogArg, CatalogCommand};
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    if app.tools.catalog.is_none() && !app.tools.loading && app.tools.error.is_none() {
        app.refresh_catalog(ui.ctx());
    }
    if app.tools.selected.is_some() {
        detail(app, ui);
    } else {
        list(app, ui);
    }
}

fn list(app: &mut App, ui: &mut egui::Ui) {
    page_header(
        ui,
        "Tool database",
        "Every CLI command, its arguments and examples. Generated from the same clap parser the agent uses.",
    );
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.tools.query)
                .hint_text("Search commands, arguments or summaries...")
                .desired_width(380.0),
        );
        if secondary_button(ui, if app.tools.ascending { "A-Z" } else { "Z-A" })
            .on_hover_text("Toggle alphabetical order")
            .clicked()
        {
            app.tools.ascending = !app.tools.ascending;
        }
        if secondary_button(ui, "Refresh").clicked() {
            app.tools.catalog = None;
            app.refresh_catalog(ui.ctx());
        }
    });
    ui.add_space(6.0);
    if app.tools.loading {
        ui.horizontal(|ui| {
            ui.spinner();
            muted(ui, "Loading the command catalog...");
        });
        return;
    }
    if let Some(error) = &app.tools.error {
        error_banner(ui, error);
        return;
    }
    let Some(catalog) = app.tools.catalog.clone() else {
        return;
    };
    let query = app.tools.query.trim().to_ascii_lowercase();
    let mut commands = catalog
        .commands
        .into_iter()
        .filter(|command| matches_query(command, &query))
        .collect::<Vec<_>>();
    commands.sort_by(|left, right| {
        if app.tools.ascending {
            left.path.cmp(&right.path)
        } else {
            right.path.cmp(&left.path)
        }
    });
    if commands.is_empty() {
        empty_state(
            ui,
            "No matching commands",
            "Try a command group such as `app`, `ui`, `session` or clear the search.",
        );
        return;
    }
    ui.horizontal(|ui| {
        small(ui, format!("{} command(s)", commands.len()));
        if !query.is_empty() {
            status_pill(ui, "filtered", theme::BLUE);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            small(ui, "Click a row for full documentation");
        });
    });
    ui.add_space(4.0);
    let width = ui.available_width();
    let widths = proportional_widths(width, &[0.30, 0.13, 0.57], 10.0);
    table_header(
        ui,
        &[
            ("Tool", widths[0]),
            ("Safety", widths[1]),
            ("Summary", widths[2]),
        ],
    );
    let mut open = None;
    for command in &commands {
        let selected = false;
        let response = selectable_row(ui, selected, theme::ROW_HEIGHT, |ui| {
            row_cell(ui, widths[0], |ui| {
                ui.add(
                    egui::Label::new(RichText::new(&command.path).size(10.5).monospace())
                        .truncate(),
                );
            });
            row_cell(ui, widths[1], |ui| {
                status_pill(ui, &command.safety, safety_color(&command.safety));
            });
            row_cell(ui, widths[2], |ui| {
                ui.add(egui::Label::new(RichText::new(&command.summary).size(10.0)).truncate());
            });
        });
        if response.clicked() {
            open = Some(command.path.clone());
        }
    }
    if let Some(path) = open {
        app.tools.selected = Some(path);
        app.tools.detail_started = Some(std::time::Instant::now());
    }
}

fn detail(app: &mut App, ui: &mut egui::Ui) {
    let Some(path) = app.tools.selected.clone() else {
        return;
    };
    let Some(command) = app
        .tools
        .catalog
        .as_ref()
        .and_then(|catalog| catalog.commands.iter().find(|command| command.path == path))
        .cloned()
    else {
        app.tools.selected = None;
        return;
    };
    let started = app
        .tools
        .detail_started
        .unwrap_or_else(std::time::Instant::now);
    let progress = (started.elapsed().as_secs_f32() / 0.22).clamp(0.0, 1.0);
    if progress < 1.0 {
        let eased = 1.0 - (1.0 - progress).powi(3);
        ui.add_space((1.0 - eased) * 14.0);
        ui.set_opacity(0.35 + 0.65 * eased);
        ui.ctx().request_repaint();
    }
    if secondary_button(ui, "< Tool database").clicked() {
        app.tools.selected = None;
        return;
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(&command.path).size(26.0).strong());
        status_pill(ui, &command.safety, safety_color(&command.safety));
    });
    ui.label(
        RichText::new(&command.tool_name)
            .size(10.5)
            .monospace()
            .color(theme::MUTED),
    );
    ui.add_space(6.0);
    ui.label(RichText::new(&command.summary).size(12.0));
    if let Some(description) = &command.description
        && description != &command.summary
    {
        muted(ui, description);
    }
    ui.add_space(10.0);
    card(ui, |ui| {
        section_title(ui, "Usage");
        ui.label(RichText::new(&command.usage).size(10.5).monospace());
    });
    ui.add_space(8.0);
    card(ui, |ui| {
        section_title(ui, "Examples");
        if command.examples.is_empty() {
            muted(
                ui,
                "No example is recorded yet; use `--help` for the current syntax.",
            );
        }
        for example in &command.examples {
            ui.horizontal(|ui| {
                ui.label(RichText::new(example).size(10.0).monospace());
                copy_button(ui, example, "Copy");
            });
            ui.add_space(3.0);
        }
    });
    ui.add_space(8.0);
    card(ui, |ui| {
        section_title(ui, "Arguments");
        if command.arguments.is_empty() {
            muted(ui, "This command takes no command-specific arguments.");
            return;
        }
        argument_table(ui, &command.arguments);
    });
    ui.add_space(8.0);
    card(ui, |ui| {
        section_title(ui, "MCP");
        muted(
            ui,
            "This command is exposed as an MCP tool. Curated high-traffic tools keep their friendly schemas; all other commands use the same CLI argument names.",
        );
        path_row(ui, &command.tool_name);
        ui.label(
            RichText::new(format!(
                "Safety: {}. Mutating commands require confirm=true in MCP.",
                command.safety
            ))
            .size(9.5)
            .color(theme::MUTED),
        );
    });
    if progress < 1.0 {
        ui.set_opacity(1.0);
    }
}

fn argument_table(ui: &mut egui::Ui, arguments: &[CatalogArg]) {
    let width = ui.available_width();
    let widths = proportional_widths(width, &[0.20, 0.10, 0.12, 0.08, 0.50], 6.0);
    table_header(
        ui,
        &[
            ("Argument", widths[0]),
            ("Kind", widths[1]),
            ("Type", widths[2]),
            ("Required", widths[3]),
            ("Description", widths[4]),
        ],
    );
    for argument in arguments {
        let name = argument
            .long
            .as_deref()
            .map(|long| format!("--{long}"))
            .or_else(|| argument.short.map(|short| format!("-{short}")))
            .unwrap_or_else(|| argument.id.clone());
        let response = selectable_row(ui, false, theme::ROW_HEIGHT, |ui| {
            row_cell(ui, widths[0], |ui| {
                ui.add(egui::Label::new(RichText::new(name).size(10.0).monospace()).truncate());
            });
            row_cell(ui, widths[1], |ui| {
                let kind = if argument.positional {
                    "positional".to_string()
                } else {
                    "option".to_string()
                };
                ui.label(RichText::new(kind).size(9.5));
            });
            row_cell(ui, widths[2], |ui| {
                ui.label(RichText::new(&argument.kind).size(9.5));
            });
            row_cell(ui, widths[3], |ui| {
                ui.label(RichText::new(if argument.required { "yes" } else { "no" }).size(9.5));
            });
            row_cell(ui, widths[4], |ui| {
                let mut text = argument.help.clone().unwrap_or_else(|| "-".to_string());
                if let Some(default) = &argument.default {
                    text.push_str(&format!(" (default: {default})"));
                }
                if !argument.possible_values.is_empty() {
                    text.push_str(&format!(" [{}]", argument.possible_values.join(", ")));
                }
                ui.add(egui::Label::new(RichText::new(text).size(9.5)).truncate());
            });
        });
        if !argument.possible_values.is_empty() {
            response.on_hover_text(argument.possible_values.join("\n"));
        }
    }
}

fn matches_query(command: &CatalogCommand, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut haystack = format!(
        "{} {} {} {}",
        command.path,
        command.tool_name,
        command.summary,
        command.description.clone().unwrap_or_default()
    );
    for argument in &command.arguments {
        haystack.push(' ');
        haystack.push_str(&argument.id);
        if let Some(long) = &argument.long {
            haystack.push(' ');
            haystack.push_str(long);
        }
        if let Some(help) = &argument.help {
            haystack.push(' ');
            haystack.push_str(help);
        }
    }
    haystack.to_ascii_lowercase().contains(query)
}

fn safety_color(safety: &str) -> egui::Color32 {
    match safety {
        "destructive" | "unsafe" => theme::RED,
        "device-mutation" => theme::YELLOW,
        "long-running" => theme::BLUE,
        _ => theme::GREEN,
    }
}
