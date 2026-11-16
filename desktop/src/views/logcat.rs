use eframe::egui;
use egui::{RichText, Vec2};

use crate::app::App;
use crate::components::*;
use crate::theme;

const LEVELS: [&str; 6] = ["verbose", "debug", "info", "warn", "error", "fatal"];

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    page_header(
        ui,
        "Logcat",
        "A bounded live log window. Filtering and search happen locally; the device buffer is never cleared.",
    );
    let sessions = app.sessions.value.sessions.clone();
    card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Session").size(10.0).color(theme::MUTED));
            let current = app
                .logcat
                .session_id
                .clone()
                .unwrap_or_else(|| "Select a session".to_string());
            egui::ComboBox::from_id_salt("logcat-session")
                .width(240.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for session in &sessions {
                        if ui
                            .selectable_label(
                                app.logcat.session_id.as_deref() == Some(session.id.as_str()),
                                format!(
                                    "{} · {} · {}",
                                    session.serial.as_deref().unwrap_or("-"),
                                    session.id,
                                    session.state
                                ),
                            )
                            .clicked()
                        {
                            app.logcat_select_session(Some(session.id.clone()));
                            if app.logcat.running {
                                app.start_logcat(ui.ctx());
                            }
                        }
                    }
                });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Package").size(10.0).color(theme::MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut app.logcat.package)
                        .desired_width(150.0)
                        .hint_text("optional package"),
                );
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Level").size(10.0).color(theme::MUTED));
                egui::ComboBox::from_id_salt("logcat-level")
                    .width(90.0)
                    .selected_text(&app.logcat.level)
                    .show_ui(ui, |ui| {
                        for level in LEVELS {
                            ui.selectable_value(&mut app.logcat.level, level.to_string(), level);
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Search").size(10.0).color(theme::MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut app.logcat.search)
                        .desired_width(170.0)
                        .hint_text("message, tag or pid"),
                );
            });
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if app.logcat.running {
                if danger_button(ui, "Stop").clicked() {
                    app.stop_logcat();
                }
                if secondary_button(ui, if app.logcat.paused { "Resume" } else { "Pause" })
                    .clicked()
                {
                    app.toggle_logcat_pause(ui.ctx());
                }
                status_pill(
                    ui,
                    if app.logcat.error.is_some() {
                        "disconnected"
                    } else if app.logcat.paused {
                        "paused"
                    } else {
                        "streaming"
                    },
                    if app.logcat.error.is_some() {
                        theme::RED
                    } else if app.logcat.paused {
                        theme::YELLOW
                    } else {
                        theme::GREEN
                    },
                );
            } else if primary_button(ui, "Start streaming").clicked() {
                app.start_logcat(ui.ctx());
            }
            if secondary_button(ui, "Export").clicked() {
                app.export_logcat();
            }
            if ghost_button(ui, "Clear").clicked() {
                app.clear_logcat();
            }
            ui.checkbox(&mut app.logcat.follow, "Follow latest");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                small(
                    ui,
                    format!(
                        "{} / {} lines buffered",
                        app.logcat.lines.len(),
                        app.logcat.max_lines
                    ),
                );
            });
        });
        if let Some(error) = &app.logcat.error {
            error_banner(ui, error);
        }
        for note in &app.logcat.notes {
            small(ui, note);
        }
        if let Some(path) = &app.logcat.exported {
            path_row(ui, path);
        }
    });
    ui.add_space(8.0);
    let min_rank = level_rank(&app.logcat.level);
    let query = app.logcat.search.to_ascii_lowercase();
    let entries = app
        .logcat
        .lines
        .iter()
        .filter(|entry| level_rank(&entry.level) >= min_rank)
        .filter(|entry| {
            query.is_empty()
                || entry.message.to_ascii_lowercase().contains(&query)
                || entry.tag.to_ascii_lowercase().contains(&query)
                || entry.raw.to_ascii_lowercase().contains(&query)
        })
        .cloned()
        .collect::<Vec<_>>();
    if entries.is_empty() {
        empty_state(
            ui,
            "No log lines",
            if app.logcat.running {
                "Waiting for output that matches the current filters."
            } else {
                "Select a session and start streaming."
            },
        );
        return;
    }
    let available = ui.available_height().max(240.0);
    egui::ScrollArea::both()
        .id_salt("logcat-scroll")
        .stick_to_bottom(app.logcat.follow && !app.logcat.paused)
        .max_height(available)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for entry in &entries {
                let color = log_color(&entry.level, entry.crash);
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width().max(600.0), 18.0),
                    egui::Sense::hover(),
                );
                if entry.crash {
                    ui.painter().rect_filled(
                        rect,
                        2.0,
                        egui::Color32::from_rgba_unmultiplied(255, 107, 107, 18),
                    );
                }
                let painter = ui.painter();
                painter.text(
                    rect.left_center() + Vec2::new(4.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &entry.timestamp,
                    egui::FontId::monospace(9.0),
                    theme::MUTED,
                );
                painter.text(
                    rect.left_center() + Vec2::new(112.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &entry.level,
                    egui::FontId::monospace(9.5),
                    color,
                );
                let tag_clip = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 132.0, rect.top()),
                    egui::pos2(rect.left() + 308.0, rect.bottom()),
                );
                painter.with_clip_rect(tag_clip).text(
                    rect.left_center() + Vec2::new(132.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    truncate_end(&entry.tag, 30),
                    egui::FontId::monospace(9.5),
                    color,
                );
                let message_clip = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 320.0, rect.top()),
                    rect.right_bottom(),
                );
                painter.with_clip_rect(message_clip).text(
                    rect.left_center() + Vec2::new(320.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &entry.message,
                    egui::FontId::monospace(9.5),
                    if entry.crash {
                        theme::RED
                    } else {
                        ui.style().visuals.text_color()
                    },
                );
            }
        });
}

fn truncate_end(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut text = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    text.push('…');
    text
}

fn level_rank(level: &str) -> i32 {
    match level.to_ascii_lowercase().as_str() {
        "verbose" | "v" => 0,
        "debug" | "d" => 1,
        "info" | "i" => 2,
        "warn" | "w" | "warning" => 3,
        "error" | "e" => 4,
        "fatal" | "f" | "assert" => 5,
        _ => 0,
    }
}

fn log_color(level: &str, crash: bool) -> egui::Color32 {
    if crash {
        return theme::RED;
    }
    match level_rank(level) {
        0 => theme::MUTED,
        1 => theme::BLUE,
        2 => theme::GREEN,
        3 => theme::YELLOW,
        4 => theme::RED,
        _ => theme::PURPLE,
    }
}
