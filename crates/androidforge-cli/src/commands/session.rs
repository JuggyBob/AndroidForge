use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::Device;
use androidforge_core::output::CommandSuccess;
use androidforge_core::session::{
    Session, SessionKind, SessionState, gc_sessions, list_sessions, owner_of_serial,
    prune_sessions, set_current_session,
};
use androidforge_core::util::{ensure_dir, now_rfc3339, parse_duration, sha256_hex};
use androidforge_emulator::{DevicePrepareOptions, DevicePreparer};
use serde_json::Value;

use crate::cli::{
    AvdCommand, EmulatorCommand, SessionAttachArgs, SessionCommand, SessionEnsureArgs,
    SessionGcArgs, SessionPruneArgs, SessionReleaseArgs, SessionRestartArgs, SessionStartArgs,
    SessionStopArgs, SnapshotCommand,
};
use crate::commands::common::{default_owner, success};
use crate::context::CliContext;

pub async fn session(ctx: &CliContext, command: &SessionCommand) -> Result<CommandSuccess> {
    match command {
        SessionCommand::Start(args) => start(ctx, args).await,
        SessionCommand::Ensure(args) => ensure(ctx, args).await,
        SessionCommand::Attach(args) | SessionCommand::Claim(args) => attach(ctx, args).await,
        SessionCommand::List {
            active,
            mine,
            owner,
        } => list(ctx, *active, *mine, owner.as_deref()),
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
        SessionCommand::Prune(args) => prune(ctx, args).await,
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

async fn allocate_emulator_port(
    ctx: &CliContext,
    requested: Option<u16>,
) -> Result<androidforge_emulator::PortReservation> {
    let adb = ctx.adb()?;
    let mut occupied = adb
        .devices_raw()
        .await?
        .iter()
        .filter_map(|device| parse_emulator_port(&device.serial))
        .collect::<BTreeSet<_>>();
    // Sessions record their port before the emulator process is spawned. The
    // TCP reservation is released at that point, so another concurrent
    // `session ensure` must treat a live session's port as occupied even when
    // ADB does not list the emulator yet.
    for entry in list_sessions(&ctx.paths)? {
        if matches!(
            entry.state,
            SessionState::Released | SessionState::Failed | SessionState::Abandoned
        ) {
            continue;
        }
        if let Some(port) = entry.port {
            occupied.insert(port);
        }
    }
    if let Some(port) = requested {
        if occupied.contains(&port) {
            return Err(ForgeError::new(
                ErrorCode::DeviceBusy,
                format!("emulator console port {port} already belongs to a connected ADB emulator"),
            )
            .with_context("port", serde_json::json!(port))
            .with_suggestion(
                "Choose a different even --port, or omit --port for automatic allocation.",
            )
            .device(format!("emulator-{port}")));
        }
        return androidforge_emulator::allocate_port(Some(port));
    }
    for port in (5554u16..=5682u16).step_by(2) {
        if occupied.contains(&port) {
            continue;
        }
        if let Ok(reservation) = androidforge_emulator::allocate_port(Some(port)) {
            return Ok(reservation);
        }
    }
    let reservation = androidforge_emulator::allocate_port(None)?;
    if occupied.contains(&reservation.console_port) {
        return Err(ForgeError::new(
            ErrorCode::DeviceBusy,
            format!(
                "automatic port allocation chose emulator-{}, which is already connected over ADB",
                reservation.console_port
            ),
        )
        .with_suggestion("Stop an unused emulator or pass an explicit even --port."));
    }
    Ok(reservation)
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
                "snapshot_save": args.snapshot_save,
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
    if args.reuse
        && let Some((serial, port)) = find_running_avd(ctx, &args.avd).await?
    {
        let mut session = Session::new(
            owner,
            SessionKind::Attached,
            Some(serial.clone()),
            Some(args.avd.clone()),
            Some(port),
            true,
        );
        session.create(&ctx.paths)?;
        session.bind_project(&ctx.paths, &ctx.paths.project_root.to_string_lossy())?;
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

    // The port choice and the session record that makes it durable must be one
    // cross-process critical section. Otherwise two parallel matrix workers can
    // both probe 5554 as free before either session exists, and the second one
    // fails on the device lock with a colliding serial.
    let port_lock = PortWindowLock::acquire(&ctx.paths).await?;
    let reservation = allocate_emulator_port(ctx, args.port).await?;
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
    session.snapshot_save = args.snapshot_save;
    session.create(&ctx.paths)?;
    session.bind_project(&ctx.paths, &ctx.paths.project_root.to_string_lossy())?;
    if let Err(error) = session.acquire_device(&ctx.paths, &serial) {
        let _ = session.mark_failed(&ctx.paths, error.message());
        return Err(error.session(session.id.clone()));
    }
    reservation.release();
    drop(port_lock);
    let log_dir = ctx.paths.sessions_dir().join(&session.id).join("logs");
    let started = manager
        .start(
            &args.avd,
            Some(port),
            headless,
            args.memory,
            args.cores,
            args.cold_boot,
            args.snapshot_save,
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

fn success_session_id(success: &CommandSuccess) -> Result<String> {
    success
        .result
        .get("session")
        .and_then(|session| session.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            ForgeError::internal("session command completed without returning a session id")
        })
}

fn success_warnings(success: &CommandSuccess) -> Vec<String> {
    success
        .result
        .get("warnings")
        .and_then(Value::as_array)
        .map(|warnings| {
            warnings
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn device_operational(device: &Device) -> bool {
    device.state.is_operational() && device.boot_completed != Some(false)
}

fn ensure_response(
    command: &str,
    session: &Session,
    device: Option<&Device>,
    acquisition_mode: &str,
    readiness: &str,
    recovery: Vec<String>,
    warnings: Vec<String>,
) -> Result<CommandSuccess> {
    let lease_expired = session.lease_expired();
    let result = serde_json::json!({
        "session_id": session.id,
        "session": session,
        "owner": session.owner,
        "device_serial": session.serial,
        "avd": session.avd,
        "profile": session.profile,
        "kind": session.kind,
        "state": session.state,
        "acquisition_mode": acquisition_mode,
        "ready": readiness == "ready",
        "readiness": readiness,
        "lease_expires_at": session.lease_expires_at,
        "lease_ttl_seconds": session.lease_ttl_seconds,
        "lease_expired": lease_expired,
        "needs_reconciliation": lease_expired,
        "recovery": recovery,
        "warnings": warnings,
        "device": device.map(|device| device.compact()),
    });
    let human = format!(
        "Session {} ({acquisition_mode})\nowner: {}\nserial: {}\navd: {}\nreadiness: {}\nlease: {}{}\n",
        session.id,
        session.owner,
        session.serial.as_deref().unwrap_or("-"),
        session.avd.as_deref().unwrap_or("-"),
        readiness,
        session.lease_expires_at.as_deref().unwrap_or("none"),
        if lease_expired {
            " (expired; reconcile or renew with --ttl)"
        } else {
            ""
        }
    );
    let mut response = success(command, result, human);
    response.session_id = Some(session.id.clone());
    response.device_serial = session.serial.clone();
    Ok(response)
}

struct EnsureLock {
    path: PathBuf,
}

impl EnsureLock {
    async fn acquire(paths: &androidforge_core::paths::ForgePaths, identity: &str) -> Result<Self> {
        let dir = paths.state_dir.join("locks");
        ensure_dir(&dir)?;
        let digest = sha256_hex(identity.as_bytes());
        let path = dir.join(format!("ensure-{}.lock", &digest[..16]));
        let started = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let _ = writeln!(file, "{}", std::process::id());
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if ensure_lock_is_stale(&path) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() > Duration::from_secs(180) {
                        return Err(ForgeError::new(
                            ErrorCode::DeviceBusy,
                            "another `session ensure` for this owner, project and target is still running",
                        )
                        .with_context_string("lock", path.to_string_lossy())
                        .with_suggestion(
                            "Wait for the other ensure operation to finish, then retry.",
                        )
                        .with_suggestion(
                            "If it crashed, remove the stale lock file shown in the context.",
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Err(error) => {
                    return Err(ForgeError::internal(format!(
                        "could not create ensure lock {}: {error}",
                        path.display()
                    )));
                }
            }
        }
    }
}

impl Drop for EnsureLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Serializes emulator port probing with session creation across processes.
/// It is a region lock, not a boot lock: once the session records its port the
/// lock is released and the emulator boots in parallel with other targets.
struct PortWindowLock {
    path: PathBuf,
}

impl PortWindowLock {
    async fn acquire(paths: &androidforge_core::paths::ForgePaths) -> Result<Self> {
        let dir = paths.state_dir.join("locks");
        ensure_dir(&dir)?;
        let path = dir.join("emulator-port.lock");
        let started = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let _ = writeln!(file, "{}", std::process::id());
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if ensure_lock_is_stale(&path) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() > Duration::from_secs(60) {
                        return Err(ForgeError::new(
                            ErrorCode::DeviceBusy,
                            "another process is still allocating an emulator port",
                        )
                        .with_context_string("lock", path.to_string_lossy())
                        .with_suggestion(
                            "Wait for the other emulator start to finish, then retry.",
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(error) => {
                    return Err(ForgeError::internal(format!(
                        "could not create emulator port lock {}: {error}",
                        path.display()
                    )));
                }
            }
        }
    }
}

impl Drop for PortWindowLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn ensure_lock_is_stale(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let age = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .unwrap_or_default();
    let pid = std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| contents.trim().parse::<u32>().ok());
    match pid {
        Some(pid) => !androidforge_process::process_exists(pid) && age > Duration::from_secs(5),
        None => age > Duration::from_secs(300),
    }
}

async fn ensure_serial_not_owned(ctx: &CliContext, serial: &str, owner: &str) -> Result<()> {
    let Some(existing) = owner_of_serial(&ctx.paths, serial)? else {
        return Ok(());
    };
    if existing.owner != owner {
        let alternatives = alternative_serials(ctx).await.unwrap_or_default();
        return Err(ForgeError::new(
            ErrorCode::DeviceBusy,
            format!(
                "device {serial} is owned by session {} (owner '{}', state {:?})",
                existing.id, existing.owner, existing.state
            ),
        )
        .with_context_string("owner", existing.owner.clone())
        .with_context_string("session_id", existing.id.clone())
        .with_context("alternatives", serde_json::json!(alternatives))
        .with_suggestion(format!(
            "Do not steal this device. Use `--session {}` only if you are that owner.",
            existing.id
        ))
        .with_suggestion(
            "Choose an available serial from the `alternatives` context or another AVD.",
        )
        .session(existing.id.clone())
        .device(serial.to_string()));
    }
    Err(ForgeError::new(
        ErrorCode::DeviceBusy,
        format!(
            "device {serial} is already used by your session {} with different project, AVD or profile settings",
            existing.id
        ),
    )
    .with_context_string("session_id", existing.id.clone())
    .with_suggestion("Use the existing session as-is, release it explicitly, or choose another AVD.")
    .session(existing.id)
    .device(serial.to_string()))
}

async fn alternative_serials(ctx: &CliContext) -> Result<Vec<String>> {
    let adb = ctx.adb()?;
    let mut alternatives = Vec::new();
    for device in adb.devices_raw().await? {
        if !device.state.is_operational() {
            continue;
        }
        if owner_of_serial(&ctx.paths, &device.serial)?.is_none() {
            alternatives.push(device.serial);
        }
    }
    Ok(alternatives)
}

pub(crate) async fn ensure(ctx: &CliContext, args: &SessionEnsureArgs) -> Result<CommandSuccess> {
    if !args.wait {
        return ensure_once(ctx, args).await;
    }
    let timeout = parse_duration(&args.timeout).map_err(ForgeError::invalid_argument)?;
    if timeout.is_zero() {
        return Err(ForgeError::invalid_argument(
            "session ensure --timeout must be greater than zero",
        ));
    }
    let started = std::time::Instant::now();
    loop {
        match ensure_once(ctx, args).await {
            Ok(success) => return Ok(success),
            Err(error) if error.code() == ErrorCode::DeviceBusy => {
                if started.elapsed() >= timeout {
                    return Err(error.with_suggestion(format!(
                        "Waited {}s for a device. Inspect `session list` and choose another AVD or serial.",
                        timeout.as_secs()
                    )));
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

async fn ensure_once(ctx: &CliContext, args: &SessionEnsureArgs) -> Result<CommandSuccess> {
    if args.avd.is_none() && args.serial.is_none() {
        return Err(ForgeError::invalid_argument(
            "session ensure needs --avd <AVD> or --serial <SERIAL>",
        )
        .with_suggestion("Run `androidforge avd list` to see local AVDs.")
        .with_suggestion("Run `androidforge devices list` to see connected devices.")
        .with_suggestion("Use `androidforge context --json` to reuse a configured AVD."));
    }
    if args.avd.is_some() && args.serial.is_some() {
        return Err(
            ForgeError::invalid_argument("choose either --avd or --serial, not both")
                .with_suggestion(
                    "An AVD is launched if needed; a serial attaches to an existing device.",
                ),
        );
    }
    let owner = caller_owner(args.owner.as_deref());
    let ttl = args
        .ttl
        .as_deref()
        .map(parse_duration)
        .transpose()
        .map_err(ForgeError::invalid_argument)?;
    if let Some(ttl) = ttl
        && ttl.is_zero()
    {
        return Err(ForgeError::invalid_argument(
            "lease duration must be greater than zero",
        ));
    }
    let project_root = ctx.paths.project_root.to_string_lossy().into_owned();
    if ctx.dry_run {
        let mut would = vec!["reuse a compatible session owned by the same owner and project"];
        if args.serial.is_some() {
            would.push("attach to the requested serial only if it is unowned");
        } else {
            would.push("attach to a running emulator for the requested AVD if unowned");
            would.push("otherwise launch the requested AVD headless");
        }
        would.push("wait for device readiness and record the lease");
        return Ok(success(
            "session.ensure",
            serde_json::json!({
                "dry_run": true,
                "owner": owner,
                "project_root": project_root,
                "avd": args.avd,
                "serial": args.serial,
                "profile": args.profile,
                "port": args.port,
                "headless": true,
                "prepare": args.prepare,
                "wait": args.wait,
                "timeout": args.timeout,
                "lease_ttl_seconds": ttl.map(|ttl| ttl.as_secs()),
                "would": would,
                "destructive": false,
            }),
            format!(
                "Would ensure a session for {} as {owner} (no changes).\n",
                args.avd
                    .as_deref()
                    .or(args.serial.as_deref())
                    .unwrap_or("-")
            ),
        ));
    }

    let identity = format!(
        "{}|{}|{}|{}",
        owner,
        project_root,
        args.avd.as_deref().unwrap_or("-"),
        args.serial.as_deref().unwrap_or("-")
    );
    let _lock = EnsureLock::acquire(&ctx.paths, &identity).await?;
    let adb = ctx.adb()?;
    let mut legacy_warning = None;

    for entry in list_sessions(&ctx.paths)? {
        if matches!(
            entry.state,
            SessionState::Released | SessionState::Failed | SessionState::Abandoned
        ) {
            continue;
        }
        let mut session = match Session::load(&ctx.paths, &entry.id) {
            Ok(session) => session,
            Err(_) => continue,
        };
        if session.owner != owner {
            continue;
        }
        let legacy = session.project_root.is_none();
        if let Some(root) = &session.project_root
            && !crate::context::same_project_root(root, &project_root)
        {
            continue;
        }
        if let Some(serial) = &args.serial
            && session.serial.as_deref() != Some(serial.as_str())
        {
            continue;
        }
        if let Some(avd) = &args.avd
            && session.avd.as_deref() != Some(avd.as_str())
        {
            continue;
        }
        if let Some(profile) = &args.profile
            && session.profile.as_deref() != Some(profile.as_str())
        {
            continue;
        }
        if legacy {
            legacy_warning = Some(format!(
                "Session {} was created before project binding was recorded; it is now bound to this project.",
                session.id
            ));
        }
        let Some(serial) = session.serial.clone() else {
            continue;
        };
        let inspected = adb.inspect(&serial).await.ok();
        if let Some(device) = inspected.as_ref()
            && device_operational(device)
        {
            let mut recovery = Vec::new();
            let mut warnings = Vec::new();
            if let Some(warning) = legacy_warning.take() {
                warnings.push(warning);
            }
            if session.lease_expired() {
                match ttl {
                    Some(ttl) => {
                        session.set_lease(&ctx.paths, Some(ttl))?;
                        recovery.push("expired_lease_renewed".to_string());
                    }
                    None => warnings.push(
                        "The session lease has expired. It was not renewed because no --ttl was supplied; reconcile or pass --ttl to renew it."
                            .to_string(),
                    ),
                }
            } else if let Some(ttl) = ttl {
                session.set_lease(&ctx.paths, Some(ttl))?;
            }
            if session.project_root.is_none() {
                session.bind_project(&ctx.paths, &project_root)?;
            }
            if let Some(profile) = &args.profile
                && session.profile.as_deref() != Some(profile.as_str())
            {
                session.profile = Some(profile.clone());
                session.save(&ctx.paths)?;
            }
            if !matches!(session.state, SessionState::Ready) {
                session.mark_ready(&ctx.paths)?;
            }
            set_current_session(&ctx.paths, &session.id)?;
            return ensure_response(
                "session.ensure",
                &session,
                Some(device),
                "reused_session",
                "ready",
                recovery,
                warnings,
            );
        }

        let dead_emulator = session.kind == SessionKind::Emulator
            && session
                .emulator_pid
                .map(|pid| !androidforge_process::process_exists(pid))
                .unwrap_or(true);
        if dead_emulator {
            let _ = session.mark_failed(
                &ctx.paths,
                format!("emulator process for {serial} is no longer running"),
            );
            continue;
        }

        let mut warnings = Vec::new();
        if let Some(warning) = legacy_warning.take() {
            warnings.push(warning);
        }
        warnings.push(format!(
            "The compatible session {} exists but device {serial} is not currently operational; no device was stolen or restarted.",
            session.id
        ));
        set_current_session(&ctx.paths, &session.id)?;
        return ensure_response(
            "session.ensure",
            &session,
            None,
            "reused_session",
            "device_unavailable",
            Vec::new(),
            warnings,
        );
    }

    let (acquisition_mode, created, mut warnings) = if let Some(serial) = &args.serial {
        ensure_serial_not_owned(ctx, serial, &owner).await?;
        let attach_args = SessionAttachArgs {
            owner: Some(owner.clone()),
            avd: None,
            prepare: args.prepare,
            no_wake: false,
        };
        let created = attach_with_serial(ctx, &attach_args, Some(serial)).await?;
        let warnings = success_warnings(&created);
        ("attached_existing", created, warnings)
    } else {
        let avd = args.avd.as_deref().unwrap_or_default();
        if let Some((serial, running_port)) = find_running_avd(ctx, avd).await? {
            if let Some(requested_port) = args.port
                && requested_port != running_port
            {
                return Err(ForgeError::invalid_argument(format!(
                    "AVD {avd} is already running as {serial} on console port {running_port}, not requested port {requested_port}"
                ))
                .with_suggestion("Omit --port to attach to the running emulator.")
                .with_suggestion("Stop the running emulator first if you need a specific port.")
                .device(serial));
            }
            ensure_serial_not_owned(ctx, &serial, &owner).await?;
            let attach_args = SessionAttachArgs {
                owner: Some(owner.clone()),
                avd: Some(avd.to_string()),
                prepare: args.prepare,
                no_wake: false,
            };
            let created = attach_with_serial(ctx, &attach_args, Some(&serial)).await?;
            let warnings = success_warnings(&created);
            ("attached_existing", created, warnings)
        } else {
            let start_args = SessionStartArgs {
                avd: avd.to_string(),
                owner: Some(owner.clone()),
                port: args.port,
                headless: true,
                windowed: false,
                memory: None,
                cores: None,
                cold_boot: false,
                snapshot_save: args.snapshot_save,
                reuse: true,
                prepare: args.prepare,
                no_wake: false,
                timeout: None,
            };
            let created = start(ctx, &start_args).await?;
            let warnings = success_warnings(&created);
            ("launched_emulator", created, warnings)
        }
    };

    let id = success_session_id(&created)?;
    let mut session = Session::load(&ctx.paths, &id)?;
    session.bind_project(&ctx.paths, &project_root)?;
    if let Some(profile) = &args.profile {
        session.profile = Some(profile.clone());
        session.save(&ctx.paths)?;
    }
    if let Some(ttl) = ttl {
        session.set_lease(&ctx.paths, Some(ttl))?;
    }
    if let Some(warning) = legacy_warning {
        warnings.push(warning);
    }
    set_current_session(&ctx.paths, &session.id)?;
    let inspected = match session.serial.as_deref() {
        Some(serial) => adb.inspect(serial).await.ok(),
        None => None,
    };
    let readiness = if inspected.as_ref().is_some_and(device_operational) {
        "ready"
    } else {
        "device_unavailable"
    };
    ensure_response(
        "session.ensure",
        &session,
        inspected.as_ref(),
        acquisition_mode,
        readiness,
        Vec::new(),
        warnings,
    )
}

async fn attach(ctx: &CliContext, args: &SessionAttachArgs) -> Result<CommandSuccess> {
    attach_with_serial(ctx, args, None).await
}

async fn attach_with_serial(
    ctx: &CliContext,
    args: &SessionAttachArgs,
    serial_override: Option<&str>,
) -> Result<CommandSuccess> {
    let serial = serial_override
        .map(str::to_string)
        .or_else(|| ctx.serial_arg.clone())
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
    session.bind_project(&ctx.paths, &ctx.paths.project_root.to_string_lossy())?;
    if let Err(error) = session.acquire_device(&ctx.paths, &serial) {
        let _ = session.mark_failed(&ctx.paths, error.message());
        return Err(error.session(session.id.clone()));
    }
    let warnings = prepare_session(ctx, &mut session, &device, args.prepare, args.no_wake).await?;
    session.mark_ready(&ctx.paths)?;
    session_success("session.attach", &session, &device, warnings, None)
}

fn list(
    ctx: &CliContext,
    active_only: bool,
    mine: bool,
    owner: Option<&str>,
) -> Result<CommandSuccess> {
    let mut sessions = list_sessions(&ctx.paths)?;
    let caller = caller_owner(owner);
    if mine || owner.is_some() {
        sessions.retain(|session| session.owner == caller);
    }
    if active_only {
        sessions.retain(|session| {
            !matches!(
                session.state,
                SessionState::Released | SessionState::Failed | SessionState::Abandoned
            )
        });
    }
    let mut rendered = Vec::with_capacity(sessions.len());
    for session in &sessions {
        let classification = classify_session(session);
        let reason = session_reason(session, classification);
        let mut value = serde_json::to_value(session)?;
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "classification".to_string(),
                serde_json::Value::String(classification.to_string()),
            );
            object.insert("reason".to_string(), serde_json::Value::String(reason));
        }
        rendered.push(value);
    }
    let mut result = serde_json::json!({
        "active_only": active_only,
        "mine": mine || owner.is_some(),
        "sessions": rendered,
    });
    if mine || owner.is_some() {
        result["owner"] = serde_json::Value::String(caller.clone());
    }
    let mut human = String::new();
    if sessions.is_empty() {
        if mine || owner.is_some() {
            human.push_str(&format!("No sessions owned by {caller}.\n"));
        } else {
            human.push_str("No sessions.\n");
        }
    } else {
        for (session, rendered) in sessions.iter().zip(rendered.iter()) {
            human.push_str(&format!(
                "{} owner={} state={:?} class={} serial={} avd={} age={}s stale={} operation={} reason={}\n",
                session.id,
                session.owner,
                session.state,
                rendered["classification"].as_str().unwrap_or("-"),
                session.serial.as_deref().unwrap_or("-"),
                session.avd.as_deref().unwrap_or("-"),
                session.age_seconds,
                session.stale,
                session.operation.as_deref().unwrap_or("-"),
                rendered["reason"].as_str().unwrap_or("-"),
            ));
        }
    }
    Ok(success("session.list", result, human))
}

fn classify_session(session: &androidforge_core::session::SessionListEntry) -> &'static str {
    match session.state {
        SessionState::Released => "released",
        SessionState::Failed => "failed",
        SessionState::Abandoned => "abandoned",
        _ if session.stale => "stale",
        _ if session.operation_active => "busy",
        _ => "active",
    }
}

fn session_reason(
    session: &androidforge_core::session::SessionListEntry,
    classification: &str,
) -> String {
    match classification {
        "released" => "session was released; this is a tombstone".to_string(),
        "failed" => session
            .last_error
            .clone()
            .unwrap_or_else(|| "session failed without a recorded error".to_string()),
        "abandoned" => "session was reclaimed because its heartbeat stopped".to_string(),
        "stale" => format!(
            "heartbeat is {}s old; run `androidforge session gc --reclaim --dry-run` to preview cleanup",
            session.age_seconds
        ),
        "busy" => format!(
            "operation '{}' is active",
            session.operation.as_deref().unwrap_or("unknown")
        ),
        _ => "session heartbeat is current".to_string(),
    }
}

async fn inspect(ctx: &CliContext, detailed: bool) -> Result<CommandSuccess> {
    let mut session = ctx.require_session()?;
    let adb = ctx.adb()?;
    let mut device = None;
    if let Ok(serial) = session.serial() {
        device = adb.inspect(serial).await.ok();
    }
    let mut live_build_refreshed = false;
    let mut live_build_error = None;
    if let Ok(serial) = session.serial().map(str::to_string) {
        match refresh_live_build(ctx, &mut session, &serial).await {
            Ok(refreshed) => live_build_refreshed = refreshed,
            Err(error) => live_build_error = Some(error.message().to_string()),
        }
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
        object.insert(
            "live_build_refreshed".to_string(),
            Value::from(live_build_refreshed),
        );
        object.insert(
            "live_build_error".to_string(),
            serde_json::to_value(&live_build_error).unwrap_or(Value::Null),
        );
    }
    let mut human = format!(
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
    if let Some(build) = &session.build {
        human.push_str(&format!(
            "build: {}\ninstalled APK: {}\ninstalled SHA-256: {}\n",
            build.summary(),
            build.installed_apk_path.as_deref().unwrap_or("-"),
            build.installed_apk_sha256.as_deref().unwrap_or("-")
        ));
    }
    if let Some(error) = &live_build_error {
        human.push_str(&format!("live build refresh failed: {error}\n"));
    }
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

/// Refresh the cached build identity from the live device. The session file is
/// updated so later runs do not inherit an install record that an out-of-band
/// `adb install` has made stale.
async fn refresh_live_build(ctx: &CliContext, session: &mut Session, serial: &str) -> Result<bool> {
    let Some(package) = session
        .build
        .as_ref()
        .and_then(|build| build.package.clone())
        .or_else(|| ctx.config.package().map(str::to_string))
    else {
        return Ok(false);
    };
    let adb = ctx.adb()?;
    let inspector = ctx.inspector();
    let live = androidforge_app::inspect_installed(&adb, &inspector, serial, &package).await?;
    let mut build = session
        .build
        .clone()
        .unwrap_or_else(|| androidforge_core::model::BuildIdentity::for_package(package.clone()));
    build.package = Some(package.clone());
    match live {
        Some(live) => {
            build.version_name = live.version_name.clone();
            build.version_code = live.version_code;
            build.installed_apk_path = Some(live.base_remote_path.clone());
            build.installed_apk_sha256 = Some(live.base_apk.sha256.clone());
            build.cert_sha256 = live
                .base_apk
                .signature
                .cert_sha256
                .clone()
                .or(build.cert_sha256.take());
            build.debug_signed = live.base_apk.signature.debug_signed();
            build.verified = true;
            for warning in live.warnings {
                if !build.notes.contains(&warning) {
                    build.notes.push(warning);
                }
            }
        }
        None => {
            build.installed_apk_path = None;
            build.installed_apk_sha256 = None;
            build.verified = false;
            let note = format!("package {package} is not currently installed on {serial}");
            if !build.notes.contains(&note) {
                build.notes.push(note);
            }
        }
    }
    session.build = Some(build);
    session.save(&ctx.paths)?;
    Ok(true)
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
    .with_suggestion("Pass `--owner <OWNER>` to `session release`, `session stop` or `session restart`, or set ANDROIDFORGE_OWNER.")
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
            if let Some(serial) = &session.serial
                && let Some(owner) = owner_of_serial(&ctx.paths, serial)?
                && owner.id != session.id
            {
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
    let report = gc_sessions(&ctx.paths, args.reclaim, args.stop, ctx.dry_run)?;
    let mut warnings = Vec::new();
    if args.stop && !ctx.dry_run {
        for id in &report.reclaimed_sessions {
            let Some(entry) = before.iter().find(|entry| &entry.id == id) else {
                continue;
            };
            if let Some(pid) = entry.emulator_pid
                && androidforge_process::process_exists(pid)
                && let Ok(manager) = ctx.emulator_manager()
                && let Some(serial) = &entry.serial
            {
                match manager.stop(serial, Some(pid)).await {
                    Ok(result) => {
                        if !result.process_exited {
                            warnings.push(format!("abandoned emulator {} did not stop", serial));
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
    Ok(success(
        "session.gc",
        serde_json::json!({
            "report": report,
            "warnings": warnings,
            "would_reclaim": if ctx.dry_run { report.reclaimed_sessions.clone() } else { Vec::<String>::new() },
            "next_commands": if ctx.dry_run && !report.reclaimed_sessions.is_empty() {
                vec!["androidforge session gc --reclaim".to_string()]
            } else {
                Vec::<String>::new()
            },
        }),
        format!(
            "{}{}{}",
            serde_json::to_string_pretty(&report).unwrap_or_default(),
            if ctx.dry_run && !report.reclaimed_sessions.is_empty() {
                format!(
                    "\nDry run: {} session(s) would be marked abandoned. Re-run `androidforge session gc --reclaim` to apply it; add --stop to also stop abandoned emulators.\n",
                    report.reclaimed_sessions.len()
                )
            } else {
                String::new()
            },
            if !warnings.is_empty() {
                format!("\nwarnings:\n{}\n", warnings.join("\n"))
            } else {
                String::new()
            }
        ),
    ))
}

async fn prune(ctx: &CliContext, args: &SessionPruneArgs) -> Result<CommandSuccess> {
    if !ctx.yes && !ctx.dry_run {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            "pruning session tombstones is destructive; pass --yes",
        )
        .with_suggestion("Use --dry-run to preview which sessions would be removed."));
    }
    let older_than = args
        .older_than
        .as_deref()
        .map(parse_duration)
        .transpose()
        .map_err(ForgeError::invalid_argument)?
        .unwrap_or(Duration::from_secs(7 * 86_400));
    let keep_last = args.keep_last.unwrap_or(3);
    let report = prune_sessions(&ctx.paths, older_than, keep_last, ctx.dry_run)?;
    let human = format!(
        "{} {} of {} terminal session(s); kept {} total.\n",
        if ctx.dry_run { "Would prune" } else { "Pruned" },
        report.pruned.len(),
        report.terminal_sessions,
        report.kept
    );
    Ok(success(
        "session.prune",
        serde_json::to_value(&report)?,
        human,
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
            session.snapshot_save,
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
        if name.as_deref() == Some(avd)
            && let Some(port) = parse_emulator_port(&device.serial)
        {
            return Ok(Some((device.serial, port)));
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
    let device = ctx.resolve_device(Some(&session)).await?;
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
            ctx.ensure_physical_mutation_allowed(&device, "saving an emulator snapshot")?;
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
            if ctx.dry_run {
                return Ok(success(
                    "snapshot.load",
                    serde_json::json!({"dry_run": true, "name": name, "destructive": true, "requires_confirmation": true}),
                    format!(
                        "Would restore snapshot {name}; current emulator user data would be replaced and --yes is required\n"
                    ),
                ));
            }
            if !ctx.yes {
                return Err(ForgeError::new(
                    ErrorCode::InvalidArgument,
                    "snapshot load overwrites emulator user data; pass --yes",
                )
                .with_suggestion("Save important state first or use a disposable AVD."));
            }
            ctx.ensure_physical_mutation_allowed(&device, "loading an emulator snapshot")?;
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
            ctx.ensure_physical_mutation_allowed(&device, "deleting an emulator snapshot")?;
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

    #[test]
    fn session_release_accepts_an_explicit_owner_flag() {
        use clap::Parser;
        let cli = crate::cli::Cli::try_parse_from([
            "androidforge",
            "session",
            "release",
            "--session",
            "ses-test",
            "--owner",
            "AgentA",
            "--json",
        ])
        .expect("session release --owner must parse");
        match cli.command {
            crate::cli::Command::Session {
                command: SessionCommand::Release(args),
            } => assert_eq!(args.owner.as_deref(), Some("AgentA")),
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
