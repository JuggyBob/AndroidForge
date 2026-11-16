use eframe::egui;
use egui::{RichText, Vec2};
use egui_extras::{Column, TableBuilder};

use crate::app::App;
use crate::backend::evidence;
use crate::components::{copy_button, empty_state, error_banner, key_value, muted, section_title};
use crate::state::{EvidenceTab, RunDetailState};
use crate::theme;
use crate::views::overview::format_duration;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let Some((run_id, status, path, tab, project, command, started_at, duration_ms)) =
        app.run_detail.as_ref().map(|state| {
            let report = &state.detail.report;
            (
                report.run_id.clone(),
                report.status.clone(),
                state.detail.path.path.clone(),
                state.evidence.tab,
                report.project.clone(),
                report.command.clone(),
                report.started_at.clone(),
                report.duration_ms,
            )
        })
    else {
        return;
    };
    let mut back = false;
    ui.horizontal(|ui| {
        if ui.button("← Back to runs").clicked() {
            back = true;
        }
        ui.label(RichText::new(&run_id).size(15.0).strong());
        ui.label(
            RichText::new(&status)
                .size(11.0)
                .color(theme::status_color(&status)),
        );
        if app.run_loading {
            ui.spinner();
        }
        if !path.is_empty() {
            ui.add_space(8.0);
            if ui.small_button("Open run folder").clicked() {
                let _ = open::that(&path);
            }
            copy_button(ui, &path, "Copy path");
        }
    });
    if back {
        app.clear_run_detail();
        return;
    }
    ui.label(
        RichText::new(format!(
            "{} · {} · {} · {}",
            project,
            command,
            started_at,
            format_duration(duration_ms)
        ))
        .size(10.0)
        .color(theme::MUTED),
    );
    ui.add_space(5.0);
    let mut new_tab = tab;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut new_tab, EvidenceTab::Summary, "Summary");
        ui.selectable_value(&mut new_tab, EvidenceTab::Artifacts, "Evidence");
        ui.selectable_value(&mut new_tab, EvidenceTab::Raw, "Raw files");
    });
    if new_tab != tab {
        if let Some(state) = app.run_detail.as_mut() {
            state.evidence.tab = new_tab;
        }
    }
    ui.separator();
    match new_tab {
        EvidenceTab::Summary => {
            if let Some(state) = app.run_detail.as_ref() {
                summary(ui, state);
            }
        }
        EvidenceTab::Artifacts => {
            let mut filter = app
                .run_detail
                .as_ref()
                .map(|state| state.evidence.filter.clone())
                .unwrap_or_default();
            let mut pending = None;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Filter").size(11.0).color(theme::MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut filter)
                        .desired_width(220.0)
                        .hint_text("path, kind or description"),
                );
            });
            if let Some(state) = app.run_detail.as_mut() {
                state.evidence.filter = filter.clone();
            }
            ui.add_space(3.0);
            if let Some(state) = app.run_detail.as_mut() {
                evidence_split(ui, state, &filter, &mut pending);
            }
            if let Some(key) = pending {
                let ctx = ui.ctx().clone();
                select_artifact(app, &key, &ctx);
            }
        }
        EvidenceTab::Raw => {
            if let Some(state) = app.run_detail.as_ref() {
                raw_files(ui, state);
            }
        }
    }
}

