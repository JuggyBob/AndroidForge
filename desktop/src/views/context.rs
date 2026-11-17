use eframe::egui;
use egui::RichText;
use serde_json::Value;

use crate::app::App;
use crate::components::*;
use crate::state::Page;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    page_header(
        ui,
        "Context",
        "One read-only snapshot of the project, session, device, app, screen and last run.",
    );
    ui.horizontal(|ui| {
        if primary_button(ui, "Refresh context").clicked() {
            app.refresh_context(ui.ctx());
        }
        if app.context.loading {
            ui.spinner();
            muted(ui, "Reading the CLI context snapshot...");
        }
        if let Some(updated) = app.context.updated {
            muted(
                ui,
                format!("Snapshot age: {:.1}s", updated.elapsed().as_secs_f64()),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ghost_button(ui, "Open project").clicked()
                && let Some(project) = &app.project_path
            {
                let _ = open::that(project);
            }
        });
    });
    if let Some(error) = &app.context.error {
        error_banner(ui, error);
    }
    let Some(snapshot) = app.context.snapshot.clone() else {
        if !app.context.loading {
            empty_state(
                ui,
                "No context snapshot yet",
                "Refresh to read project/session/device/app/screen/last-run data from the CLI.",
            );
        }
        return;
    };
    ui.add_space(8.0);
    ui.columns(2, |columns| {
        project_card(&mut columns[0], &snapshot);
        session_card(&mut columns[1], &snapshot);
    });
    ui.add_space(10.0);
    ui.columns(2, |columns| {
        device_card(&mut columns[0], &snapshot);
        app_card(&mut columns[1], &snapshot);
    });
    ui.add_space(10.0);
    ui.columns(2, |columns| {
        screen_card(&mut columns[0], &snapshot);
        last_run_card(&mut columns[1], &snapshot);
    });
    ui.add_space(10.0);
    actions_panel(app, ui, &snapshot);
    warnings_panel(ui, &snapshot);
}

fn text(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    if current.is_null() {
        return None;
    }
    match current {
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Array(items) => Some(format!("{} item(s)", items.len())),
        _ => Some(current.to_string()),
    }
}

fn show_row(ui: &mut egui::Ui, key: &str, value: Option<String>) {
    ui.label(RichText::new(key).size(9.5).color(theme::MUTED));
    ui.label(RichText::new(value.unwrap_or_else(|| "unavailable".to_string())).size(10.0));
}

fn project_card(ui: &mut egui::Ui, snapshot: &Value) {
    let project = &snapshot["project"];
    card(ui, |ui| {
        section_title(ui, "Project");
        show_row(ui, "Name", text(project, &["name"]));
        show_row(ui, "Root", text(project, &["root"]));
        show_row(ui, "Package", text(project, &["package"]));
        show_row(ui, "Debug package", text(project, &["debug_package"]));
        show_row(ui, "Adapter valid", text(project, &["valid"]));
        show_row(ui, "Preferred AVD", text(project, &["preferred_avd"]));
        show_row(ui, "Artifacts", text(project, &["artifact_dir"]));
    });
}

fn session_card(ui: &mut egui::Ui, snapshot: &Value) {
    let session = &snapshot["session"];
    card(ui, |ui| {
        section_title(ui, "Session and ownership");
        if session["available"].as_bool() != Some(true) {
            warning_banner(
                ui,
                session["reason"]
                    .as_str()
                    .unwrap_or("No active session is available."),
            );
            return;
        }
        show_row(ui, "Session", text(session, &["id"]));
        show_row(ui, "Owner", text(session, &["owner"]));
        show_row(ui, "State", text(session, &["state"]));
        show_row(ui, "Device", text(session, &["serial"]));
        show_row(ui, "Profile", text(session, &["profile"]));
        show_row(ui, "Lease expires", text(session, &["lease_expires_at"]));
        show_row(ui, "Lease expired", text(session, &["lease_expired"]));
        if session["build"]["package"].as_str().is_some() {
            show_row(
                ui,
                "Installed build",
                Some(format!(
                    "{} {}",
                    session["build"]["package"].as_str().unwrap_or("-"),
                    session["build"]["version_name"].as_str().unwrap_or("-")
                )),
            );
        }
    });
}

fn device_card(ui: &mut egui::Ui, snapshot: &Value) {
    let device = &snapshot["device"];
    card(ui, |ui| {
        section_title(ui, "Device");
        if device["available"].as_bool() != Some(true) {
            warning_banner(
                ui,
                device["reason"]
                    .as_str()
                    .unwrap_or("No reachable device is available."),
            );
            return;
        }
        show_row(ui, "Serial", text(device, &["serial"]));
        show_row(ui, "State", text(device, &["state"]));
        show_row(ui, "Model", text(device, &["model"]));
        show_row(ui, "Android", text(device, &["android_version"]));
        show_row(ui, "API level", text(device, &["api_level"]));
        show_row(ui, "Resolution", text(device, &["resolution"]));
        show_row(ui, "Boot completed", text(device, &["boot_completed"]));
    });
}

fn app_card(ui: &mut egui::Ui, snapshot: &Value) {
    let app = &snapshot["app"];
    card(ui, |ui| {
        section_title(ui, "Application");
        if app["available"].as_bool() != Some(true) {
            warning_banner(
                ui,
                app["reason"]
                    .as_str()
                    .unwrap_or("Application state is unavailable."),
            );
            return;
        }
        show_row(ui, "Package", text(app, &["package"]));
        show_row(ui, "Installed", text(app, &["installed"]));
        show_row(ui, "Version", text(app, &["version_name"]));
        show_row(ui, "Version code", text(app, &["version_code"]));
        show_row(ui, "Foreground", text(app, &["foreground"]));
        show_row(ui, "Process alive", text(app, &["process_alive"]));
        show_row(ui, "Top activity", text(app, &["top_resumed_activity"]));
    });
}

