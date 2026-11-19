use eframe::egui;
use egui::RichText;

use crate::app::App;
use crate::components::*;
use crate::state::*;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    if app.run_detail.is_none() && app.triage.run_id.is_none() && !app.runs.value.runs.is_empty() {
        app.open_triage(None, ui.ctx());
    }
    page_header(
        ui,
        "Triage",
        "The failure, the evidence and the signals that help decide whether it is reproducible.",
    );
    if app.run_detail.is_none() {
        empty_state(
            ui,
            "No run selected for triage",
            "Open a failed run from Runs or select the latest failure.",
        );
        if primary_button(ui, "Triage latest failure").clicked() {
            app.open_triage(None, ui.ctx());
        }
        return;
    }
    let report = app
        .run_detail
        .as_ref()
        .map(|state| state.detail.report.clone())
        .unwrap();
    let path = app
        .run_detail
        .as_ref()
        .map(|state| state.detail.path.path.clone())
        .unwrap();
    ui.horizontal(|ui| {
        ui.label(RichText::new(&report.run_id).size(17.0).strong());
        status_pill(ui, &report.status, theme::status_color(&report.status));
        if ghost_button(ui, "Back to runs").clicked() {
            app.set_page(Page::Runs);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if secondary_button(ui, "Open full run").clicked() {
                app.set_page(Page::Runs);
            }
            if ghost_button(ui, "Open folder").clicked() && !path.is_empty() {
                let _ = open::that(&path);
            }
            if ghost_button(ui, "Copy JSON").clicked()
                && let Ok(text) = serde_json::to_string_pretty(&report)
            {
                ui.ctx().copy_text(text);
            }
        });
    });
    ui.label(
        RichText::new(format!(
            "{} · {} · {} · {}",
            report.project,
            report.command,
            report.started_at,
            crate::views::format_duration(report.duration_ms)
        ))
        .size(10.0)
        .color(theme::MUTED),
    );
    ui.add_space(theme::PAGE_GAP);
    ui.columns(2, |columns| {
        failure_panel(app, &mut columns[0], &report, &path);
        signals_panel(app, &mut columns[1], &report);
    });
    ui.add_space(theme::PAGE_GAP);
    diagnosis_panel(app, ui);
    ui.add_space(theme::PAGE_GAP);
    receipt_panel(ui, &report);
    ui.add_space(theme::PAGE_GAP);
    recurrence_panel(app, ui);
    ui.add_space(theme::PAGE_GAP);
    bundle_panel(app, ui);
    ui.add_space(theme::PAGE_GAP);
    artifacts_panel(app, ui, &report, &path);
    ui.add_space(theme::PAGE_GAP);
    if let Some(flaky) = app.triage.flaky.clone() {
        flaky_panel(ui, &flaky);
    } else if app.triage.flaky_loading {
        ui.horizontal(|ui| {
            ui.spinner();
            muted(ui, "Analyzing recent runs for flaky steps...");
        });
    } else if let Some(error) = &app.triage.flaky_error {
        error_banner(ui, error);
    }
}

