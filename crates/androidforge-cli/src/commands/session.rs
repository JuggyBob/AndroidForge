use std::time::Duration;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::Device;
use androidforge_core::output::CommandSuccess;
use androidforge_core::session::{
    Session, SessionKind, SessionState, gc_sessions, list_sessions, owner_of_serial,
    set_current_session,
};
use androidforge_core::util::now_rfc3339;
use androidforge_emulator::{DevicePrepareOptions, DevicePreparer};
use serde_json::Value;

use crate::cli::{
    AvdCommand, EmulatorCommand, SessionAttachArgs, SessionCommand, SessionGcArgs,
    SessionReleaseArgs, SessionRestartArgs, SessionStartArgs, SessionStopArgs, SnapshotCommand,
};
use crate::commands::common::{default_owner, success};
use crate::context::CliContext;

pub async fn session(ctx: &CliContext, command: &SessionCommand) -> Result<CommandSuccess> {
    match command {
        SessionCommand::Start(args) => start(ctx, args).await,
        SessionCommand::Attach(args) | SessionCommand::Claim(args) => attach(ctx, args).await,
        SessionCommand::List { active } => list(ctx, *active),
        SessionCommand::Inspect => inspect(ctx, false).await,
        SessionCommand::Status => inspect(ctx, true).await,
        SessionCommand::Release(args) => release(ctx, args, false).await,
        SessionCommand::Stop(args) => stop(ctx, args).await,
        SessionCommand::Use { session_id } => {
            let session = Session::load(&ctx.paths, session_id)?;
            if matches!(
                session.state,
                SessionState::Released | SessionState::Failed | SessionState::Abandoned
            ) {
                return Err(ForgeError::new(
                    ErrorCode::SessionExpired,
                    format!("session {session_id} is {:?}", session.state),
                ));
            }
            set_current_session(&ctx.paths, session_id)?;
            Ok(success(
                "session.use",
                serde_json::json!({"session_id": session_id, "pointer": ctx.paths.session_pointer_path()}),
                format!("Current session set to {session_id}\n"),
            ))
        }
        SessionCommand::Current => {
            let current = androidforge_core::session::current_session(&ctx.paths)?;
            Ok(success(
                "session.current",
                serde_json::json!({"session_id": current, "pointer": ctx.paths.session_pointer_path()}),
                current
                    .map(|session| format!("{session}\n"))
                    .unwrap_or_else(|| "(no current session)\n".to_string()),
            ))
        }
        SessionCommand::Gc(args) => gc(ctx, args).await,
        SessionCommand::Restart(args) => restart(ctx, args).await,
    }
}

pub async fn avd(ctx: &CliContext, command: &AvdCommand) -> Result<CommandSuccess> {
    match command {
        AvdCommand::List => avd_list(ctx).await,
        AvdCommand::Clone { source, name } => Err(ForgeError::unsupported(
            "AVD cloning and named-profile isolation are not implemented",
        )
        .with_context_string("source_avd", source.clone())
        .with_context_string("requested_name", name.clone().unwrap_or_default())
        .with_suggestion(
            "Use `session start --avd <AVD>` for exclusive AndroidForge ownership of one AVD.",
        )
        .with_suggestion(
            "Use snapshots for reversible state on an emulator version that supports them.",
        )
        .with_suggestion(
            "Create another AVD with the Android SDK tools if you need two independent devices.",
        )),
    }
}

pub async fn avd_list(ctx: &CliContext) -> Result<CommandSuccess> {
    let manager = ctx.emulator_manager()?;
    let avds = manager.list_avds().await?;
    let result = serde_json::json!({"avds": avds});
    let mut human = String::new();
    if avds.is_empty() {
        human.push_str("No AVDs found.\n");
    } else {
        for avd in &avds {
            human.push_str(&format!(
                "{}{} {}{}\n",
                avd.name,
                avd.target
                    .as_ref()
                    .map(|target| format!(" target={target}"))
                    .unwrap_or_default(),
                avd.abi.as_deref().unwrap_or(""),
                avd.device
                    .as_ref()
                    .map(|device| format!(" device={device}"))
                    .unwrap_or_default()
            ));
        }
    }
    Ok(success("avd.list", result, human))
}