fn screen_card(ui: &mut egui::Ui, snapshot: &Value) {
    let screen = &snapshot["screen"];
    card(ui, |ui| {
        section_title(ui, "Current screen");
        if screen["available"].as_bool() == Some(false) {
            warning_banner(
                ui,
                screen["reason"]
                    .as_str()
                    .unwrap_or("No screen observation is available."),
            );
            return;
        }
        show_row(
            ui,
            "Foreground package",
            text(screen, &["foreground", "package"]),
        );
        show_row(
            ui,
            "Foreground activity",
            text(screen, &["foreground", "activity"]),
        );
        show_row(ui, "Observed at", text(screen, &["observed_at"]));
        show_row(ui, "Observation token", text(screen, &["token"]));
        show_row(ui, "Age (ms)", text(screen, &["age_ms"]));
        show_row(ui, "Nodes", text(screen, &["hierarchy", "node_count"]));
        if let Some(texts) = screen["visible_text"].as_array() {
            let joined = texts
                .iter()
                .filter_map(Value::as_str)
                .take(8)
                .collect::<Vec<_>>()
                .join(" · ");
            show_row(ui, "Visible text", (!joined.is_empty()).then_some(joined));
        }
        if let Some(actionable) = screen["actionable"].as_array() {
            ui.add_space(3.0);
            ui.label(RichText::new("Actionable elements").size(10.0).strong());
            for node in actionable.iter().take(8) {
                let label = node["text"]
                    .as_str()
                    .or(node["content_desc"].as_str())
                    .or(node["resource_id"].as_str())
                    .unwrap_or("(no label)");
                ui.label(
                    RichText::new(format!(
                        "{}  {}{}",
                        if node["clickable"].as_bool().unwrap_or(false) {
                            "▣"
                        } else {
                            "·"
                        },
                        label,
                        if node["enabled"].as_bool() == Some(false) {
                            " (disabled)"
                        } else {
                            ""
                        }
                    ))
                    .size(9.5)
                    .color(theme::MUTED),
                );
                if let (Some(reference), Some(token)) =
                    (node["ref"].as_str(), screen["token"].as_str())
                {
                    let command = format!(
                        "androidforge ui tap --ref {reference} --expect-token {token} --json"
                    );
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(reference)
                                .size(8.5)
                                .monospace()
                                .color(theme::BLUE),
                        );
                        copy_button(ui, &command, "Copy ref action");
                    });
                }
            }
        }
    });
}

fn last_run_card(ui: &mut egui::Ui, snapshot: &Value) {
    let last = &snapshot["last-run"];
    card(ui, |ui| {
        section_title(ui, "Last run");
        if last["available"].as_bool() != Some(true) {
            warning_banner(
                ui,
                last["reason"].as_str().unwrap_or("No runs are available."),
            );
            return;
        }
        show_row(ui, "Run", text(last, &["run_id"]));
        show_row(ui, "Scenario", text(last, &["scenario"]));
        show_row(ui, "Status", text(last, &["status"]));
        show_row(ui, "Started", text(last, &["started_at"]));
        show_row(ui, "Duration (ms)", text(last, &["duration_ms"]));
        show_row(ui, "Failed steps", text(last, &["failed_steps"]));
        if let Some(failure) = last.get("failure") {
            if let Some(message) = failure["message"].as_str() {
                ui.add_space(3.0);
                error_banner(
                    ui,
                    &format!(
                        "{}: {}",
                        failure["code"].as_str().unwrap_or("failure"),
                        message
                    ),
                );
            }
        }
    });
}

fn actions_panel(app: &mut App, ui: &mut egui::Ui, snapshot: &Value) {
    card(ui, |ui| {
        section_title(ui, "Suggested next commands");
        let Some(actions) = snapshot["next_actions"].as_array() else {
            muted(ui, "No next actions were suggested for this state.");
            return;
        };
        if actions.is_empty() {
            muted(ui, "No next actions were suggested for this state.");
            return;
        }
        for action in actions {
            let reason = action["reason"].as_str().unwrap_or("Next action");
            let argv = action["command"]["argv"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            ui.add_space(2.0);
            ui.label(RichText::new(reason).size(9.5).color(theme::MUTED));
            ui.horizontal(|ui| {
                ui.label(RichText::new(&argv).size(9.5).monospace());
                copy_button(ui, &argv, "Copy");
            });
        }
        ui.add_space(4.0);
        if secondary_button(ui, "Open runs").clicked() {
            app.set_page(Page::Runs);
        }
    });
}

fn warnings_panel(ui: &mut egui::Ui, snapshot: &Value) {
    let warnings = snapshot["warnings"].as_array();
    if warnings.is_none_or(|warnings| warnings.is_empty()) {
        return;
    }
    ui.add_space(10.0);
    card(ui, |ui| {
        section_title(ui, "Warnings");
        if let Some(warnings) = warnings {
            for warning in warnings {
                if let Some(warning) = warning.as_str() {
                    warning_banner(ui, warning);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_nested_snapshot_values_without_faking_missing_data() {
        let snapshot = json!({
            "project": {"name": "Example", "valid": true},
            "session": {"available": false, "reason": "no session"},
            "screen": {"visible_text": ["One", "Two"]}
        });
        assert_eq!(
            text(&snapshot, &["project", "name"]).as_deref(),
            Some("Example")
        );
        assert_eq!(
            text(&snapshot, &["project", "valid"]).as_deref(),
            Some("true")
        );
        assert_eq!(text(&snapshot, &["session", "id"]), None);
        assert_eq!(
            text(&snapshot["screen"], &["visible_text"]).as_deref(),
            Some("2 item(s)")
        );
    }
}