fn failure_panel(app: &mut App, ui: &mut egui::Ui, report: &crate::models::RunReport, path: &str) {
    card_fixed_height(ui, 320.0, "triage-failure-panel", |ui| {
        section_title(ui, "Failure");
        if let Some(failure) = &report.failure {
            error_banner(ui, &format!("{}: {}", failure.code, failure.message));
            ui.add_space(4.0);
            kv_grid(
                ui,
                "triage-failure",
                &[
                    (
                        "Operation",
                        failure
                            .failed_operation
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Recoverable",
                        if failure.recoverable { "yes" } else { "no" }.to_string(),
                    ),
                    (
                        "Device",
                        report
                            .device_serial
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Package",
                        report
                            .build
                            .package
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            );
            for suggestion in &failure.suggestions {
                ui.label(
                    RichText::new(format!("→ {suggestion}"))
                        .size(10.0)
                        .color(theme::BLUE),
                );
            }
        } else {
            warning_banner(ui, "This run did not record a failure object.");
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if primary_button(ui, "Compare with baseline").clicked() {
                app.compare_form.current = Some(report.run_id.clone());
                app.set_page(Page::Compare);
            }
            if secondary_button(ui, "Open folder").clicked() && !path.is_empty() {
                let _ = open::that(path);
            }
        });
    });
}

fn diagnosis_panel(app: &mut App, ui: &mut egui::Ui) {
    let explanation = app.triage.explanation.clone();
    let loading = app.triage.explanation_loading;
    let error = app.triage.explanation_error.clone();
    let run_id = app.triage.run_id.clone();
    card(ui, |ui| {
        ui.horizontal(|ui| {
            section_title(ui, "Agent explanation");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if secondary_button(ui, "Refresh explanation").clicked()
                    && let Some(run_id) = &run_id
                {
                    app.refresh_explanation(run_id, ui.ctx());
                }
                if let Some(explanation) = &explanation {
                    copy_button(ui, &explanation.fingerprint, "Copy fingerprint");
                }
            });
        });
        if loading {
            ui.horizontal(|ui| {
                ui.spinner();
                muted(
                    ui,
                    "Deriving a deterministic explanation from the stored report...",
                );
            });
            return;
        }
        if let Some(error) = &error {
            error_banner(ui, error);
        }
        let Some(explanation) = explanation else {
            muted(
                ui,
                "No explanation loaded yet. Refresh to classify this failure without an LLM.",
            );
            return;
        };
        let diagnosis = &explanation.diagnosis;
        let color = match diagnosis.confidence.as_str() {
            "high" => theme::GREEN,
            "medium" => theme::YELLOW,
            "low" => theme::RED,
            _ => theme::MUTED,
        };
        ui.horizontal_wrapped(|ui| {
            status_pill(ui, &diagnosis.category, theme::BLUE);
            status_pill(ui, &diagnosis.confidence, color);
            if let Some(code) = &diagnosis.code {
                ui.label(
                    RichText::new(code)
                        .size(10.0)
                        .monospace()
                        .color(theme::MUTED),
                );
            }
        });
        ui.add_space(3.0);
        ui.label(RichText::new(&diagnosis.summary).size(11.0));
        kv_grid(
            ui,
            "triage-explanation",
            &[
                (
                    "Failed operation",
                    diagnosis
                        .failed_operation
                        .clone()
                        .unwrap_or_else(|| "-".to_string()),
                ),
                (
                    "Project root",
                    if explanation.project_root.is_empty() {
                        "-".to_string()
                    } else {
                        explanation.project_root.clone()
                    },
                ),
                ("Fingerprint", explanation.fingerprint.clone()),
            ],
        );
        if !explanation.uncertainty.is_empty() {
            ui.add_space(3.0);
            for line in &explanation.uncertainty {
                ui.label(
                    RichText::new(format!("Uncertainty: {line}"))
                        .size(9.5)
                        .color(theme::YELLOW),
                );
            }
        }
        if !explanation.alternatives.is_empty() {
            ui.add_space(3.0);
            ui.label(
                RichText::new("Alternative explanations")
                    .size(10.0)
                    .strong(),
            );
            for alternative in &explanation.alternatives {
                ui.label(
                    RichText::new(format!(
                        "{} ({}) - {}",
                        alternative.category, alternative.confidence, alternative.summary
                    ))
                    .size(9.5)
                    .color(theme::MUTED),
                );
            }
        }
        if !explanation.evidence.is_empty() {
            ui.add_space(5.0);
            ui.label(RichText::new("Relevant evidence").size(10.0).strong());
            for evidence in explanation.evidence.iter().take(6) {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&evidence.source)
                            .size(9.5)
                            .monospace()
                            .color(theme::BLUE),
                    );
                    ui.label(
                        RichText::new(&evidence.description)
                            .size(9.5)
                            .color(theme::MUTED),
                    );
                    copy_button(ui, &evidence.source, "Copy path");
                });
            }
        }
        if !explanation.next_actions.is_empty() {
            ui.add_space(5.0);
            ui.label(
                RichText::new("Next actions (never auto-executed)")
                    .size(10.0)
                    .strong(),
            );
            for action in explanation.next_actions.iter().take(6) {
                ui.add_space(2.0);
                ui.label(RichText::new(&action.reason).size(9.5).color(theme::MUTED));
                ui.horizontal(|ui| {
                    let command = action.command.display();
                    ui.label(RichText::new(&command).size(9.5).monospace());
                    copy_button(ui, &command, "Copy command");
                    if action.destructive {
                        status_pill(ui, "destructive", theme::RED);
                    }
                });
                if let Some(warning) = &action.warning {
                    ui.label(RichText::new(warning).size(9.0).color(theme::RED));
                }
            }
        }
    });
}