async fn start(ctx: &CliContext, args: &SessionStartArgs) -> Result<CommandSuccess> {
    if args.headless && args.windowed {
        return Err(ForgeError::invalid_argument(
            "--headless and --windowed cannot both be used",
        ));
    }
    let headless = !args.windowed;
    let owner = args.owner.clone().unwrap_or_else(default_owner);
    if ctx.dry_run {
        return Ok(success(
            "session.start",
            serde_json::json!({
                "dry_run": true,
                "avd": args.avd,
                "owner": owner,
                "port": args.port,
                "headless": headless,
                "reuse": args.reuse,
                "prepare": args.prepare,
            }),
            format!(
                "Would start session for AVD {} on port {} (headless={}, owner={})\n",
                args.avd,
                args.port
                    .map(|port| port.to_string())
                    .unwrap_or_else(|| "auto".to_string()),
                headless,
                owner
            ),
        ));
    }
    let mut manager = ctx.emulator_manager()?;
    if let Some(timeout) = &args.timeout {
        manager.boot_timeout = ctx.parse_timeout(Some(timeout), manager.boot_timeout.as_secs())?;
    }
    let adb = ctx.adb()?;
    if args.reuse {
        if let Some((serial, port)) = find_running_avd(ctx, &args.avd).await? {
            let mut session = Session::new(
                owner,
                SessionKind::Attached,
                Some(serial.clone()),
                Some(args.avd.clone()),
                Some(port),
                true,
            );
            session.create(&ctx.paths)?;
            if let Err(error) = session.acquire_device(&ctx.paths, &serial) {
                let _ = session.mark_failed(&ctx.paths, error.message());
                return Err(error.session(session.id.clone()));
            }
            if let Err(error) = adb
                .wait_for_device(
                    Some(&serial),
                    true,
                    ctx.parse_timeout(args.timeout.as_deref(), manager.boot_timeout.as_secs())?,
                )
                .await
            {
                let _ = session.mark_failed(&ctx.paths, error.message());
                return Err(error.session(session.id.clone()));
            }
            let device = adb.inspect(&serial).await?;
            let warnings =
                prepare_session(ctx, &mut session, &device, args.prepare, args.no_wake).await?;
            session.mark_ready(&ctx.paths)?;
            return session_success(
                "session.start",
                &session,
                &device,
                warnings,
                Some("attached to a warm emulator"),
            );
        }
    }

    let reservation = androidforge_emulator::allocate_port(args.port)?;
    let port = reservation.console_port;
    let serial = reservation.serial();
    let mut session = Session::new(
        owner,
        SessionKind::Emulator,
        Some(serial.clone()),
        Some(args.avd.clone()),
        Some(port),
        headless,
    );
    session.create(&ctx.paths)?;
    if let Err(error) = session.acquire_device(&ctx.paths, &serial) {
        let _ = session.mark_failed(&ctx.paths, error.message());
        return Err(error.session(session.id.clone()));
    }
    reservation.release();
    let log_dir = ctx.paths.sessions_dir().join(&session.id).join("logs");
    let started = manager
        .start(
            &args.avd,
            Some(port),
            headless,
            args.memory,
            args.cores,
            args.cold_boot,
            &[],
            &log_dir,
        )
        .await;
    let started = match started {
        Ok(started) => started,
        Err(error) => {
            let _ = session.mark_failed(&ctx.paths, error.message());
            return Err(error.session(session.id.clone()));
        }
    };
    session.emulator_pid = Some(started.pid);
    session.emulator_args = started.args.clone();
    session.mark_ready(&ctx.paths)?;
    let device = started.device.clone();
    let warnings = prepare_session(ctx, &mut session, &device, args.prepare, args.no_wake).await?;
    let mut success = session_success("session.start", &session, &device, warnings, None)?;
    if let Some(object) = success.result.as_object_mut() {
        let mut emulator = serde_json::to_value(&started).unwrap_or(Value::Null);
        if let Some(emulator_object) = emulator.as_object_mut() {
            emulator_object.insert(
                "device".to_string(),
                serde_json::to_value(device.compact()).unwrap_or(Value::Null),
            );
        }
        object.insert("emulator".to_string(), emulator);
    }
    if let Some(human) = success.human.strip_suffix('\n') {
        success.human = format!("{human}\n");
    }
    Ok(success)
}

