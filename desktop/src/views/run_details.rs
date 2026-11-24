use eframe::egui;
use egui::{RichText, Vec2};

use crate::app::App;
use crate::backend::evidence;
use crate::components::*;
use crate::state::*;
use crate::theme;
use crate::views::format_duration;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let Some(state) = app.run_detail.as_ref() else {
        empty_state(ui, "No run selected", "Choose a run from the Runs page.");
        return;
    };
    let report = state.detail.report.clone();
    let path = state.detail.path.path.clone();
    let tab = state.evidence.tab;
    let mut new_tab = tab;
    ui.horizontal(|ui| {
        if ghost_button(ui, "← Back to runs").clicked() {
            app.clear_run_detail();
        }
        ui.label(RichText::new(&report.run_id).size(17.0).strong());
        status_pill(ui, &report.status, theme::status_color(&report.status));
        if app.run_loading {
            ui.spinner();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if secondary_button(ui, "Open folder").clicked() && !path.is_empty() {
                let _ = open::that(&path);
            }
            if ghost_button(ui, "Copy path").clicked() && !path.is_empty() {
                ui.ctx().copy_text(path.clone());
            }
            if ghost_button(ui, "Compare").clicked() {
                app.compare_form.current = Some(report.run_id.clone());
                app.set_page(Page::Compare);
            }
            if !matches!(report.status.as_str(), "passed")
                && secondary_button(ui, "Triage").clicked()
            {
                app.open_triage(Some(report.run_id.clone()), ui.ctx());
            }
        });
    });
    ui.label(
        RichText::new(format!(
            "{} · {} · {} · {}",
            report.project,
            report.command,
            report.started_at,
            format_duration(report.duration_ms)
        ))
        .size(10.0)
        .color(theme::MUTED),
    );
    ui.add_space(6.0);
    segmented(
        ui,
        &[
            ("Summary", tab == EvidenceTab::Summary),
            ("Evidence", tab == EvidenceTab::Artifacts),
            ("Raw files", tab == EvidenceTab::Raw),
        ],
        |index| {
            new_tab = match index {
                0 => EvidenceTab::Summary,
                1 => EvidenceTab::Artifacts,
                _ => EvidenceTab::Raw,
            }
        },
    );
    if new_tab != tab {
        if let Some(state) = app.run_detail.as_mut() {
            state.evidence.tab = new_tab;
        }
    }
    ui.add_space(8.0);
    match new_tab {
        EvidenceTab::Summary => {
            summary(ui, &report);
        }
        EvidenceTab::Artifacts => {
            let mut pending = None;
            if let Some(state) = app.run_detail.as_mut() {
                pending = evidence_tab(ui, state);
            }
            if let Some(key) = pending {
                select_artifact(app, &key, ui.ctx());
            }
        }
        EvidenceTab::Raw => {
            if let Some(state) = app.run_detail.as_ref() {
                raw_files(ui, state);
            }
        }
    }
}

