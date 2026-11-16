use eframe::egui;
use egui::RichText;

use crate::app::App;
use crate::components::{ghost_button, small, status_pill};
use crate::state::*;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let Some(operation) = app.operation.as_ref() else {
        return;
    };
    let title = operation.handle.title.clone();
    let command = operation.handle.command.clone();
    let outcome = operation.outcome.clone();
    let elapsed = outcome
        .as_ref()
        .map(|outcome| outcome.duration_ms as f32 / 1000.0)
        .unwrap_or_else(|| operation.handle.elapsed().as_secs_f32());
    let running = operation.is_running();
    let lines = operation.lines.clone();
    let cancellable = operation.kind.cancellable();
    let mut cancel = false;
    let mut dismiss = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(12.0).strong());
        ui.label(
            RichText::new(format!("{elapsed:.1}s"))
                .size(9.5)
                .color(theme::MUTED),
        );
        if running {
            ui.spinner();
            if cancellable {
                if ghost_button(ui, "Cancel...").clicked() {
                    cancel = true;
                }
            } else {
                small(ui, "runs to completion");
            }
        } else if let Some(outcome) = &outcome {
            let (label, color) = if outcome.cancelled {
                ("cancelled", theme::PURPLE)
            } else if outcome.success {
                ("success", theme::GREEN)
            } else {
                ("failed", theme::RED)
            };
            status_pill(ui, label, color);
            if outcome.cancelled {
                small(ui, "process tree closed");
            } else {
                ui.label(
                    RichText::new(format!("exit {:?}", outcome.status_code))
                        .size(9.5)
                        .color(theme::MUTED),
                );
            }
            if outcome.truncated {
                small(ui, "output truncated");
            }
            if ghost_button(ui, "Dismiss").clicked() {
                dismiss = true;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Copy output").clicked() {
                let text = outcome
                    .as_ref()
                    .map(|outcome| {
                        format!("STDOUT\n{}\n\nSTDERR\n{}", outcome.stdout, outcome.stderr)
                    })
                    .unwrap_or_else(|| {
                        lines
                            .iter()
                            .map(|line| line.text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                ui.ctx().copy_text(text);
            }
        });
    });
    ui.add(
        egui::Label::new(
            RichText::new(&command)
                .size(9.0)
                .monospace()
                .color(theme::MUTED),
        )
        .truncate(),
    )
    .on_hover_text(&command);
    ui.add_space(4.0);
    let body_height = (ui.available_height() - 8.0).max(60.0);
    egui::ScrollArea::vertical()
        .id_salt("operation-output")
        .stick_to_bottom(true)
        .max_height(body_height)
        .show(ui, |ui| {
            if let Some(outcome) = &outcome {
                if outcome.cancelled {
                    ui.label(
                        RichText::new(
                            "No final report was produced because the operation was cancelled. The emulator and session were left intact.",
                        )
                        .size(10.0)
                        .color(theme::YELLOW),
                    );
                    ui.add_space(4.0);
                }
                for line in outcome.stdout.lines().chain(outcome.stderr.lines()) {
                    ui.label(
                        RichText::new(line)
                            .font(egui::FontId::monospace(9.5))
                            .size(9.5),
                    );
                }
                if let Some(error) = &outcome.parse_error
                    && !outcome.cancelled
                {
                    ui.colored_label(theme::RED, error);
                }
            } else {
                for line in &lines {
                    let color = if line.stream == LogStream::Stderr {
                        theme::YELLOW
                    } else {
                        ui.style().visuals.text_color()
                    };
                    ui.label(
                        RichText::new(&line.text)
                            .font(egui::FontId::monospace(9.5))
                            .size(9.5)
                            .color(color),
                    );
                }
            }
        });
    if cancel {
        app.confirm(ConfirmAction::CancelOperation);
    }
    if dismiss {
        app.dismiss_operation();
    }
}