async fn attach(ctx: &CliContext, args: &SessionAttachArgs) -> Result<CommandSuccess> {
    let serial = ctx
        .serial_arg
        .clone()
        .or_else(|| std::env::var("ANDROIDFORGE_SERIAL").ok())
        .ok_or_else(|| {
            ForgeError::invalid_argument("session attach requires --serial <SERIAL>")
                .with_suggestion("Run `androidforge devices list` to inspect connected devices.")
        })?;
    let owner = args.owner.clone().unwrap_or_else(default_owner);
    if ctx.dry_run {
        return Ok(success(
            "session.attach",
            serde_json::json!({"dry_run": true, "serial": serial, "owner": owner}),
            format!("Would attach session to {serial} as {owner}\n"),
        ));
    }
    let adb = ctx.adb()?;
    let manager = ctx.emulator_manager()?;
    let device = adb
        .wait_for_device(
            Some(&serial),
            true,
            ctx.parse_timeout(None, manager.boot_timeout.as_secs())?,
        )
        .await?;
    let port = parse_emulator_port(&serial);
    let avd = args.avd.clone();
    let mut session = Session::new(
        owner,
        SessionKind::Attached,
        Some(serial.clone()),
        avd,
        port,
        true,
    );
    session.create(&ctx.paths)?;
    if let Err(error) = session.acquire_device(&ctx.paths, &serial) {
        let _ = session.mark_failed(&ctx.paths, error.message());
        return Err(error.session(session.id.clone()));
    }
    let warnings = prepare_session(ctx, &mut session, &device, args.prepare, args.no_wake).await?;
    session.mark_ready(&ctx.paths)?;
    session_success("session.attach", &session, &device, warnings, None)
}

fn list(ctx: &CliContext, active_only: bool) -> Result<CommandSuccess> {
    let mut sessions = list_sessions(&ctx.paths)?;
    if active_only {
        sessions.retain(|session| {
            !matches!(
                session.state,
                SessionState::Released | SessionState::Failed | SessionState::Abandoned
            )
        });
    }
    let result = serde_json::json!({"active_only": active_only, "sessions": sessions});
    let mut human = String::new();
    if sessions.is_empty() {
        human.push_str("No sessions.\n");
    } else {
        for session in &sessions {
            human.push_str(&format!(
                "{} owner={} state={:?} serial={} avd={} stale={} operation={}\n",
                session.id,
                session.owner,
                session.state,
                session.serial.as_deref().unwrap_or("-"),
                session.avd.as_deref().unwrap_or("-"),
                session.stale,
                session.operation.as_deref().unwrap_or("-"),
            ));
        }
    }
    Ok(success("session.list", result, human))
}

async fn inspect(ctx: &CliContext, detailed: bool) -> Result<CommandSuccess> {
    let session = ctx.require_session()?;
    let adb = ctx.adb()?;
    let mut device = None;
    if let Ok(serial) = session.serial() {
        device = adb.inspect(serial).await.ok();
    }
    let emulator = if detailed {
        let manager = ctx.emulator_manager()?;
        Some(
            manager
                .status(
                    session.serial.as_deref().unwrap_or(""),
                    session.emulator_pid,
                )
                .await
                .ok(),
        )
    } else {
        None
    };
    let mut result = serde_json::to_value(&session)?;
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "device".to_string(),
            device
                .as_ref()
                .map(|device| serde_json::to_value(device.compact()).unwrap_or(Value::Null))
                .unwrap_or(Value::Null),
        );
        object.insert(
            "emulator".to_string(),
            serde_json::to_value(&emulator).unwrap_or(Value::Null),
        );
        object.insert(
            "heartbeat_age_seconds".to_string(),
            Value::from(session.age_seconds()),
        );
        object.insert("stale".to_string(), Value::from(session.is_stale()));
        object.insert(
            "operation_active".to_string(),
            Value::from(session.operation_active()),
        );
    }
    let human = format!(
        "session {}\nowner: {}\nstate: {:?}\nserial: {}\navd: {}\nport: {}\nheadless: {}\nheartbeat age: {}s\n",
        session.id,
        session.owner,
        session.state,
        session.serial.as_deref().unwrap_or("-"),
        session.avd.as_deref().unwrap_or("-"),
        session
            .port
            .map(|port| port.to_string())
            .unwrap_or_else(|| "-".to_string()),
        session.headless,
        session.age_seconds()
    );
    Ok(success(
        if detailed {
            "session.status"
        } else {
            "session.inspect"
        },
        result,
        human,
    ))
}

