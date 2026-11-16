use eframe::egui;
use egui::RichText;

use crate::app::App;
use crate::state::*;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let Some(operation) = app.operation.as_ref() else {
        return;
    };
    let title = operation.handle.title.clone();
    let command = operation.handle.command.clone();
    let elapsed = operation.handle.elapsed();
    let running = operation.is_running();
    let outcome = operation.outcome.clone();
    let lines = operation.lines.clone();
    let mut cancel_clicked = false;
    let mut dismiss_clicked = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong());
        ui.label(
            RichText::new(format!("{:.1}s", elapsed.as_secs_f32()))
                .size(10.0)
                .color(theme::MUTED),
        );
        if running {
            ui.spinner();
            if operation.kind.cancellable() {
                if ui.button("Cancel...").clicked() {
                    cancel_clicked = true;
                }
            } else {
                ui.label(
                    RichText::new("session operation · runs to completion")
                        .size(10.0)
                        .color(theme::MUTED),
                )
                .on_hover_text(
                    "Session lifecycle operations cannot be cancelled safely. Wait for the CLI to finish; cancellation of a run never stops the emulator.",
                );
            }
        } else {
            if let Some(outcome) = &outcome {
                let (label, color) = if outcome.cancelled {
                    ("cancelled", theme::YELLOW)
                } else if outcome.success {
                    ("success", theme::GREEN)
                } else {
                    ("failed", theme::RED)
                };
                ui.label(RichText::new(label).size(11.0).color(color));
                ui.label(
                    RichText::new(format!("exit {:?}", outcome.status_code))
                        .size(10.0)
                        .color(theme::MUTED),
                );
                if outcome.truncated {
                    ui.label(
                        RichText::new("output truncated")
                            .size(10.0)
                            .color(theme::YELLOW),
                    );
                }
            }
            if ui.button("Dismiss").clicked() {
                dismiss_clicked = true;
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
    ui.separator();
    let body_height = (ui.available_height() - 8.0).max(60.0);
    egui::ScrollArea::vertical()
        .id_salt("operation-output")
        .stick_to_bottom(true)
        .max_height(body_height)
        .show(ui, |ui| {
            if let Some(outcome) = &outcome {
                for line in outcome.stdout.lines().chain(outcome.stderr.lines()) {
                    ui.label(
                        RichText::new(line)
                            .font(egui::FontId::monospace(10.0))
                            .size(10.0),
                    );
                }
                if let Some(error) = &outcome.parse_error {
                    if outcome.cancelled {
                        ui.colored_label(
                            theme::YELLOW,
                            "No final report was produced because the operation was cancelled.",
                        );
                    } else {
                        ui.colored_label(theme::RED, error);
                    }
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
                            .font(egui::FontId::monospace(10.0))
                            .size(10.0)
                            .color(color),
                    );
                }
            }
        });
    if cancel_clicked {
        app.confirm(ConfirmAction::CancelOperation);
    }
    if dismiss_clicked {
        app.dismiss_operation();
    }
}
