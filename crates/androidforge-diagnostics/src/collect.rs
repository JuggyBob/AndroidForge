use std::time::Duration;

use androidforge_adb::AdbClient;
use androidforge_app::AppManager;
use androidforge_core::error::Result;
use androidforge_core::model::ArtifactRef;
use androidforge_core::run::RunContext;
use androidforge_core::util::now_rfc3339;
use androidforge_ui::{UiHierarchy, UiManager};
use serde::{Deserialize, Serialize};

use crate::capture::capture_screenshot;
use crate::logcat::{collect_logcat_tail, detect_crashes, filter_logcat};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticDump {
    pub name: String,
    pub path: String,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionResult {
    pub package: Option<String>,
    pub serial: String,
    pub collected_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dumps: Vec<DiagnosticDump>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub focused_nodes: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logcat_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crashes: Option<crate::logcat::CrashReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_states: Option<androidforge_app::AppStates>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureBundle {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub focused_nodes: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_states: Option<androidforge_app::AppStates>,
}

impl FailureBundle {
    pub fn markdown_summary(&self) -> String {
        let mut out = format!("Failure evidence: {}\n", self.reason);
        for artifact in &self.artifacts {
            out.push_str(&format!("- [{}] {}\n", artifact.kind, artifact.path));
        }
        if !self.warnings.is_empty() {
            out.push_str("Warnings:\n");
            for warning in &self.warnings {
                out.push_str(&format!("- {warning}\n"));
            }
        }
        out
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn collect_diagnostics(
    adb: &AdbClient,
    ui: Option<&UiManager>,
    app: Option<&AppManager>,
    run: &mut RunContext,
    serial: &str,
    package: Option<&str>,
    log_lines: usize,
    max_log_bytes: usize,
    include_dumps: bool,
) -> Result<CollectionResult> {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let mut artifacts = Vec::new();
    let mut dumps = Vec::new();
    let mut warnings = Vec::new();
    let mut focused_nodes = Vec::new();
    let mut logcat_preview = None;
    let mut crashes = None;
    let mut app_states = None;

    match capture_screenshot(
        adb,
        serial,
        run,
        Some("diagnostics"),
        Duration::from_secs(30),
    )
    .await
    {
        Ok(screenshot) => artifacts.push(screenshot.artifact),
        Err(error) => warnings.push(format!("screenshot failed: {}", error.message())),
    }

    if let Some(ui) = ui {
        match ui.dump(serial).await {
            Ok(hierarchy) => {
                let relative = format!("ui/hierarchy-{stamp}.xml");
                match run.write_text_artifact(
                    &relative,
                    "ui_hierarchy",
                    Some("UIAutomator hierarchy at diagnostics time"),
                    &hierarchy.xml,
                ) {
                    Ok(artifact) => artifacts.push(artifact),
                    Err(error) => {
                        warnings.push(format!("could not write hierarchy: {}", error.message()))
                    }
                }
                focused_nodes = focused_nodes_from(&hierarchy);
            }
            Err(error) => warnings.push(format!("UI hierarchy failed: {}", error.message())),
        }
    }

    match collect_logcat_tail(adb, serial, package, Some(log_lines), Some(max_log_bytes)).await {
        Ok(logcat) => {
            logcat_preview = Some(logcat.preview.clone());
            let redacted_raw = run.redactor().redact(&logcat.raw);
            let relative = format!("logs/logcat-{stamp}.txt");
            match run.write_text_artifact(
                &relative,
                "logcat_raw",
                Some("Raw logcat tail collected without clearing the device buffer"),
                &redacted_raw,
            ) {
                Ok(artifact) => artifacts.push(artifact),
                Err(error) => {
                    warnings.push(format!("could not write raw logcat: {}", error.message()))
                }
            }
            let filtered = package
                .map(|package| filter_logcat(&redacted_raw, package, &logcat.pid_filter))
                .unwrap_or_else(|| redacted_raw.clone());
            match run.write_text_artifact(
                &format!("logs/logcat-filtered-{stamp}.txt"),
                "logcat_filtered",
                Some("Package/process-filtered logcat tail"),
                &filtered,
            ) {
                Ok(artifact) => artifacts.push(artifact),
                Err(error) => warnings.push(format!(
                    "could not write filtered logcat: {}",
                    error.message()
                )),
            }
            if !logcat.crash_buffer.trim().is_empty() {
                match run.write_text_artifact(
                    &format!("logs/crash-buffer-{stamp}.txt"),
                    "crash_buffer",
                    Some("Android crash logcat buffer"),
                    &logcat.crash_buffer,
                ) {
                    Ok(artifact) => artifacts.push(artifact),
                    Err(error) => {
                        warnings.push(format!("could not write crash buffer: {}", error.message()))
                    }
                }
            }
        }
        Err(error) => warnings.push(format!("logcat collection failed: {}", error.message())),
    }

    if let Some(package) = package {
        match detect_crashes(adb, serial, package).await {
            Ok(report) => {
                match run.write_text_artifact(
                    &format!("diagnostics/crashes-{stamp}.json"),
                    "crash_report",
                    Some("Crash/ANR/process-death detection report"),
                    &serde_json::to_string_pretty(&report).unwrap_or_default(),
                ) {
                    Ok(artifact) => artifacts.push(artifact),
                    Err(error) => {
                        warnings.push(format!("could not write crash report: {}", error.message()))
                    }
                }
                crashes = Some(report);
            }
            Err(error) => warnings.push(format!("crash detection failed: {}", error.message())),
        }
    }

    if let Some(app) = app {
        if let Some(package) = package {
            match app.status(serial, package).await {
                Ok(states) => {
                    if let Ok(text) = serde_json::to_string_pretty(&states) {
                        match run.write_text_artifact(
                            &format!("diagnostics/app-status-{stamp}.json"),
                            "app_status",
                            Some("Process, task, activity and foreground state"),
                            &text,
                        ) {
                            Ok(artifact) => artifacts.push(artifact),
                            Err(error) => warnings
                                .push(format!("could not write app status: {}", error.message())),
                        }
                    }
                    app_states = Some(states);
                }
                Err(error) => warnings.push(format!("app status failed: {}", error.message())),
            }
        }
    }

    if include_dumps {
        let mut services = vec!["activity", "package"];
        if package.is_some() {
            services.extend(["notification", "jobscheduler", "alarm"]);
        }
        services.extend(["deviceidle", "battery", "connectivity"]);
        for service in services {
            match adb.dumpsys(serial, service).await {
                Ok(text) => {
                    let relative = format!("diagnostics/{}-{stamp}.txt", service.replace('.', "_"));
                    match run.write_text_artifact(&relative, "dumpsys", Some(service), &text) {
                        Ok(artifact) => {
                            dumps.push(DiagnosticDump {
                                name: service.to_string(),
                                path: artifact.path.clone(),
                                bytes: artifact.size_bytes.unwrap_or(0),
                                note: None,
                            });
                            artifacts.push(artifact);
                        }
                        Err(error) => warnings.push(format!(
                            "could not write dumpsys {service}: {}",
                            error.message()
                        )),
                    }
                }
                Err(error) => {
                    warnings.push(format!("dumpsys {service} failed: {}", error.message()))
                }
            }
        }
    }

    let metadata = serde_json::json!({
        "schema_version": 1,
        "serial": serial,
        "package": package,
        "collected_at": now_rfc3339(),
        "focused_nodes": focused_nodes.clone(),
        "warnings": warnings.clone(),
    });
    let _ = run.write_text_artifact(
        &format!("diagnostics/metadata-{stamp}.json"),
        "diagnostics_metadata",
        Some("Diagnostics collection metadata"),
        &serde_json::to_string_pretty(&metadata).unwrap_or_default(),
    );

    Ok(CollectionResult {
        package: package.map(str::to_string),
        serial: serial.to_string(),
        collected_at: now_rfc3339(),
        artifacts,
        dumps,
        warnings,
        focused_nodes,
        logcat_preview,
        crashes,
        app_states,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn capture_failure_bundle(
    adb: &AdbClient,
    ui: Option<&UiManager>,
    app: Option<&AppManager>,
    run: &mut RunContext,
    serial: &str,
    package: Option<&str>,
    reason: &str,
    include_dumps: bool,
) -> FailureBundle {
    let result = collect_diagnostics(
        adb,
        ui,
        app,
        run,
        serial,
        package,
        if package.is_some() { 800 } else { 300 },
        4 * 1024 * 1024,
        include_dumps,
    )
    .await;
    match result {
        Ok(result) => FailureBundle {
            reason: reason.to_string(),
            artifacts: result.artifacts,
            warnings: result.warnings,
            focused_nodes: result.focused_nodes,
            app_states: result.app_states,
        },
        Err(error) => FailureBundle {
            reason: reason.to_string(),
            artifacts: error.body().artifacts.clone(),
            warnings: vec![format!(
                "failure evidence collection failed: {}",
                error.message()
            )],
            focused_nodes: Vec::new(),
            app_states: None,
        },
    }
}

fn focused_nodes_from(hierarchy: &UiHierarchy) -> Vec<serde_json::Value> {
    hierarchy
        .focused()
        .into_iter()
        .map(|node| {
            serde_json::json!({
                "path": node.path,
                "text": node.text,
                "content_desc": node.content_desc,
                "resource_id": node.resource_id,
                "class": node.class,
                "bounds": node.bounds,
                "password": node.password,
            })
        })
        .collect()
}