fn summary(ui: &mut egui::Ui, report: &crate::models::RunReport) {
    if let Some(failure) = &report.failure {
        error_banner(ui, &format!("{}: {}", failure.code, failure.message));
        for suggestion in &failure.suggestions {
            ui.label(
                RichText::new(format!("• {suggestion}"))
                    .size(10.0)
                    .color(theme::MUTED),
            );
        }
        ui.add_space(4.0);
    }
    let build = &report.build;
    section_label(ui, "Build identity");
    card(ui, |ui| {
        ui.columns(2, |columns| {
            kv_grid(
                &mut columns[0],
                "run-build-left",
                &[
                    (
                        "Package",
                        build.package.clone().unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Version",
                        format!(
                            "{} ({})",
                            build
                                .version_name
                                .clone()
                                .unwrap_or_else(|| "-".to_string()),
                            build
                                .version_code
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "-".to_string())
                        ),
                    ),
                    (
                        "APK SHA-256",
                        build.apk_sha256.clone().unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Installed SHA-256",
                        build
                            .installed_apk_sha256
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            );
            kv_grid(
                &mut columns[1],
                "run-build-right",
                &[
                    (
                        "Certificate",
                        build.cert_sha256.clone().unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Verified",
                        if build.verified { "yes" } else { "no" }.to_string(),
                    ),
                    (
                        "Session",
                        report.session_id.clone().unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Device",
                        report
                            .device_serial
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            );
        });
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        section_title(ui, "Steps");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ghost_button(ui, "Copy Markdown").clicked() {
                ui.ctx().copy_text(markdown_summary(report));
            }
            if ghost_button(ui, "Copy JSON").clicked() {
                if let Ok(text) = serde_json::to_string_pretty(report) {
                    ui.ctx().copy_text(text);
                }
            }
        });
    });
    if report.steps.is_empty() {
        empty_state(ui, "No steps", "The report contains no recorded steps.");
        return;
    }
    let steps = report.steps.clone();
    let width = ui.available_width();
    let widths = proportional_widths(width, &[0.05, 0.09, 0.53, 0.13, 0.07, 0.13], 10.0);
    let id_width = widths[0];
    let status_width = widths[1];
    let name_width = widths[2];
    let duration_width = widths[3];
    let exit_width = widths[4];
    let artifacts_width = widths[5];
    table_header(
        ui,
        &[
            ("#", id_width),
            ("Status", status_width),
            ("Step", name_width),
            ("Duration", duration_width),
            ("Exit", exit_width),
            ("Artifacts", artifacts_width),
        ],
    );
    for step in &steps {
        selectable_row(ui, false, theme::ROW_HEIGHT, |ui| {
            row_cell(ui, id_width, |ui| {
                ui.label(RichText::new(&step.id).size(9.0).monospace());
            });
            row_cell(ui, status_width, |ui| {
                status_pill(ui, &step.status, theme::status_color(&step.status));
            });
            row_cell(ui, name_width, |ui| {
                ui.add(egui::Label::new(RichText::new(&step.name).size(10.5)).truncate())
                    .on_hover_text(&step.name);
            });
            row_cell(ui, duration_width, |ui| {
                ui.label(RichText::new(format_duration(step.duration_ms)).size(10.0));
            });
            row_cell(ui, exit_width, |ui| {
                ui.label(
                    RichText::new(
                        step.exit_code
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    )
                    .size(10.0),
                );
            });
            row_cell(ui, artifacts_width, |ui| {
                ui.label(RichText::new(step.artifacts.len().to_string()).size(10.0));
            });
        });
        ui.add_space(2.0);
    }
    if !report.artifacts.is_empty() {
        muted(
            ui,
            format!("{} artifact(s) in this run", report.artifacts.len()),
        );
    }
}

fn evidence_tab(ui: &mut egui::Ui, state: &mut RunDetailState) -> Option<String> {
    let artifacts = state.detail.report.artifacts.clone();
    let run_dir = state.detail.path.path.clone();
    let mut filter = state.evidence.filter.clone();
    let selected = state.evidence.selected_artifact.clone();
    let mut pending = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Filter").size(10.0).color(theme::MUTED));
        ui.add(
            egui::TextEdit::singleline(&mut filter)
                .desired_width(240.0)
                .hint_text("path, kind or description"),
        );
        if !artifacts.is_empty() {
            small(ui, format!("{} artifact(s)", artifacts.len()));
        }
    });
    if filter != state.evidence.filter {
        state.evidence.filter = filter.clone();
    }
    ui.add_space(4.0);
    ui.columns(2, |columns| {
        columns[0].set_min_width(240.0);
        columns[0].set_max_width(380.0);
        egui::ScrollArea::vertical()
            .id_salt("evidence-list")
            .max_height(500.0)
            .show(&mut columns[0], |ui| {
                let query = filter.to_ascii_lowercase();
                for artifact in &artifacts {
                    let haystack = format!(
                        "{} {} {}",
                        artifact.kind,
                        artifact.path,
                        artifact.description.as_deref().unwrap_or("")
                    )
                    .to_ascii_lowercase();
                    if !query.is_empty() && !haystack.contains(&query) {
                        continue;
                    }
                    let is_selected = selected.as_deref() == Some(artifact.path.as_str());
                    let label = format!(
                        "{}  {}",
                        icon_for_kind(&artifact.kind),
                        artifact.path.rsplit('/').next().unwrap_or(&artifact.path)
                    );
                    if ui
                        .selectable_label(is_selected, RichText::new(label).size(10.0))
                        .on_hover_text(format!("{}\n{}", artifact.path, artifact.kind))
                        .clicked()
                    {
                        pending = Some(artifact.path.clone());
                    }
                }
            });
        columns[1].set_min_width(320.0);
        egui::ScrollArea::vertical()
            .id_salt("evidence-preview")
            .max_height(500.0)
            .show(&mut columns[1], |ui| {
                if let Some(key) = selected.clone() {
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
    if let Some(key) = &pending {
        state.evidence.selected_artifact = Some(key.clone());
    }
    pending
}

fn preview(
    ui: &mut egui::Ui,
    evidence_state: &mut EvidenceState,
    artifacts: &[crate::models::ArtifactRef],
    run_dir: &str,
    key: &str,
) {
    let artifact = artifacts.iter().find(|artifact| artifact.path == key);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new(key).size(9.5).monospace()).truncate())
            .on_hover_text(key);
        if let Some(artifact) = artifact {
            small(
                ui,
                format!("{} · {} bytes", artifact.kind, artifact.size_bytes),
            );
        }
        copy_button(ui, key, "Copy");
        let full = format!("{}/{}", run_dir.replace('\\', "/"), key);
        if ui.small_button("Open").clicked() {
            let _ = open::that(full);
        }
    });
    ui.separator();
    if evidence_state.image_path.as_deref() == Some(key) {
        if let Some(texture) = evidence_state.image_texture.clone() {
            let (w, h) = evidence_state.image_size.unwrap_or((1000, 1000));
            ui.horizontal(|ui| {
                small(ui, format!("{w} × {h}"));
                if ui.small_button("Fit").clicked() {
                    evidence_state.fit = true;
                }
                if ui.small_button("100%").clicked() {
                    evidence_state.fit = false;
                    evidence_state.zoom = 1.0;
                }
                if !evidence_state.fit {
                    ui.add(
                        egui::Slider::new(&mut evidence_state.zoom, 0.1..=3.0).show_value(false),
                    );
                }
            });
            let available = ui.available_width().max(200.0);
            let scale = if evidence_state.fit {
                (available / w as f32).min(520.0 / h as f32).min(1.0)
            } else {
                evidence_state.zoom
            };
            let size = Vec2::new(w as f32 * scale, h as f32 * scale);
            egui::ScrollArea::both()
                .id_salt("image-scroll")
                .show(ui, |ui| {
                    ui.image((texture.id(), size));
                });
        } else if let Some(error) = &evidence_state.image_error {
            error_banner(ui, error);
        }
        return;
    }
    if evidence_state.text_key.as_deref() == Some(key) {
        if let Some(error) = &evidence_state.text_error {
            error_banner(ui, error);
        } else if let Some(text) = evidence_state.text.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Search").size(10.0).color(theme::MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut evidence_state.search)
                        .desired_width(200.0)
                        .hint_text("filter log lines"),
                );
                small(
                    ui,
                    format!(
                        "{} bytes{}",
                        text.size_bytes,
                        if text.truncated {
                            " · preview truncated"
                        } else {
                            ""
                        }
                    ),
                );
            });
            let query = evidence_state.search.to_ascii_lowercase();
            let mut content = String::new();
            for (index, line) in text.text.lines().enumerate() {
                if query.is_empty() || line.to_ascii_lowercase().contains(&query) {
                    content.push_str(&format!("{:>5}  {line}\n", index + 1));
                }
                if content.len() > 1_500_000 {
                    content.push_str("\n[... preview display capped ...]\n");
                    break;
                }
            }
            ui.add(
                egui::TextEdit::multiline(&mut content)
                    .font(egui::FontId::monospace(9.5))
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
            "Choose it again to load its preview.",
        );
    }
}