fn summary(ui: &mut egui::Ui, state: &RunDetailState) {
    let report = &state.detail.report;
    if let Some(failure) = &report.failure {
        error_banner(ui, &format!("{}: {}", failure.code, failure.message));
        for suggestion in &failure.suggestions {
            ui.label(RichText::new(format!("• {suggestion}")).size(10.0));
        }
        ui.add_space(4.0);
    }
    section_title(ui, "Build identity");
    egui::Grid::new("run-build")
        .num_columns(4)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            key_value(
                ui,
                "Package",
                report.build.package.as_deref().unwrap_or("-"),
            );
            key_value(
                ui,
                "Version",
                &format!(
                    "{} ({})",
                    report.build.version_name.as_deref().unwrap_or("-"),
                    report
                        .build
                        .version_code
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string())
                ),
            );
            ui.end_row();
            key_value(
                ui,
                "APK SHA-256",
                report.build.apk_sha256.as_deref().unwrap_or("-"),
            );
            key_value(
                ui,
                "Installed SHA-256",
                report.build.installed_apk_sha256.as_deref().unwrap_or("-"),
            );
            ui.end_row();
            key_value(
                ui,
                "Certificate",
                report.build.cert_sha256.as_deref().unwrap_or("-"),
            );
            key_value(
                ui,
                "Verified",
                if report.build.verified { "yes" } else { "no" },
            );
            ui.end_row();
            key_value(ui, "Session", report.session_id.as_deref().unwrap_or("-"));
            key_value(ui, "Device", report.device_serial.as_deref().unwrap_or("-"));
            ui.end_row();
        });
    ui.add_space(8.0);
    section_title(ui, "Steps");
    if report.steps.is_empty() {
        empty_state(ui, "No steps", "The report contains no recorded steps.");
        return;
    }
    let steps = &report.steps;
    TableBuilder::new(ui)
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(55.0))
        .column(Column::initial(65.0))
        .column(Column::remainder().at_least(160.0))
        .column(Column::initial(75.0))
        .column(Column::initial(55.0))
        .column(Column::initial(65.0))
        .header(22.0, |mut header| {
            for name in ["#", "Status", "Step", "Duration", "Exit", "Artifacts"] {
                header.col(|ui| {
                    ui.label(RichText::new(name).size(10.0).strong().color(theme::MUTED));
                });
            }
        })
        .body(|body| {
            body.rows(24.0, steps.len(), |mut row| {
                let step = &steps[row.index()];
                row.col(|ui| {
                    ui.label(RichText::new(&step.id).size(9.0).monospace());
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(&step.status)
                            .size(10.0)
                            .color(theme::status_color(&step.status)),
                    );
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(&step.name).size(10.0)).truncate())
                        .on_hover_text(&step.name);
                });
                row.col(|ui| {
                    ui.label(RichText::new(format_duration(step.duration_ms)).size(10.0));
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(
                            step.exit_code
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "-".to_string()),
                        )
                        .size(10.0),
                    );
                });
                row.col(|ui| {
                    ui.label(RichText::new(step.artifacts.len().to_string()).size(10.0));
                });
            });
        });
    if !report.artifacts.is_empty() {
        ui.add_space(6.0);
        muted(
            ui,
            format!("{} artifact(s) in this run", report.artifacts.len()),
        );
    }
}

fn evidence_split(
    ui: &mut egui::Ui,
    state: &mut RunDetailState,
    filter: &str,
    pending: &mut Option<String>,
) {
    let artifacts = state.detail.report.artifacts.clone();
    let run_dir = state.detail.path.path.clone();
    ui.columns(2, |columns| {
        columns[0].set_min_width(240.0);
        columns[0].set_max_width(360.0);
        egui::ScrollArea::vertical()
            .id_salt("evidence-list")
            .max_height(490.0)
            .show(&mut columns[0], |ui| {
                let filter = filter.to_ascii_lowercase();
                for artifact in &artifacts {
                    let haystack = format!(
                        "{} {} {}",
                        artifact.kind,
                        artifact.path,
                        artifact.description.as_deref().unwrap_or("")
                    )
                    .to_ascii_lowercase();
                    if !filter.is_empty() && !haystack.contains(&filter) {
                        continue;
                    }
                    let selected =
                        state.evidence.selected_artifact.as_deref() == Some(artifact.path.as_str());
                    let label = format!(
                        "{}  {}",
                        icon_for_kind(&artifact.kind),
                        artifact.path.rsplit('/').next().unwrap_or(&artifact.path)
                    );
                    if ui
                        .selectable_label(selected, RichText::new(label).size(10.0))
                        .on_hover_text(format!("{}\n{}", artifact.path, artifact.kind))
                        .clicked()
                    {
                        *pending = Some(artifact.path.clone());
                    }
                }
            });
        columns[1].set_min_width(300.0);
        egui::ScrollArea::vertical()
            .id_salt("evidence-preview")
            .max_height(490.0)
            .show(&mut columns[1], |ui| {
                if let Some(key) = state.evidence.selected_artifact.clone() {
                    preview(ui, &mut state.evidence, &artifacts, &run_dir, &key);
                } else {
                    empty_state(
                        ui,
                        "No artifact selected",
                        "Choose an artifact to inspect it.",
                    );
                }
            });
    });
}

