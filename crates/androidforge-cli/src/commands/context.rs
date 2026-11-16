use std::time::Instant;

use androidforge_core::error::{ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::run::{RunContext, RunStatus, list_runs};
use androidforge_core::session::{Session, SessionState};
use androidforge_core::util::now_rfc3339;
use serde_json::{Value, json};

use crate::cli::{Cli, ContextArgs};
use crate::commands::common::success;
use crate::commands::observation::{
    Observation, collect_observation, latest_observation, load_observation, verify_boot_id,
};
use crate::context::CliContext;

const SECTIONS: &[&str] = &[
    "project",
    "environment",
    "session",
    "device",
    "app",
    "screen",
    "last-run",
];

pub async fn context(cli: &Cli, args: &ContextArgs) -> Result<CommandSuccess> {
    let started = Instant::now();
    let ctx = CliContext::new(cli)?;
    for requested in &args.include {
        if !SECTIONS.contains(&requested.as_str()) {
            return Err(ForgeError::invalid_argument(format!(
                "unknown context section '{requested}'"
            ))
            .with_suggestion(format!("Use one or more of: {}", SECTIONS.join(", "))));
        }
    }
    let include = |section: &str| {
        if args.no_screen && section == "screen" {
            return false;
        }
        args.include.is_empty() || args.include.iter().any(|value| value == section)
    };
    let mut warnings = ctx.config.warnings.clone();
    if ctx.paths.project_config_path.is_none() {
        warnings.push(
            "No project adapter was found; package, AVD and fixture defaults are unavailable."
                .to_string(),
        );
    }

    let project = if include("project") {
        json!({
            "available": true,
            "name": ctx.config.project_name(),
            "root": ctx.paths.project_root,
            "config": ctx.paths.project_config_path,
            "package": ctx.config.package(),
            "debug_package": ctx.config.debug_package(),
            "preferred_avd": ctx.config.file.project.default_avd,
            "default_device": ctx.config.file.project.default_device,
            "artifact_dir": ctx.paths.artifacts_dir,
            "valid": ctx.config.validate().valid,
        })
    } else {
        Value::Null
    };

    let toolchain = &ctx.toolchain;
    let environment = if include("environment") {
        json!({
            "available": true,
            "cli_version": env!("CARGO_PKG_VERSION"),
            "schema_version": 1,
            "sdk": toolchain.sdk_root,
            "adb": toolchain.adb,
            "emulator": toolchain.emulator,
            "java": toolchain.java,
            "aapt2": toolchain.aapt2,
            "apksigner": toolchain.apksigner,
            "maestro": toolchain.maestro,
            "android_home": std::env::var("ANDROID_HOME").ok(),
            "android_sdk_root": std::env::var("ANDROID_SDK_ROOT").ok(),
            "java_home": std::env::var("JAVA_HOME").ok(),
        })
    } else {
        Value::Null
    };

    let requested_session = ctx.session_arg.clone().or_else(|| {
        std::env::var("ANDROIDFORGE_SESSION")
            .ok()
            .filter(|value| !value.trim().is_empty())
    });
    let session_result = match requested_session.as_deref() {
        Some(id) => Session::load(&ctx.paths, id).map_err(|error| error.body().message.clone()),
        None => match ctx.session_id() {
            Ok(Some(id)) => {
                Session::load(&ctx.paths, &id).map_err(|error| error.body().message.clone())
            }
            Ok(None) => Err("no session is active".to_string()),
            Err(error) => Err(error.body().message.clone()),
        },
    };
    let session = session_result.as_ref().ok().cloned();
    let session_value = if include("session") {
        match &session {
            Some(session) => json!({
                "available": true,
                "id": session.id,
                "owner": session.owner,
                "state": session.state,
                "kind": session.kind,
                "serial": session.serial,
                "avd": session.avd,
                "port": session.port,
                "headless": session.headless,
                "profile": session.profile,
                "created_at": session.created_at,
                "last_activity": session.heartbeat_at,
                "build": session.build,
                "lease_expires_at": session.lease_expires_at,
                "lease_expired": session.lease_expired(),
            }),
            None => json!({
                "available": false,
                "reason": session_result.err().unwrap_or_else(|| "no session is active".to_string()),
            }),
        }
    } else {
        Value::Null
    };

    let mut device = None;
    let mut device_error = None;
    if let Some(session) = &session {
        match ctx.adb().map(|adb| (adb, session.serial.clone())) {
            Ok((adb, Some(serial))) => match adb.inspect(&serial).await {
                Ok(value) => device = Some(value),
                Err(error) => device_error = Some(error.body().message.clone()),
            },
            Ok((_, None)) => {
                device_error = Some("session has no device serial yet".to_string());
            }
            Err(error) => device_error = Some(error.body().message.clone()),
        }
    }
    let device_value = if include("device") {
        match &device {
            Some(device) => json!({
                "available": true,
                "serial": device.serial,
                "kind": device.kind,
                "state": device.state,
                "model": device.model,
                "manufacturer": device.manufacturer,
                "android_version": device.android_version,
                "api_level": device.api_level,
                "architecture": device.architecture,
                "resolution": device.resolution,
                "density": device.density,
                "boot_completed": device.boot_completed,
                "owner": session.as_ref().map(|session| json!({
                    "session_id": session.id,
                    "owner": session.owner,
                    "state": session.state,
                })),
            }),
            None => json!({
                "available": false,
                "reason": device_error.unwrap_or_else(|| {
                    if session.is_some() {
                        "the session device is not reachable".to_string()
                    } else {
                        "no session is active".to_string()
                    }
                }),
            }),
        }
    } else {
        Value::Null
    };

    let package = ctx.config.debug_package().map(str::to_string);
    let mut app_value = Value::Null;
    if include("app") {
        app_value = match (&device, &package) {
            (Some(device), Some(package)) => match ctx.app_manager() {
                Ok(manager) => match manager.status(&device.serial, package).await {
                    Ok(status) => json!({
                        "available": true,
                        "package": status.package,
                        "installed": status.installed,
                        "version_name": status.version_name,
                        "version_code": status.version_code,
                        "debuggable": status.debuggable,
                        "process_exists": status.process_exists,
                        "process_alive": status.process_alive,
                        "task_exists": status.task_exists,
                        "activity_exists": status.activity_exists,
                        "foreground": status.foreground,
                        "resumed": status.resumed,
                        "foreground_app": status.foreground_app,
                        "top_resumed_activity": status.top_resumed_activity,
                        "notes": status.notes,
                    }),
                    Err(error) => json!({
                        "available": false,
                        "package": package,
                        "reason": error.body().message,
                        "code": error.body().code.as_str(),
                    }),
                },
                Err(error) => json!({
                    "available": false,
                    "package": package,
                    "reason": error.body().message,
                }),
            },
            (None, Some(package)) => json!({
                "available": false,
                "package": package,
                "reason": "no reachable device",
            }),
            _ => json!({
                "available": false,
                "reason": "no package is configured for this project",
            }),
        };
    }

    let mut screen_value = Value::Null;
    let mut observation: Option<Observation> = None;
    if include("screen") {
        if let (Some(session), Some(device)) = (&session, &device) {
            if ctx.dry_run {
                observation = latest_observation(&ctx.paths, &session.id)
                    .filter(|observation| observation.age_ms <= 5_000);
                if observation.is_none() {
                    screen_value = json!({
                        "available": false,
                        "dry_run": true,
                        "reason": "dry-run: no recent screen observation was captured or reused",
                    });
                }
            } else {
                let cached = if args.refresh_screen {
                    None
                } else {
                    latest_observation(&ctx.paths, &session.id)
                        .filter(|observation| observation.age_ms <= 5_000)
                };
                let cached = match cached {
                    Some(cached) => match load_observation(&ctx.paths, &session.id, &cached.id) {
                        Ok(record) if verify_boot_id(&ctx, session, &record).await.is_ok() => {
                            Some(cached)
                        }
                        _ => None,
                    },
                    None => None,
                };
                match cached {
                    Some(cached) => observation = Some(cached),
                    None => match collect_observation(&ctx, session, device, None, args.full).await
                    {
                        Ok(value) => observation = Some(value),
                        Err(error) => {
                            screen_value = json!({
                                "available": false,
                                "reason": error.body().message,
                                "code": error.body().code.as_str(),
                            });
                        }
                    },
                }
            }
            if let Some(observation) = &observation {
                let mut value = serde_json::to_value(observation)?;
                if !args.full
                    && let Some(object) = value.as_object_mut()
                {
                    object.remove("nodes");
                    let needs_cap = object
                        .get("actionable")
                        .and_then(|value| value.as_array())
                        .is_some_and(|actionable| actionable.len() > 16);
                    if needs_cap {
                        if let Some(actionable) = object
                            .get_mut("actionable")
                            .and_then(|value| value.as_array_mut())
                        {
                            actionable.truncate(16);
                        }
                        object.insert(
                            "actionable_truncated".to_string(),
                            serde_json::Value::Bool(true),
                        );
                    }
                    if let Some(text) = object
                        .get_mut("visible_text")
                        .and_then(|value| value.as_array_mut())
                    {
                        text.truncate(24);
                    }
                }
                screen_value = value;
            }
        } else {
            screen_value = json!({
                "available": false,
                "reason": "screen observation needs both a session and a reachable device",
            });
        }
    }

    let runs = list_runs(&ctx.paths.artifacts_dir).unwrap_or_default();
    let latest = runs.first().cloned();
    let last_run_value = if include("last-run") {
        match &latest {
            Some(summary) => {
                let mut value = json!({
                    "available": true,
                    "run_id": summary.run_id,
                    "project": summary.project,
                    "scenario": summary.command,
                    "status": summary.status,
                    "started_at": summary.started_at,
                    "duration_ms": summary.duration_ms,
                    "failed_steps": summary.failed_steps,
                    "steps": summary.steps,
                    "path": summary.path,
                });
                if let Ok(run) =
                    RunContext::open(&ctx.paths.artifacts_dir, &summary.run_id, ctx.redactor())
                {
                    if let Some(failure) = &run.report.failure {
                        value["failure"] = json!({
                            "code": failure.code.as_str(),
                            "message": failure.message,
                            "failed_operation": failure.failed_operation,
                            "recoverable": failure.recoverable,
                        });
                    }
                    if args.full {
                        value["report"] = serde_json::to_value(&run.report)?;
                    }
                }
                value
            }
            None => json!({
                "available": false,
                "reason": "no runs have been recorded for this project",
            }),
        }
    } else {
        Value::Null
    };

    let next_actions = build_next_actions(
        &ctx,
        session.as_ref(),
        device.as_ref(),
        &observation,
        &latest,
        &package,
    );
    let mut result = json!({
        "schema_version": 1,
        "generated_at": now_rfc3339(),
        "duration_ms": started.elapsed().as_millis() as u64,
        "project": project,
        "environment": environment,
        "session": session_value,
        "device": device_value,
        "app": app_value,
        "screen": screen_value,
        "last_run": last_run_value,
        "next_actions": next_actions,
        "warnings": warnings,
    });
    if !args.full
        && let Some(object) = result.as_object_mut()
        && let Some(warnings) = object
            .get_mut("warnings")
            .and_then(|value| value.as_array_mut())
    {
        warnings.truncate(10);
    }
    let session_label = session
        .as_ref()
        .map(|session| session.id.as_str())
        .unwrap_or("no session");
    let human = format!(
        "Context: {} · {} · {} · {}ms\n",
        ctx.config.project_name(),
        session_label,
        device
            .as_ref()
            .map(|device| device.serial.as_str())
            .unwrap_or("no device"),
        started.elapsed().as_millis()
    );
    Ok(success("context", result, human))
}

fn action(reason: impl Into<String>, argv: Vec<String>, priority: &str) -> Value {
    json!({
        "reason": reason.into(),
        "priority": priority,
        "command": { "argv": argv },
    })
}

fn build_next_actions(
    ctx: &CliContext,
    session: Option<&Session>,
    device: Option<&androidforge_core::model::Device>,
    observation: &Option<Observation>,
    latest: &Option<androidforge_core::run::RunSummary>,
    package: &Option<String>,
) -> Vec<Value> {
    let mut actions = Vec::new();
    if ctx.paths.project_config_path.is_none() {
        actions.push(action(
            "No project adapter is configured; package and AVD defaults are unavailable.",
            vec![
                "androidforge".to_string(),
                "config".to_string(),
                "validate".to_string(),
                "--json".to_string(),
            ],
            "high",
        ));
    }
    match session {
        None => {
            let mut ensure = vec![
                "androidforge".to_string(),
                "session".to_string(),
                "ensure".to_string(),
            ];
            if let Some(avd) = &ctx.config.file.project.default_avd {
                ensure.extend(["--avd".to_string(), avd.clone()]);
            } else if let Some(serial) = &ctx.config.file.project.default_device {
                ensure.extend(["--serial".to_string(), serial.clone()]);
            }
            if ensure.len() > 3 {
                ensure.push("--json".to_string());
                actions.push(action(
                    "No session is active; acquire a session for the configured target.",
                    ensure,
                    "high",
                ));
            } else {
                actions.push(action(
                    "No session is active and no default AVD or device is configured.",
                    vec![
                        "androidforge".to_string(),
                        "avd".to_string(),
                        "list".to_string(),
                        "--json".to_string(),
                    ],
                    "high",
                ));
                actions.push(action(
                    "Inspect connected devices before starting or attaching a session.",
                    vec![
                        "androidforge".to_string(),
                        "devices".to_string(),
                        "list".to_string(),
                        "--json".to_string(),
                    ],
                    "high",
                ));
            }
        }
        Some(session) if session.lease_expired() => {
            actions.push(action(
                "The session lease is expired; inspect it before relying on the device.",
                vec![
                    "androidforge".to_string(),
                    "session".to_string(),
                    "inspect".to_string(),
                    "--session".to_string(),
                    session.id.clone(),
                    "--json".to_string(),
                ],
                "high",
            ));
        }
        Some(session)
            if matches!(
                session.state,
                SessionState::Starting | SessionState::Stopping
            ) =>
        {
            actions.push(action(
                "The session is still changing state; wait for readiness.",
                vec![
                    "androidforge".to_string(),
                    "session".to_string(),
                    "status".to_string(),
                    "--session".to_string(),
                    session.id.clone(),
                    "--json".to_string(),
                ],
                "high",
            ));
        }
        Some(_) => {}
    }
    if device.is_none() && session.is_some() {
        actions.push(action(
            "The session's device is not reachable.",
            vec![
                "androidforge".to_string(),
                "devices".to_string(),
                "list".to_string(),
                "--json".to_string(),
            ],
            "high",
        ));
    }
    if observation.is_none() && device.is_some() {
        let mut observe = vec![
            "androidforge".to_string(),
            "ui".to_string(),
            "observe".to_string(),
        ];
        if let Some(session) = session {
            observe.extend(["--session".to_string(), session.id.clone()]);
        }
        observe.push("--json".to_string());
        actions.push(action(
            "The screen has not been observed recently.",
            observe,
            "high",
        ));
    }
    if let Some(package) = package
        && device.is_some()
        && session.is_some()
    {
        let mut observe = vec![
            "androidforge".to_string(),
            "ui".to_string(),
            "observe".to_string(),
        ];
        if let Some(session) = session {
            observe.extend(["--session".to_string(), session.id.clone()]);
        }
        observe.extend([
            "--package".to_string(),
            package.clone(),
            "--json".to_string(),
        ]);
        actions.push(action(
            "Filter the next observation to the configured application.",
            observe,
            "normal",
        ));
    }
    if let Some(run) = latest
        && !matches!(run.status, RunStatus::Passed)
    {
        actions.push(action(
            "The latest run is not successful; get a deterministic failure explanation.",
            vec![
                "androidforge".to_string(),
                "report".to_string(),
                "explain".to_string(),
                run.run_id.clone(),
                "--json".to_string(),
            ],
            "high",
        ));
    }
    actions
}