fn receipt_panel(ui: &mut egui::Ui, report: &crate::models::RunReport) {
    let receipt = report.steps.iter().find_map(|step| {
        step.details
            .get("receipt")
            .or_else(|| {
                step.failure
                    .as_ref()
                    .and_then(|failure| failure.context.get("receipt"))
            })
            .cloned()
    });
    let Some(receipt) = receipt else {
        return;
    };
    card(ui, |ui| {
        section_title(ui, "Action receipt");
        let effect = receipt["effect"].as_str().unwrap_or("unknown");
        let completeness = receipt["completeness"].as_str().unwrap_or("incomplete");
        ui.horizontal_wrapped(|ui| {
            status_pill(ui, effect, theme::status_color(effect));
            status_pill(
                ui,
                completeness,
                if completeness == "complete" {
                    theme::GREEN
                } else {
                    theme::YELLOW
                },
            );
            if let Some(state) = receipt["dispatch_state"].as_str() {
                status_pill(ui, state, theme::MUTED);
            }
        });
        let empty = "not available".to_string();
        kv_grid(
            ui,
            "triage-receipt",
            &[
                (
                    "Action",
                    receipt["action"].as_str().unwrap_or("-").to_string(),
                ),
                (
                    "Before token",
                    receipt["before"]["token"]
                        .as_str()
                        .unwrap_or(&empty)
                        .to_string(),
                ),
                (
                    "After token",
                    receipt["after"]["token"]
                        .as_str()
                        .unwrap_or(&empty)
                        .to_string(),
                ),
                (
                    "Semantic changes",
                    format!(
                        "{} added · {} removed · {} modified",
                        receipt["changes"]["added_total"].as_u64().unwrap_or(0),
                        receipt["changes"]["removed_total"].as_u64().unwrap_or(0),
                        receipt["changes"]["modified_total"].as_u64().unwrap_or(0)
                    ),
                ),
                (
                    "Postcondition",
                    receipt["postcondition"]["result"]
                        .as_str()
                        .unwrap_or("not requested")
                        .to_string(),
                ),
                (
                    "Duration",
                    receipt["duration_ms"]
                        .as_u64()
                        .map(crate::views::format_duration)
                        .unwrap_or_else(|| "-".to_string()),
                ),
            ],
        );
        if let Some(reference) = receipt.get("reference") {
            ui.add_space(3.0);
            ui.label(
                RichText::new(format!(
                    "Ref {} · expected token {} · validation {}",
                    reference["ref"].as_str().unwrap_or("-"),
                    reference["expected_token"].as_str().unwrap_or("-"),
                    reference["validation"].as_str().unwrap_or("-")
                ))
                .size(9.0)
                .monospace()
                .color(theme::MUTED),
            );
        }
        for warning in receipt["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str())
        {
            ui.label(RichText::new(warning).size(9.0).color(theme::YELLOW));
        }
    });
}

fn recurrence_panel(app: &mut App, ui: &mut egui::Ui) {
    let explanation = app.triage.explanation.clone();
    let history = app.triage.history.clone();
    let history_loading = app.triage.history_loading;
    let history_error = app.triage.history_error.clone();
    card(ui, |ui| {
        ui.horizontal(|ui| {
            section_title(ui, "Failure recurrence");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if secondary_button(ui, "Refresh history").clicked()
                    && let Some(explanation) = &explanation
                {
                    app.refresh_recurrence_history(&explanation.fingerprint, ui.ctx());
                }
                if let Some(explanation) = &explanation {
                    copy_button(ui, &explanation.fingerprint, "Copy fingerprint");
                }
            });
        });
        let Some(explanation) = explanation else {
            muted(ui, "Load the explanation to derive recurrence memory.");
            return;
        };
        if history_loading {
            ui.horizontal(|ui| {
                ui.spinner();
                muted(ui, "Reading stored occurrences of this fingerprint...");
            });
        }
        if let Some(error) = &history_error {
            error_banner(ui, error);
        }
        match &explanation.recurrence {
            Some(recurrence) => {
                let color = match recurrence.status.as_str() {
                    "new" => theme::BLUE,
                    "recurring" => theme::YELLOW,
                    "regressed" => theme::RED,
                    _ => theme::MUTED,
                };
                ui.horizontal_wrapped(|ui| {
                    status_pill(ui, &recurrence.status, color);
                    status_pill(
                        ui,
                        if recurrence.currently_resolved {
                            "currently resolved"
                        } else {
                            "open"
                        },
                        if recurrence.currently_resolved {
                            theme::GREEN
                        } else {
                            theme::YELLOW
                        },
                    );
                    status_pill(
                        ui,
                        &format!("{} occurrence(s)", recurrence.occurrence_count),
                        theme::MUTED,
                    );
                });
                kv_grid(
                    ui,
                    "triage-recurrence",
                    &[
                        ("First seen", recurrence.first_seen.clone()),
                        ("Last seen", recurrence.last_seen.clone()),
                        (
                            "Resolved by",
                            recurrence
                                .resolved_by_run
                                .clone()
                                .unwrap_or_else(|| "-".to_string()),
                        ),
                        (
                            "Devices",
                            if recurrence.devices.is_empty() {
                                "-".to_string()
                            } else {
                                recurrence.devices.join(", ")
                            },
                        ),
                        (
                            "App versions",
                            if recurrence.app_versions.is_empty() {
                                "-".to_string()
                            } else {
                                recurrence.app_versions.join(", ")
                            },
                        ),
                    ],
                );
                for line in recurrence.uncertainty.iter().take(3) {
                    ui.label(RichText::new(line).size(9.0).color(theme::YELLOW));
                }
            }
            None => {
                warning_banner(
                    ui,
                    "Recurrence metadata is unavailable for this run. It may have failed too early to produce a comparable fingerprint.",
                );
            }
        }
        if let Some(history) = &history {
            if let Some(entry) = history
                .get("fingerprint")
                .filter(|entry| entry.get("occurrences").is_some())
            {
                ui.add_space(5.0);
                ui.label(RichText::new("Stored occurrences").size(10.0).strong());
                if let Some(occurrences) = entry["occurrences"].as_array() {
                    for occurrence in occurrences.iter().take(8) {
                        let run_id = occurrence["run_id"].as_str().unwrap_or("-");
                        let status = occurrence["run_status"].as_str().unwrap_or("-");
                        let step = occurrence["failed_step"].as_str().unwrap_or("-");
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{run_id}  {status}  {step}"))
                                    .size(9.5)
                                    .monospace(),
                            );
                            if ghost_button(ui, "Open").clicked() {
                                app.selected_run = Some(run_id.to_string());
                                app.set_page(Page::Runs);
                            }
                        });
                    }
                }
            }
        }
        small(
            ui,
            "Recurrence is derived from stored run reports. A passing run only proves resolution when it exercised the same project, scenario and failing step.",
        );
    });
}

