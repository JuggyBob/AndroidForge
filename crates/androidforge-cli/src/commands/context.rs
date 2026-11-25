use std::time::Instant;

use androidforge_core::error::{ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::run::{RunContext, RunStatus, list_runs};
use androidforge_core::session::{Session, SessionState, owner_of_serial};
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
    let mut session = session_result.as_ref().ok().cloned();
    let mut session_ignored = None;
    if let Some(candidate) = &session
        && let Some(project_root) = &candidate.project_root
        && !crate::context::same_project_root(
            project_root,
            &ctx.paths.project_root.to_string_lossy(),
        )
    {
        let reason = format!(
            "Session {} is bound to project {}, not {}; it was ignored.",
            candidate.id,
            project_root,
            ctx.paths.project_root.display()
        );
        warnings.push(reason.clone());
        session_ignored = Some(reason);
        session = None;
    }

    // Advisory signals only. They reuse persisted Session fields and the same
    // semantics as `session list/inspect/status`; no lock is created or enforced.
    let lock_serial = session
        .as_ref()
        .and_then(|session| session.serial.as_deref())
        .or(ctx.config.file.project.default_device.as_deref());
    let mut lock_owner = None;
    if let Some(serial) = lock_serial {
        match owner_of_serial(&ctx.paths, serial) {
            Ok(owner) => lock_owner = owner,
            Err(error) => warnings.push(error.body().message.clone()),
        }
    }
    warnings.extend(advisory_warnings(
        session.as_ref(),
        lock_owner.as_ref(),
        lock_serial,
    ));

    let session_value = if include("session") {
        match &session {
            Some(session) => {
                let advisory = session_advisory(session);
                let mut value = json!({
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
                    "project_root": session.project_root,
                    "created_at": session.created_at,
                    "last_activity": session.heartbeat_at,
                    "build": session.build,
                    "lease_expires_at": session.lease_expires_at,
                    "lease_expired": session.lease_expired(),
                });
                if let (Some(object), Some(advisory)) =
                    (value.as_object_mut(), advisory.as_object())
                {
                    for (key, signal) in advisory {
                        object.insert(key.clone(), signal.clone());
                    }
                }
                value
            }
            None => json!({
                "available": false,
                "reason": session_ignored
                    .clone()
                    .or_else(|| session_result.err())
                    .unwrap_or_else(|| "no session is active".to_string()),
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
                        .is_some_and(|actionable| actionable.len() > 12);
                    if needs_cap {
                        if let Some(actionable) = object
                            .get_mut("actionable")
                            .and_then(|value| value.as_array_mut())
                        {
                            actionable.truncate(12);
                        }
                        object.insert(
                            "actionable_truncated".to_string(),
                            serde_json::Value::Bool(true),
                        );
                    }
                    let needs_context_cap = object
                        .get("context")
                        .and_then(|value| value.as_array())
                        .is_some_and(|context| context.len() > 8);
                    if needs_context_cap {
                        if let Some(context) = object
                            .get_mut("context")
                            .and_then(|value| value.as_array_mut())
                        {
                            context.truncate(8);
                        }
                        object.insert(
                            "context_truncated".to_string(),
                            serde_json::Value::Bool(true),
                        );
                    }
                    if let Some(text) = object
                        .get_mut("visible_text")
                        .and_then(|value| value.as_array_mut())
                    {
                        text.truncate(16);
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

/// Advisory signals derived only from persisted `Session` fields. These mirror
/// the fields added by `session inspect` / `session status` and never touch a
/// device or modify a lock.
pub(crate) fn session_advisory(session: &Session) -> Value {
    json!({
        "heartbeat_age_seconds": session.age_seconds(),
        "stale": session.is_stale(),
        "operation_active": session.operation_active(),
        "operation": session.operation.as_ref().map(|operation| operation.operation.clone()),
        "lease_expires_at": session.lease_expires_at,
        "lease_expired": session.lease_expired(),
    })
}

/// Warnings are emitted only for actionable states: an old heartbeat, a live
/// operation, an expired advisory lease, or a device lock held by a different
/// session. Healthy sessions and our own lock stay silent.
pub(crate) fn advisory_warnings(
    session: Option<&Session>,
    lock_owner: Option<&Session>,
    serial: Option<&str>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(session) = session {
        if session.is_stale() {
            warnings.push(format!(
                "Session {} heartbeat is {}s old (stale); run `androidforge session gc --reclaim --dry-run` to preview cleanup.",
                session.id,
                session.age_seconds()
            ));
        }
        if session.operation_active() {
            let operation = session
                .operation
                .as_ref()
                .map(|lock| lock.operation.as_str())
                .unwrap_or("unknown");
            let pid = session.operation.as_ref().map(|lock| lock.pid).unwrap_or(0);
            warnings.push(format!(
                "Session {} has an active '{operation}' operation (pid {pid}); the device may be busy.",
                session.id
            ));
        }
        if session.lease_expired() {
            warnings.push(format!(
                "Session {} has an expired advisory lease ({}); work continues, but renew it with `session ensure --ttl`.",
                session.id,
                session.lease_expires_at.as_deref().unwrap_or("unknown")
            ));
        }
    }
    if let Some(owner) = lock_owner
        && Some(owner.id.as_str()) != session.map(|session| session.id.as_str())
    {
        let serial = serial.unwrap_or("device");
        let owner_label = if owner.owner.trim().is_empty() {
            "no owner recorded".to_string()
        } else {
            format!("owner '{}'", owner.owner)
        };
        let expiry = match owner.lease_expires_at.as_deref() {
            Some(value) if owner.lease_expired() => format!("until {value} (lease expired)"),
            Some(value) => format!("until {value}"),
            None => "with no advisory lease expiry recorded".to_string(),
        };
        warnings.push(format!(
            "Device {serial} is in use by session {} ({owner_label}, state {:?}) {expiry}; do not steal it.",
            owner.id, owner.state
        ));
    }
    warnings
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

#[cfg(test)]
mod tests {
    use androidforge_core::session::{OperationLock, SessionKind};
    use chrono::{Duration, Utc};

    use super::*;

    fn synthetic(id: &str, owner: &str) -> Session {
        let mut session = Session::new(
            owner,
            SessionKind::Attached,
            Some("emulator-5554".to_string()),
            None,
            None,
            true,
        );
        session.id = id.to_string();
        session.state = SessionState::Ready;
        session
    }

    fn warning_contains(warnings: &[String], needle: &str) -> bool {
        warnings.iter().any(|warning| warning.contains(needle))
    }

    #[test]
    fn healthy_session_is_quiet_and_reports_current_signals() {
        let session = synthetic("ses-healthy", "alice");
        let advisory = session_advisory(&session);
        assert_eq!(advisory["stale"], false);
        assert_eq!(advisory["operation_active"], false);
        assert_eq!(advisory["lease_expired"], false);
        assert!(advisory["lease_expires_at"].is_null());
        assert!(
            advisory["heartbeat_age_seconds"]
                .as_i64()
                .is_some_and(|age| (0..=5).contains(&age))
        );
        assert!(advisory_warnings(Some(&session), None, None).is_empty());
    }

    #[test]
    fn stale_heartbeat_is_flagged_and_warned() {
        let mut session = synthetic("ses-stale", "alice");
        session.heartbeat_at = (Utc::now() - Duration::hours(3)).to_rfc3339();
        let advisory = session_advisory(&session);
        assert_eq!(advisory["stale"], true);
        assert!(
            advisory["heartbeat_age_seconds"]
                .as_i64()
                .is_some_and(|age| age > 2 * 60 * 60)
        );
        let warnings = advisory_warnings(Some(&session), None, None);
        assert_eq!(warnings.len(), 1);
        assert!(warning_contains(&warnings, "ses-stale"));
        assert!(warning_contains(&warnings, "heartbeat"));
    }

    #[test]
    fn active_operation_is_flagged_as_busy() {
        let mut session = synthetic("ses-busy", "alice");
        session.operation = Some(OperationLock {
            pid: std::process::id(),
            operation: "ui.tap".to_string(),
            started_at: now_rfc3339(),
        });
        let advisory = session_advisory(&session);
        assert_eq!(advisory["operation_active"], true);
        assert_eq!(advisory["operation"], "ui.tap");
        let warnings = advisory_warnings(Some(&session), None, None);
        assert!(warning_contains(&warnings, "active 'ui.tap' operation"));
        assert!(warning_contains(&warnings, "busy"));
    }

    #[test]
    fn expired_lease_is_flagged_and_warned() {
        let mut session = synthetic("ses-expired", "alice");
        session.lease_expires_at = Some(
            (Utc::now() - Duration::minutes(5)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        );
        let advisory = session_advisory(&session);
        assert_eq!(advisory["lease_expired"], true);
        assert_eq!(
            advisory["lease_expires_at"],
            json!(session.lease_expires_at)
        );
        let warnings = advisory_warnings(Some(&session), None, None);
        assert_eq!(warnings.len(), 1);
        assert!(warning_contains(&warnings, "expired advisory lease"));
    }

    #[test]
    fn foreign_lock_without_owner_reports_device_in_use() {
        let mut owner = synthetic("ses-other", "");
        owner.lease_expires_at = Some(
            (Utc::now() + Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        );
        let warnings = advisory_warnings(None, Some(&owner), Some("emulator-5554"));
        assert_eq!(warnings.len(), 1);
        assert!(warning_contains(
            &warnings,
            "Device emulator-5554 is in use by session ses-other"
        ));
        assert!(warning_contains(&warnings, "no owner recorded"));
        assert!(warning_contains(&warnings, "until"));
    }

    #[test]
    fn own_device_lock_is_not_reported_as_in_use() {
        let session = synthetic("ses-self", "alice");
        let warnings = advisory_warnings(Some(&session), Some(&session), session.serial.as_deref());
        assert!(warnings.is_empty());
    }
}