fn preview(
    ui: &mut egui::Ui,
    evidence: &mut crate::state::EvidenceState,
    artifacts: &[crate::models::ArtifactRef],
    run_dir: &str,
    key: &str,
) {
    let artifact = artifacts.iter().find(|artifact| artifact.path == key);
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).size(10.0).monospace());
        if let Some(artifact) = artifact {
            ui.label(
                RichText::new(format!("{} · {} bytes", artifact.kind, artifact.size_bytes))
                    .size(10.0)
                    .color(theme::MUTED),
            );
        }
        copy_button(ui, key, "Copy path");
        let full = format!("{}/{}", run_dir.replace('\\', "/"), key);
        if ui.small_button("Open").clicked() {
            let _ = open::that(full);
        }
    });
    ui.separator();
    if let Some(texture) = evidence.image_texture.clone() {
        if evidence.image_path.as_deref() == Some(key) {
            let (w, h) = evidence.image_size.unwrap_or((1000, 1000));
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{w} × {h} original pixels"))
                        .size(10.0)
                        .color(theme::MUTED),
                );
                if ui.small_button("Fit").clicked() {
                    evidence.fit = true;
                }
                if ui.small_button("100%").clicked() {
                    evidence.fit = false;
                    evidence.zoom = 1.0;
                }
                if !evidence.fit {
                    ui.add(egui::Slider::new(&mut evidence.zoom, 0.1..=3.0).show_value(false));
                }
            });
            let available = ui.available_width().max(200.0);
            let scale = if evidence.fit {
                (available / w as f32).min(520.0 / h as f32).min(1.0)
            } else {
                evidence.zoom
            };
            let size = Vec2::new(w as f32 * scale, h as f32 * scale);
            egui::ScrollArea::both()
                .id_salt("image-scroll")
                .show(ui, |ui| {
                    ui.image((texture.id(), size));
                });
            return;
        }
    }
    if evidence.image_path.as_deref() == Some(key) {
        if let Some(error) = &evidence.image_error {
            error_banner(ui, error);
        }
        return;
    }
    if evidence.text_key.as_deref() == Some(key) {
        if let Some(error) = &evidence.text_error {
            error_banner(ui, error);
        } else if let Some(text) = evidence.text.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Search").size(10.0).color(theme::MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut evidence.search)
                        .desired_width(200.0)
                        .hint_text("filter log lines"),
                );
                ui.label(
                    RichText::new(format!(
                        "{} bytes{}",
                        text.size_bytes,
                        if text.truncated {
                            " · preview truncated"
                        } else {
                            ""
                        }
                    ))
                    .size(10.0)
                    .color(theme::MUTED),
                );
            });
            let query = evidence.search.to_ascii_lowercase();
            let mut content = String::new();
            for (line_number, line) in text.text.lines().enumerate() {
                if query.is_empty() || line.to_ascii_lowercase().contains(&query) {
                    content.push_str(&format!("{:>5}  {line}\n", line_number + 1));
                }
                if content.len() > 1_500_000 {
                    content.push_str("\n[... preview display capped ...]\n");
                    break;
                }
            }
            ui.add(
                egui::TextEdit::multiline(&mut content)
                    .font(egui::FontId::monospace(10.0))
                    .desired_rows(24)
                    .desired_width(f32::INFINITY)
                    .interactive(true),
            );
        } else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading preview...");
            });
        }
    } else {
        empty_state(
            ui,
            "Artifact selected",
            "Choose an artifact to load its preview.",
        );
    }
}