fn bundle_panel(app: &mut App, ui: &mut egui::Ui) {
    let run_id = app.triage.run_id.clone();
    let preview = app.triage.bundle.preview.clone();
    let preview_loading = app.triage.bundle.preview_loading;
    let preview_error = app.triage.bundle.preview_error.clone();
    let result = app.triage.bundle.result.clone();
    let creating = app.triage.bundle.creating;
    let create_error = app.triage.bundle.create_error.clone();
    card(ui, |ui| {
        section_title(ui, "Diagnostic bundle");
        muted(
            ui,
            "Preview what will be included, then create a redacted ZIP under the project artifacts directory.",
        );
        let Some(run_id) = run_id else {
            warning_banner(ui, "Select a run before creating a bundle.");
            return;
        };
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if secondary_button(ui, "Preview inclusion").clicked() {
                app.preview_bundle(&run_id, ui.ctx());
            }
            let create = secondary_button(ui, "Create ZIP bundle");
            if create.clicked() && !creating {
                let default_name = format!("{run_id}-evidence.zip");
                let default_dir = app
                    .project_path
                    .as_ref()
                    .map(|path| path.join(".artifacts").join("bundles"));
                let mut dialog = rfd::FileDialog::new().set_file_name(&default_name);
                if let Some(dir) = default_dir {
                    let _ = std::fs::create_dir_all(&dir);
                    dialog = dialog.set_directory(dir);
                }
                if let Some(path) = dialog.save_file() {
                    app.create_bundle(&run_id, path, ui.ctx());
                }
            }
            if creating {
                ui.spinner();
                muted(ui, "Building and verifying the ZIP...");
            }
        });
        if preview_loading {
            ui.spinner();
        }
        if let Some(error) = &preview_error {
            error_banner(ui, error);
        }
        if let Some(preview) = &preview {
            let entries = preview
                .get("entries")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let excluded = preview
                .get("excluded")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let total = preview["total_payload_bytes"].as_u64().unwrap_or(0);
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "Inclusion preview: {} payload(s), {} bytes; {} excluded",
                    entries.len(),
                    total,
                    excluded.len()
                ))
                .size(10.0)
                .strong(),
            );
            for entry in entries.iter().take(8) {
                ui.label(
                    RichText::new(format!(
                        "  + {} ({})",
                        entry["path"].as_str().unwrap_or("?"),
                        entry["kind"].as_str().unwrap_or("?")
                    ))
                    .size(9.5)
                    .color(theme::GREEN),
                );
            }
            for item in excluded.iter().take(8) {
                ui.label(
                    RichText::new(format!(
                        "  - {} — {}",
                        item["source"].as_str().unwrap_or("?"),
                        item["reason"].as_str().unwrap_or("excluded")
                    ))
                    .size(9.5)
                    .color(theme::MUTED),
                );
            }
            if let Some(warnings) = preview["warnings"].as_array() {
                for warning in warnings.iter().filter_map(|value| value.as_str()) {
                    ui.label(RichText::new(warning).size(9.0).color(theme::YELLOW));
                }
            }
        }
        if let Some(error) = &create_error {
            error_banner(ui, error);
        }
        if let Some(result) = &result {
            let path = result["output"].as_str().unwrap_or("the bundle");
            ui.add_space(4.0);
            success_banner(ui, &format!("Bundle created and checksum-verified: {path}"));
            ui.horizontal(|ui| {
                if ghost_button(ui, "Open bundle folder").clicked() {
                    if let Some(parent) = std::path::Path::new(path).parent() {
                        let _ = open::that(parent);
                    }
                }
                copy_button(ui, path, "Copy path");
            });
        }
    });
}