fn caller_owner(owner: Option<&str>) -> String {
    owner
        .map(str::to_string)
        .or_else(|| {
            std::env::var("ANDROIDFORGE_OWNER")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(default_owner)
}

fn ensure_session_owner(session: &Session, caller: &str, force: bool) -> Result<()> {
    if force || session.owner == caller {
        return Ok(());
    }
    Err(ForgeError::new(
        ErrorCode::SessionOwnershipMismatch,
        format!(
            "session {} is owned by '{}', not '{}'",
            session.id, session.owner, caller
        ),
    )
    .with_context_string("session_owner", session.owner.clone())
    .with_context_string("caller_owner", caller.to_string())
    .with_suggestion("Pass --owner <OWNER> matching the session owner.")
    .with_suggestion("Or set ANDROIDFORGE_OWNER to your agent name.")
    .with_suggestion("Use --force only when you intentionally take over another agent's session.")
    .session(session.id.clone()))
}

async fn release(
    ctx: &CliContext,
    args: &SessionReleaseArgs,
    force_stop: bool,
) -> Result<CommandSuccess> {
    let mut session = ctx.require_session()?;
    ensure_session_owner(&session, &caller_owner(args.owner.as_deref()), args.force)?;
    if (args.stop || force_stop) && session.kind == SessionKind::Attached && !args.stop_attached {
        return Err(ForgeError::new(
            ErrorCode::SessionOwnershipMismatch,
            "this session is attached to an existing emulator; refusing to stop it",
        )
        .with_suggestion("Use --stop-attached only when you explicitly intend to stop the externally started emulator.")
        .with_suggestion("Otherwise use `androidforge session release` without --stop.")
        .session(session.id.clone()));
    }
    let warnings = if args.keep_prepared {
        Vec::new()
    } else {
        restore_prepared(ctx, &mut session).await
    };
    if args.stop || force_stop {
        stop_emulator(ctx, &mut session).await?;
    }
    session.release(&ctx.paths)?;
    clear_pointer(ctx, &session.id)?;
    let result = serde_json::json!({
        "session_id": session.id,
        "released": true,
        "stopped": args.stop || force_stop,
        "warnings": warnings,
    });
    Ok(success(
        "session.release",
        result,
        format!(
            "Released session {}{}\n",
            session.id,
            if args.stop || force_stop {
                " and stopped its emulator"
            } else {
                ""
            }
        ),
    ))
}

async fn stop(ctx: &CliContext, args: &SessionStopArgs) -> Result<CommandSuccess> {
    let mut session = match ctx.load_session(true) {
        Ok(Some(session)) => session,
        Ok(None) => {
            return Err(
                ForgeError::new(ErrorCode::SessionNotFound, "no session was specified")
                    .with_suggestion("Pass --session <ID> or run `androidforge session start`."),
            );
        }
        Err(error) if error.code() == ErrorCode::SessionExpired => {
            // `session release` intentionally keeps an owned emulator running.
            // Allow the tombstone to stop that emulator if nobody else owns it.
            let id = ctx.session_id()?.ok_or_else(|| {
                ForgeError::new(ErrorCode::SessionNotFound, "no session was specified")
            })?;
            let session = Session::load(&ctx.paths, &id)?;
            if session.kind != SessionKind::Emulator || session.state != SessionState::Released {
                return Err(error);
            }
            if let Some(serial) = &session.serial {
                if let Some(owner) = owner_of_serial(&ctx.paths, serial)? {
                    if owner.id != session.id {
                        return Err(ForgeError::new(
                            ErrorCode::DeviceBusy,
                            format!("{serial} is now owned by session {}", owner.id),
                        )
                        .with_suggestion(
                            "Stop it through the current owner instead of the released tombstone.",
                        )
                        .device(serial.clone())
                        .session(owner.id));
                    }
                }
            }
            session
        }
        Err(error) => return Err(error),
    };
    ensure_session_owner(&session, &caller_owner(args.owner.as_deref()), args.force)?;
    if session.kind == SessionKind::Attached && !args.attached {
        return Err(ForgeError::new(
            ErrorCode::SessionOwnershipMismatch,
            "this session is attached to an existing emulator; refusing to stop it",
        )
        .with_suggestion(
            "Use `androidforge session stop --attached` if you explicitly intend to stop it.",
        )
        .session(session.id.clone()));
    }
    let warnings = if args.keep_prepared {
        Vec::new()
    } else {
        restore_prepared(ctx, &mut session).await
    };
    stop_emulator(ctx, &mut session).await?;
    session.release(&ctx.paths)?;
    clear_pointer(ctx, &session.id)?;
    Ok(success(
        "session.stop",
        serde_json::json!({"session_id": session.id, "stopped": true, "warnings": warnings}),
        format!("Stopped emulator and released session {}\n", session.id),
    ))
}

async fn stop_emulator(ctx: &CliContext, session: &mut Session) -> Result<()> {
    let serial = session.serial()?.to_string();
    let manager = ctx.emulator_manager()?;
    session.mark_stopping(&ctx.paths)?;
    let result = manager.stop(&serial, session.emulator_pid).await?;
    if !result.process_exited && !result.adb_disconnected {
        return Err(ForgeError::new(
            ErrorCode::CommandFailed,
            format!("emulator {serial} did not stop cleanly"),
        )
        .with_context(
            "stop_result",
            serde_json::to_value(&result).unwrap_or(Value::Null),
        )
        .with_suggestion("Inspect the emulator process before retrying.")
        .session(session.id.clone())
        .device(serial));
    }
    Ok(())
}

async fn restore_prepared(ctx: &CliContext, session: &mut Session) -> Vec<String> {
    let serial = match session.serial() {
        Ok(serial) => serial.to_string(),
        Err(_) => return Vec::new(),
    };
    let preparer = DevicePreparer::new(match ctx.adb() {
        Ok(adb) => adb,
        Err(error) => {
            return vec![format!(
                "could not restore device preparation: {}",
                error.message()
            )];
        }
    });
    match preparer.restore(&serial, session, &ctx.paths).await {
        Ok(result) => result.warnings,
        Err(error) => vec![format!(
            "could not restore device preparation: {}",
            error.message()
        )],
    }
}

async fn gc(ctx: &CliContext, args: &SessionGcArgs) -> Result<CommandSuccess> {
    let before = list_sessions(&ctx.paths)?;
    let report = gc_sessions(&ctx.paths, args.reclaim, ctx.dry_run)?;
    let mut warnings = Vec::new();
    if args.stop && !ctx.dry_run {
        for id in &report.reclaimed_sessions {
            let Some(entry) = before.iter().find(|entry| &entry.id == id) else {
                continue;
            };
            if let Some(pid) = entry.emulator_pid {
                if androidforge_process::process_exists(pid) {
                    if let Ok(manager) = ctx.emulator_manager() {
                        if let Some(serial) = &entry.serial {
                            match manager.stop(serial, Some(pid)).await {
                                Ok(result) => {
                                    if !result.process_exited {
                                        warnings.push(format!(
                                            "abandoned emulator {} did not stop",
                                            serial
                                        ));
                                    }
                                }
                                Err(error) => warnings.push(format!(
                                    "could not stop abandoned emulator: {}",
                                    error.message()
                                )),
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(success(
        "session.gc",
        serde_json::json!({"report": report, "warnings": warnings}),
        serde_json::to_string_pretty(&report).unwrap_or_default() + "\n",
    ))
}

async fn restart(ctx: &CliContext, args: &SessionRestartArgs) -> Result<CommandSuccess> {
    let mut session = ctx.require_session()?;
    ensure_session_owner(&session, &caller_owner(args.owner.as_deref()), args.force)?;
    if session.kind != SessionKind::Emulator {
        return Err(ForgeError::new(
            ErrorCode::UnsupportedOperation,
            "restart is only supported for AndroidForge-launched emulators",
        )
        .with_suggestion("Release the attached session and start a new emulator session.")
        .session(session.id.clone()));
    }
    let avd = session.avd.clone().ok_or_else(|| {
        ForgeError::new(ErrorCode::SessionExpired, "session has no AVD name")
            .session(session.id.clone())
    })?;
    let port = session.port;
    let headless = session.headless;
    let manager = ctx.emulator_manager()?;
    let serial = session.serial()?.to_string();
    let stop = manager.stop(&serial, session.emulator_pid).await?;
    if !stop.process_exited && !stop.adb_disconnected {
        return Err(ForgeError::new(
            ErrorCode::CommandFailed,
            format!("could not stop emulator {serial} before restart"),
        )
        .session(session.id.clone()));
    }
    let log_dir = ctx.paths.sessions_dir().join(&session.id).join("logs");
    let started = manager
        .start(
            &avd,
            port,
            headless,
            None,
            None,
            args.cold_boot,
            &[],
            &log_dir,
        )
        .await?;
    session.emulator_pid = Some(started.pid);
    session.emulator_args = started.args.clone();
    session.mark_ready(&ctx.paths)?;
    let device = started.device;
    let warnings = prepare_session(ctx, &mut session, &device, false, false).await?;
    session_success(
        "session.restart",
        &session,
        &device,
        warnings,
        Some("restarted"),
    )
}

fn session_success(
    command: &str,
    session: &Session,
    device: &Device,
    warnings: Vec<String>,
    note: Option<&str>,
) -> Result<CommandSuccess> {
    let result = serde_json::json!({
        "session": session,
        "device": device.compact(),
        "warnings": warnings,
        "note": note,
    });
    let human = format!(
        "Session {}{}\nowner: {}\nserial: {}\ndevice: {}\n",
        session.id,
        note.map(|note| format!(" ({note})")).unwrap_or_default(),
        session.owner,
        device.serial,
        device.short_summary()
    );
    Ok(success(command, result, human)
        .with_session(session.id.clone())
        .with_device(device.serial.clone())
        .with_artifacts(Vec::new()))
}

async fn prepare_session(
    ctx: &CliContext,
    session: &mut Session,
    device: &Device,
    prepare: bool,
    no_wake: bool,
) -> Result<Vec<String>> {
    let adb = ctx.adb()?;
    let preparer = DevicePreparer::new(adb);
    let options = if prepare {
        DevicePrepareOptions {
            animations: true,
            keep_awake: true,
            wake: !no_wake,
            locale: None,
            timezone: None,
            font_scale: None,
            dark_mode: None,
            orientation: None,
            density: None,
        }
    } else {
        DevicePrepareOptions {
            animations: false,
            keep_awake: false,
            wake: !no_wake,
            ..DevicePrepareOptions::default()
        }
    };
    let result = preparer
        .prepare(&device.serial, &options, session, &ctx.paths)
        .await?;
    let mut warnings = result.warnings;
    if let Some(wake) = &result.wake {
        if wake.locked == Some(true) {
            warnings.push("device still reports a locked keyguard after wake/unlock".to_string());
        }
        warnings.extend(wake.notes.clone());
    }
    session.add_note(&ctx.paths, format!("prepared at {}", now_rfc3339()))?;
    Ok(warnings)
}

async fn find_running_avd(ctx: &CliContext, avd: &str) -> Result<Option<(String, u16)>> {
    let adb = ctx.adb()?;
    for device in adb.devices_raw().await? {
        if !device.state.is_operational() {
            continue;
        }
        let name = adb
            .emu(
                &device.serial,
                &["avd".to_string(), "name".to_string()],
                Some(Duration::from_secs(8)),
            )
            .await
            .ok()
            .and_then(|output| {
                output
                    .stdout_text()
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty() && *line != "OK" && *line != "KO")
                    .map(str::to_string)
            });
        if name.as_deref() == Some(avd) {
            if let Some(port) = parse_emulator_port(&device.serial) {
                return Ok(Some((device.serial, port)));
            }
        }
    }
    Ok(None)
}

fn parse_emulator_port(serial: &str) -> Option<u16> {
    serial.strip_prefix("emulator-")?.parse::<u16>().ok()
}

fn clear_pointer(ctx: &CliContext, session_id: &str) -> Result<()> {
    if androidforge_core::session::current_session(&ctx.paths)?.as_deref() == Some(session_id) {
        let path = ctx.paths.session_pointer_path();
        if path.is_file() {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

pub async fn emulator(ctx: &CliContext, command: &EmulatorCommand) -> Result<CommandSuccess> {
    match command {
        EmulatorCommand::Start(args) => start(ctx, args).await,
        EmulatorCommand::Status => {
            let session = ctx.load_session(false)?;
            let serial = match &session {
                Some(session) => session.serial()?.to_string(),
                None => ctx.serial().await?,
            };
            let manager = ctx.emulator_manager()?;
            let status = manager
                .status(
                    &serial,
                    session.as_ref().and_then(|session| session.emulator_pid),
                )
                .await?;
            let result = serde_json::to_value(&status)?;
            Ok(success(
                "emulator.status",
                result,
                format!(
                    "{} avd={} pid={} process_alive={} boot_completed={:?}\n",
                    status.serial,
                    status.avd.as_deref().unwrap_or("-"),
                    status
                        .pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    status.process_alive,
                    status.boot_completed
                ),
            )
            .with_device(status.serial)
            .with_optional_session(session.as_ref().map(|session| session.id.clone())))
        }
        EmulatorCommand::Stop(args) => {
            stop(
                ctx,
                &SessionStopArgs {
                    attached: args.attached,
                    keep_prepared: false,
                    owner: args.owner.clone(),
                    force: args.force,
                },
            )
            .await
        }
    }
}

pub async fn snapshot(ctx: &CliContext, command: &SnapshotCommand) -> Result<CommandSuccess> {
    let mut session = ctx.require_session()?;
    let serial = session.serial()?.to_string();
    let manager = ctx.emulator_manager()?;
    match command {
        SnapshotCommand::List => {
            let snapshots = manager.snapshot_list(&serial).await?;
            Ok(success(
                "snapshot.list",
                serde_json::json!({"snapshots": snapshots}),
                snapshots
                    .iter()
                    .map(|snapshot| snapshot.name.clone())
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n",
            )
            .with_session(session.id)
            .with_device(serial))
        }
        SnapshotCommand::Save { name } => {
            if ctx.dry_run {
                return Ok(success(
                    "snapshot.save",
                    serde_json::json!({"dry_run": true, "name": name}),
                    format!("Would save snapshot {name}\n"),
                ));
            }
            let snapshot = manager.snapshot_save(&serial, name).await?;
            session.record_snapshot(&ctx.paths, "save", name, true, None)?;
            Ok(success(
                "snapshot.save",
                serde_json::to_value(&snapshot)?,
                format!("Saved snapshot {name}\n"),
            )
            .with_session(session.id)
            .with_device(serial))
        }
        SnapshotCommand::Load { name } => {
            if !ctx.yes {
                return Err(ForgeError::new(
                    ErrorCode::InvalidArgument,
                    "snapshot load overwrites emulator user data; pass --yes",
                )
                .with_suggestion("Save important state first or use a disposable AVD."));
            }
            if ctx.dry_run {
                return Ok(success(
                    "snapshot.load",
                    serde_json::json!({"dry_run": true, "name": name, "destructive": true}),
                    format!(
                        "Would restore snapshot {name}; current emulator user data would be replaced\n"
                    ),
                ));
            }
            let result = manager.snapshot_load(&serial, name, ctx.yes).await;
            match result {
                Ok(snapshot) => {
                    session.record_snapshot(&ctx.paths, "load", name, true, None)?;
                    Ok(success(
                        "snapshot.load",
                        serde_json::to_value(&snapshot)?,
                        format!("Restored snapshot {name}\n"),
                    )
                    .with_session(session.id)
                    .with_device(serial))
                }
                Err(error) => {
                    session.record_snapshot(
                        &ctx.paths,
                        "load",
                        name,
                        false,
                        Some(error.message().to_string()),
                    )?;
                    Err(error.session(session.id).device(serial))
                }
            }
        }
        SnapshotCommand::Delete { name } => {
            if ctx.dry_run {
                return Ok(success(
                    "snapshot.delete",
                    serde_json::json!({"dry_run": true, "name": name}),
                    format!("Would delete snapshot {name}\n"),
                ));
            }
            let snapshot = manager.snapshot_delete(&serial, name).await?;
            session.record_snapshot(&ctx.paths, "delete", name, true, None)?;
            Ok(success(
                "snapshot.delete",
                serde_json::to_value(&snapshot)?,
                format!("Deleted snapshot {name}\n"),
            )
            .with_session(session.id)
            .with_device(serial))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(owner: &str) -> Session {
        Session::new(
            owner.to_string(),
            SessionKind::Emulator,
            Some("emulator-5554".to_string()),
            Some("test_avd".to_string()),
            Some(5554),
            true,
        )
    }

    #[test]
    fn matching_owner_can_release_or_stop() {
        assert!(ensure_session_owner(&session("AgentA"), "AgentA", false).is_ok());
    }

    #[test]
    fn another_owner_is_refused_without_force() {
        let error = ensure_session_owner(&session("AgentA"), "AgentB", false).unwrap_err();
        assert_eq!(error.code(), ErrorCode::SessionOwnershipMismatch);
        assert_eq!(error.body().context["session_owner"], "AgentA");
        assert_eq!(error.body().context["caller_owner"], "AgentB");
    }

    #[test]
    fn force_allows_deliberate_takeover() {
        assert!(ensure_session_owner(&session("AgentA"), "AgentB", true).is_ok());
    }
}