fn raw_files(ui: &mut egui::Ui, state: &RunDetailState) {
    let manifest = &state.detail.manifest;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "Manifest: {} ok · {} missing · {} mismatched",
                manifest.ok, manifest.missing, manifest.mismatched
            ))
            .size(11.0)
            .color(if manifest.missing + manifest.mismatched == 0 {
                theme::GREEN
            } else {
                theme::RED
            }),
        );
    });
    ui.add_space(4.0);
    ui.columns(2, |columns| {
        egui::ScrollArea::vertical()
            .id_salt("raw-report")
            .max_height(420.0)
            .show(&mut columns[0], |ui| {
                ui.label(RichText::new("report.json").strong());
                if let Some(preview) = &state.detail.raw_report {
                    let mut text = preview.text.clone();
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .font(egui::FontId::monospace(10.0))
                            .desired_rows(18)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                }
            });
        egui::ScrollArea::vertical()
            .id_salt("manifest-list")
            .max_height(420.0)
            .show(&mut columns[1], |ui| {
                ui.label(RichText::new("manifest.json").strong());
                for entry in &manifest.entries {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(entry.status)
                                .size(9.0)
                                .color(theme::status_color(entry.status)),
                        );
                        ui.add(
                            egui::Label::new(RichText::new(&entry.path).size(9.0).monospace())
                                .truncate(),
                        )
                        .on_hover_text(&entry.path);
                    });
                }
            });
    });
    ui.add_space(6.0);
    ui.collapsing("metadata.json", |ui| {
        if let Some(preview) = &state.detail.raw_metadata {
            let mut text = preview.text.clone();
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .font(egui::FontId::monospace(10.0))
                    .desired_rows(12)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
        }
    });
    ui.collapsing("manifest.json (raw)", |ui| {
        if let Some(preview) = &state.detail.raw_manifest {
            let mut text = preview.text.clone();
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .font(egui::FontId::monospace(10.0))
                    .desired_rows(12)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
        }
    });
}

fn select_artifact(app: &mut App, key: &str, ctx: &egui::Context) {
    let Some(state) = app.run_detail.as_ref() else {
        return;
    };
    let run_dir = state.detail.path.path.clone();
    if evidence::safe_join(std::path::Path::new(&run_dir), key).is_err() {
        if let Some(state) = app.run_detail.as_mut() {
            state.evidence.selected_artifact = Some(key.to_string());
            state.evidence.image_error = Some("Artifact path is outside the run directory".into());
        }
        return;
    }
    let extension = std::path::Path::new(key)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_image = matches!(extension.as_str(), "png" | "jpg" | "jpeg");
    if let Some(state) = app.run_detail.as_mut() {
        state.evidence.selected_artifact = Some(key.to_string());
        state.evidence.text = None;
        state.evidence.text_key = None;
        state.evidence.text_error = None;
        state.evidence.image_error = None;
        state.evidence.image_path = None;
        state.evidence.image_texture = None;
        state.evidence.image_size = None;
    }
    if is_image {
        let result = evidence::safe_join(std::path::Path::new(&run_dir), key)
            .and_then(|path| std::fs::read(path).map_err(crate::backend::DesktopError::Io));
        match result {
            Ok(bytes) => match image::load_from_memory(&bytes) {
                Ok(image) => {
                    let image = image.to_rgba8();
                    let size = [image.width() as usize, image.height() as usize];
                    let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                    let texture = ctx.load_texture(
                        format!("evidence-{key}"),
                        color,
                        egui::TextureOptions::LINEAR,
                    );
                    if let Some(state) = app.run_detail.as_mut() {
                        state.evidence.image_size = Some((image.width(), image.height()));
                        state.evidence.image_texture = Some(texture);
                        state.evidence.image_path = Some(key.to_string());
                    }
                }
                Err(error) => {
                    if let Some(state) = app.run_detail.as_mut() {
                        state.evidence.image_error = Some(error.to_string());
                    }
                }
            },
            Err(error) => {
                if let Some(state) = app.run_detail.as_mut() {
                    state.evidence.image_error = Some(error.to_string());
                }
            }
        }
    } else {
        app.request_evidence_text(key.to_string(), ctx);
    }
}

fn icon_for_kind(kind: &str) -> &'static str {
    if kind.contains("screenshot") {
        "▣"
    } else if kind.contains("log") {
        "≡"
    } else if kind.contains("hierarchy") {
        "⌘"
    } else if kind.contains("diagnostic") {
        "◇"
    } else {
        "·"
    }
}