fn signals_panel(app: &mut App, ui: &mut egui::Ui, report: &crate::models::RunReport) {
    card_fixed_height(ui, 320.0, "triage-signals", |ui| {
        section_title(ui, "Signals");
        let failure_count = report
            .steps
            .iter()
            .filter(|step| step.status == "failed")
            .count();
        kv_grid(
            ui,
            "triage-signals",
            &[
                (
                    "Steps",
                    format!("{} total · {failure_count} failed", report.steps.len()),
                ),
                ("Artifacts", format!("{} captured", report.artifacts.len())),
                (
                    "Build",
                    format!(
                        "{} {}",
                        report.build.package.as_deref().unwrap_or("-"),
                        report.build.version_name.as_deref().unwrap_or("-")
                    ),
                ),
            ],
        );
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if secondary_button(ui, "Open logcat").clicked()
                && let Some(session) = report.session_id.clone()
            {
                app.logcat_select_session(Some(session));
                app.set_page(Page::Logcat);
            }
            if secondary_button(ui, "Open inspector").clicked()
                && let Some(session) = report.session_id.clone()
            {
                app.inspector_select_session(Some(session));
                app.set_page(Page::Inspector);
                app.start_inspector(ui.ctx());
            }
        });
        ui.add_space(6.0);
        small(
            ui,
            "Flaky analysis below uses recent compatible terminal runs; it never labels a single failure as flaky.",
        );
    });
}