fn raw_files(ui: &mut egui::Ui, state: &RunDetailState) {
    let manifest = &state.detail.manifest;
    ui.horizontal(|ui| {
        status_pill(
            ui,
            if manifest.missing + manifest.mismatched == 0 {
                "manifest verified"
            } else {
                "manifest mismatch"
            },
            if manifest.missing + manifest.mismatched == 0 {
                theme::GREEN
            } else {
                theme::RED
            },
        );
        small(
            ui,
            format!(
                "{} ok · {} missing · {} mismatched",
                manifest.ok, manifest.missing, manifest.mismatched
            ),
        );
        if !state.detail.path.path.is_empty() && ui.small_button("Open folder").clicked() {
            let _ = open::that(&state.detail.path.path);
        }
    });
    ui.add_space(6.0);
    ui.columns(2, |columns| {
        egui::ScrollArea::vertical()
            .id_salt("raw-report")
            .max_height(430.0)
            .show(&mut columns[0], |ui| {
                section_title(ui, "report.json");
                if let Some(preview) = &state.detail.raw_report {
                    code_block(ui, &preview.text);
                }
            });
        egui::ScrollArea::vertical()
            .id_salt("manifest-list")
            .max_height(430.0)
            .show(&mut columns[1], |ui| {
                section_title(ui, "manifest.json");
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
    ui.add_space(8.0);
    ui.collapsing("metadata.json", |ui| {
        if let Some(preview) = &state.detail.raw_metadata {
            code_block(ui, &preview.text);
        }
    });
    ui.collapsing("manifest.json (raw)", |ui| {
        if let Some(preview) = &state.detail.raw_manifest {
            code_block(ui, &preview.text);
        }
    });
}

fn select_artifact(app: &mut App, key: &str, ctx: &egui::Context) {
    let Some(state) = app.run_detail.as_mut() else {
        return;
    };
    let run_dir = state.detail.path.path.clone();
    if evidence::safe_join(std::path::Path::new(&run_dir), key).is_err() {
        state.evidence.image_error = Some("Artifact path is outside the run directory".to_string());
        return;
    }
    let extension = std::path::Path::new(key)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_image = matches!(extension.as_str(), "png" | "jpg" | "jpeg");
    state.evidence.selected_artifact = Some(key.to_string());
    state.evidence.text = None;
    state.evidence.text_key = None;
    state.evidence.text_error = None;
    state.evidence.image_error = None;
    state.evidence.image_path = None;
    state.evidence.image_texture = None;
    state.evidence.image_size = None;
    if is_image {
        let result = evidence::safe_join(std::path::Path::new(&run_dir), key).and_then(|path| {
            let metadata = std::fs::metadata(&path).map_err(crate::backend::DesktopError::Io)?;
            if metadata.len() > 32 * 1024 * 1024 {
                return Err(crate::backend::DesktopError::EvidenceTooLarge);
            }
            std::fs::read(path).map_err(crate::backend::DesktopError::Io)
        });
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
                    state.evidence.image_size = Some((image.width(), image.height()));
                    state.evidence.image_texture = Some(texture);
                    state.evidence.image_path = Some(key.to_string());
                }
                Err(error) => state.evidence.image_error = Some(error.to_string()),
            },
            Err(error) => state.evidence.image_error = Some(error.to_string()),
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

fn markdown_summary(report: &crate::models::RunReport) -> String {
    let mut out = format!(
        "# AndroidForge run `{}`\n\n- Status: **{}**\n- Scenario: `{}`\n- Project: `{}`\n- Started: `{}`\n- Duration: {}\n",
        report.run_id,
        report.status,
        report.command,
        report.project,
        report.started_at,
        format_duration(report.duration_ms)
    );
    if let Some(failure) = &report.failure {
        out.push_str(&format!(
            "\n## Failure\n\n`{}`: {}\n",
            failure.code, failure.message
        ));
        for suggestion in &failure.suggestions {
            out.push_str(&format!("- {suggestion}\n"));
        }
    }
    out.push_str("\n## Steps\n\n| # | Status | Step | Duration |\n| --- | --- | --- | --- |\n");
    for step in &report.steps {
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            step.id,
            step.status,
            step.name,
            format_duration(step.duration_ms)
        ));
    }
    out
}