fn artifacts_panel(
    app: &mut App,
    ui: &mut egui::Ui,
    report: &crate::models::RunReport,
    path: &str,
) {
    section_label(ui, "Evidence");
    if report.artifacts.is_empty() {
        muted(ui, "No artifacts were recorded.");
        return;
    }
    let mut filter = app
        .run_detail
        .as_ref()
        .map(|state| state.evidence.filter.clone())
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Filter").size(10.0).color(theme::MUTED));
        ui.add(
            egui::TextEdit::singleline(&mut filter)
                .desired_width(240.0)
                .hint_text("screenshot, hierarchy, logcat..."),
        );
    });
    if let Some(state) = app.run_detail.as_mut() {
        state.evidence.filter = filter.clone();
    }
    let artifacts = report.artifacts.clone();
    let mut open_path = None;
    ui.horizontal_wrapped(|ui| {
        for artifact in &artifacts {
            let haystack = format!(
                "{} {} {}",
                artifact.kind,
                artifact.path,
                artifact.description.as_deref().unwrap_or("")
            )
            .to_ascii_lowercase();
            if !filter.is_empty() && !haystack.contains(&filter.to_ascii_lowercase()) {
                continue;
            }
            let label = format!(
                "{} {}",
                icon_for_kind(&artifact.kind),
                artifact.path.rsplit('/').next().unwrap_or(&artifact.path)
            );
            if ui
                .add(
                    egui::Button::new(RichText::new(label).size(9.5)).fill(theme::surface_fill(ui)),
                )
                .on_hover_text(&artifact.path)
                .clicked()
            {
                open_path = Some(format!("{}/{}", path.replace('\\', "/"), artifact.path));
            }
        }
    });
    if let Some(path) = open_path {
        let _ = open::that(path);
    }
}

fn flaky_panel(ui: &mut egui::Ui, flaky: &crate::models::FlakyReport) {
    section_title(ui, "Flaky-step analysis");
    muted(ui, &flaky.heuristic);
    ui.add_space(4.0);
    let interesting = flaky
        .steps
        .iter()
        .filter(|step| matches!(step.classification.as_str(), "flaky" | "stable_failure"))
        .take(8)
        .collect::<Vec<_>>();
    if interesting.is_empty() {
        muted(
            ui,
            "No flaky or consistently failing steps in the analyzed window.",
        );
        return;
    }
    let steps = interesting
        .iter()
        .map(|step| (*step).clone())
        .collect::<Vec<_>>();
    let width = ui.available_width();
    let widths = proportional_widths(width, &[0.55, 0.17, 0.14, 0.14], 10.0);
    let step_width = widths[0];
    let classification_width = widths[1];
    let pass_fail_width = widths[2];
    let runs_width = widths[3];
    table_header(
        ui,
        &[
            ("Step", step_width),
            ("Classification", classification_width),
            ("Pass / fail", pass_fail_width),
            ("Runs", runs_width),
        ],
    );
    for step in &steps {
        selectable_row(ui, false, theme::ROW_HEIGHT, |ui| {
            row_cell(ui, step_width, |ui| {
                ui.add(egui::Label::new(RichText::new(&step.step_name).size(10.0)).truncate());
            });
            row_cell(ui, classification_width, |ui| {
                status_pill(
                    ui,
                    &step.classification,
                    theme::status_color(&step.classification),
                );
            });
            row_cell(ui, pass_fail_width, |ui| {
                ui.label(
                    RichText::new(format!("{} / {}", step.successes, step.failures)).size(10.0),
                );
            });
            row_cell(ui, runs_width, |ui| {
                ui.label(RichText::new(step.executions.to_string()).size(10.0));
            });
        });
        ui.add_space(2.0);
    }
}

fn icon_for_kind(kind: &str) -> &'static str {
    if kind.contains("screenshot") {
        "▣"
    } else if kind.contains("log") {
        "≡"
    } else if kind.contains("hierarchy") {
        "⌘"
    } else {
        "·"
    }
}
