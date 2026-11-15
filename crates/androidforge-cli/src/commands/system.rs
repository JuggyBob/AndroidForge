use androidforge_app::{AppOps, Permissions};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::session::Session;
use androidforge_diagnostics::{
    Alarms, DeviceStateManager, Jobs, NotificationSelector, Notifications,
};
use chrono::DateTime;

use crate::cli::{
    AlarmsCommand, AppOpsCommand, BatteryCommand, DozeCommand, JobWaitArgs, JobsCommand,
    NetworkCommand, NotificationWaitArgs, NotificationsCommand, PermissionsCommand, StandbyCommand,
    TimeCommand, TimeSetArgs,
};
use crate::commands::common::success;
use crate::context::CliContext;

async fn target(ctx: &CliContext) -> Result<(Option<Session>, androidforge_core::model::Device)> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    Ok((session, device))
}

pub async fn notifications(
    ctx: &CliContext,
    command: &NotificationsCommand,
) -> Result<CommandSuccess> {
    let (session, device) = target(ctx).await?;
    let notifications = Notifications::new(ctx.adb()?);
    match command {
        NotificationsCommand::List { package } => {
            let list = notifications
                .list(&device.serial, package.as_deref())
                .await?;
            let human = if list.notifications.is_empty() {
                "No notifications parsed.\n".to_string()
            } else {
                list.notifications
                    .iter()
                    .map(|notification| {
                        format!(
                            "{} id={:?} tag={:?} channel={:?} title={:?} text={:?}\n",
                            notification.package.as_deref().unwrap_or("-"),
                            notification.id,
                            notification.tag,
                            notification.channel,
                            notification.title,
                            notification.text
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("")
            };
            Ok(
                success("notifications.list", serde_json::to_value(&list)?, human)
                    .with_optional_session(session.map(|session| session.id.clone()))
                    .with_device(device.serial.clone()),
            )
        }
        NotificationsCommand::Wait(args) | NotificationsCommand::Assert(args) => {
            wait_notifications(
                ctx,
                &notifications,
                &device,
                args,
                matches!(command, NotificationsCommand::Assert(_)),
            )
            .await
        }
        NotificationsCommand::Channels { package } => {
            let channels = notifications
                .channels(&device.serial, package.as_deref())
                .await?;
            Ok(success(
                "notifications.channels",
                serde_json::to_value(&channels)?,
                serde_json::to_string_pretty(&channels).unwrap_or_default() + "\n",
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        }
        NotificationsCommand::Tap(args) => {
            let selector = args.clone().into_selector();
            let ui = ctx.ui_manager()?;
            let result = notifications.tap(&device.serial, &selector, &ui).await?;
            Ok(success(
                "notifications.tap",
                serde_json::to_value(&result)?,
                format!(
                    "Tapped notification from {}\n",
                    result.package.as_deref().unwrap_or("-")
                ),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        }
        NotificationsCommand::Clear(args) => {
            let selector = args.clone().into_selector();
            let cleared = notifications.clear(&device.serial, &selector).await?;
            Ok(success(
                "notifications.clear",
                serde_json::json!({"cleared": cleared}),
                format!("Cleared {} notification(s)\n", cleared.len()),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        }
    }
}

async fn wait_notifications(
    ctx: &CliContext,
    notifications: &Notifications,
    device: &androidforge_core::model::Device,
    args: &NotificationWaitArgs,
    _assertion: bool,
) -> Result<CommandSuccess> {
    let selector: NotificationSelector = args.selector.clone().into_selector();
    let timeout = ctx.parse_timeout(
        args.timeout.as_deref(),
        ctx.config.file.ui.default_timeout_secs,
    )?;
    let result = notifications
        .wait(&device.serial, &selector, timeout, args.not_exists)
        .await?;
    let human = if args.not_exists {
        format!("Confirmed absence after {} attempt(s).\n", result.attempts)
    } else {
        format!(
            "Observed notification after {} attempt(s): {:?}\n",
            result.attempts,
            result
                .notification
                .as_ref()
                .map(|notification| notification.title.clone().or(notification.text.clone()))
        )
    };
    Ok(
        success("notifications.wait", serde_json::to_value(&result)?, human)
            .with_device(device.serial.clone()),
    )
}

pub async fn jobs(ctx: &CliContext, command: &JobsCommand) -> Result<CommandSuccess> {
    let (session, device) = target(ctx).await?;
    let jobs = Jobs::new(ctx.adb()?);
    match command {
        JobsCommand::List { package } => {
            let list = jobs.list(&device.serial, package.as_deref()).await?;
            let human = list
                .jobs
                .iter()
                .map(|job| {
                    format!(
                        "{} job={} service={} ready={:?} running={:?}\n",
                        job.package,
                        job.job_id.as_deref().unwrap_or("-"),
                        job.service.as_deref().unwrap_or("-"),
                        job.ready,
                        job.running
                    )
                })
                .collect::<Vec<_>>()
                .join("");
            Ok(success(
                "jobs.list",
                serde_json::to_value(&list)?,
                if human.is_empty() {
                    "No jobs parsed.\n".to_string()
                } else {
                    human
                },
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        }
        JobsCommand::Run { package, job_id } => {
            let result = jobs.run(&device.serial, package, job_id).await?;
            Ok(success(
                "jobs.run",
                serde_json::to_value(&result)?,
                format!(
                    "JobScheduler command accepted for {} job {}; confirmed={}\n",
                    result.package, result.job_id, result.confirmed
                ),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        }
        JobsCommand::Cancel { package, job_id } => {
            let result = jobs.cancel(&device.serial, package, job_id).await?;
            Ok(success(
                "jobs.cancel",
                serde_json::to_value(&result)?,
                format!(
                    "Cancel requested for {} job {}\n",
                    result.package, result.job_id
                ),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        }
        JobsCommand::Wait(args) | JobsCommand::Assert(args) => {
            wait_jobs(ctx, &jobs, &device, args).await
        }
    }
}

async fn wait_jobs(
    ctx: &CliContext,
    jobs: &Jobs,
    device: &androidforge_core::model::Device,
    args: &JobWaitArgs,
) -> Result<CommandSuccess> {
    let selector = androidforge_diagnostics::JobSelector {
        package: args.package.clone(),
        job_id: args.job_id.clone(),
        state: args.state.clone(),
    };
    let timeout = ctx.parse_timeout(
        args.timeout.as_deref(),
        ctx.config.file.ui.default_timeout_secs,
    )?;
    let result = jobs.wait(&device.serial, &selector, timeout).await?;
    Ok(success(
        "jobs.wait",
        serde_json::to_value(&result)?,
        format!(
            "Observed JobScheduler job after {} attempt(s): {:?}\n",
            result.attempts,
            result.job.as_ref().map(|job| job.job_id.clone())
        ),
    )
    .with_device(device.serial.clone()))
}

pub async fn alarms(ctx: &CliContext, command: &AlarmsCommand) -> Result<CommandSuccess> {
    let (session, device) = target(ctx).await?;
    let alarms = Alarms::new(ctx.adb()?);
    match command {
        AlarmsCommand::Next { package } | AlarmsCommand::List { package } => {
            let result = alarms.list(&device.serial, package.as_deref()).await?;
            let human = if result.alarms.is_empty() {
                "No alarms parsed.\n".to_string()
            } else {
                result
                    .alarms
                    .iter()
                    .map(|alarm| {
                        format!(
                            "{} {:?} when={:?} tag={:?}\n",
                            alarm.package.as_deref().unwrap_or("-"),
                            alarm.alarm_type,
                            alarm.when_iso,
                            alarm.tag
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("")
            };
            Ok(
                success("alarms.list", serde_json::to_value(&result)?, human)
                    .with_optional_session(session.map(|session| session.id.clone()))
                    .with_device(device.serial.clone()),
            )
        }
        AlarmsCommand::Wait { package, timeout } => {
            let timeout =
                ctx.parse_timeout(timeout.as_deref(), ctx.config.file.ui.default_timeout_secs)?;
            let result = alarms
                .wait(&device.serial, package.as_deref(), timeout)
                .await?;
            Ok(success(
                "alarms.wait",
                serde_json::to_value(&result)?,
                format!("Observed {} alarm(s).\n", result.alarms.len()),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        }
    }
}

pub async fn permissions(ctx: &CliContext, command: &PermissionsCommand) -> Result<CommandSuccess> {
    let (session, device) = target(ctx).await?;
    let permissions = Permissions::new(ctx.adb()?);
    let (package, permission, action) = match command {
        PermissionsCommand::Check {
            package,
            permission,
        } => (
            ctx.package_or_config(package.as_deref())?,
            permission.clone(),
            "check",
        ),
        PermissionsCommand::Grant {
            package,
            permission,
        } => (
            ctx.package_or_config(package.as_deref())?,
            permission.clone(),
            "grant",
        ),
        PermissionsCommand::Revoke {
            package,
            permission,
        } => (
            ctx.package_or_config(package.as_deref())?,
            permission.clone(),
            "revoke",
        ),
    };
    let state = match action {
        "check" => {
            permissions
                .check(&device.serial, &package, &permission)
                .await?
        }
        "grant" => {
            permissions
                .grant(&device.serial, &package, &permission)
                .await?
        }
        _ => {
            permissions
                .revoke(&device.serial, &package, &permission)
                .await?
        }
    };
    Ok(success(
        "permissions",
        serde_json::json!({"action": action, "state": state}),
        format!(
            "{} {}: granted={:?} source={}\n",
            action, permission, state.granted, state.source
        ),
    )
    .with_optional_session(session.map(|session| session.id.clone()))
    .with_device(device.serial.clone()))
}

pub async fn appops(ctx: &CliContext, command: &AppOpsCommand) -> Result<CommandSuccess> {
    let (session, device) = target(ctx).await?;
    let appops = AppOps::new(ctx.adb()?);
    match command {
        AppOpsCommand::Get { package, operation } => {
            let package = ctx.package_or_config(package.as_deref())?;
            let state = appops.get(&device.serial, &package, operation).await?;
            Ok(success(
                "appops.get",
                serde_json::to_value(&state)?,
                format!("{} {}={:?}\n", package, operation, state.mode),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        }
        AppOpsCommand::Set {
            package,
            operation,
            mode,
        } => {
            let package = ctx.package_or_config(package.as_deref())?;
            if ctx.dry_run {
                return Ok(success(
                    "appops.set",
                    serde_json::json!({"dry_run": true, "package": package, "operation": operation, "mode": mode}),
                    format!("Would set {operation}={mode} for {package}\n"),
                )
                .with_device(device.serial.clone()));
            }
            let state = appops
                .set(&device.serial, &package, operation, mode)
                .await?;
            Ok(success(
                "appops.set",
                serde_json::to_value(&state)?,
                format!("{} {}={:?}\n", package, operation, state.mode),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        }
        AppOpsCommand::Reset { package, operation } => {
            let package = ctx.package_or_config(package.as_deref())?;
            let state = appops
                .reset(&device.serial, &package, operation.as_deref())
                .await?;
            Ok(success(
                "appops.reset",
                serde_json::to_value(&state)?,
                format!("Reset AppOps for {package}\n"),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        }
    }
}

pub async fn doze(ctx: &CliContext, command: &DozeCommand) -> Result<CommandSuccess> {
    let (session, device) = target(ctx).await?;
    let manager = DeviceStateManager::new(ctx.adb()?);
    let state = match command {
        DozeCommand::Status => manager.doze(&device.serial).await?,
        DozeCommand::Force => {
            if ctx.dry_run {
                return Ok(success(
                    "doze.force",
                    serde_json::json!({"dry_run": true, "serial": device.serial}),
                    "Would force the device into Doze.\n".to_string(),
                )
                .with_optional_session(session.map(|session| session.id.clone()))
                .with_device(device.serial.clone()));
            }
            manager.force_doze(&device.serial, ctx.yes).await?
        }
        DozeCommand::Unforce => manager.unforce_doze(&device.serial).await?,
    };
    Ok(success(
        "doze",
        serde_json::to_value(&state)?,
        format!(
            "Doze state={:?} light={:?} charging={:?}\n",
            state.state, state.light_state, state.charging
        ),
    )
    .with_optional_session(session.map(|session| session.id.clone()))
    .with_device(device.serial.clone()))
}

pub async fn standby(ctx: &CliContext, command: &StandbyCommand) -> Result<CommandSuccess> {
    let (session, device) = target(ctx).await?;
    let manager = DeviceStateManager::new(ctx.adb()?);
    match command {
        StandbyCommand::Get { package } => {
            let package = ctx.package_or_config(package.as_deref())?;
            let state = manager.standby(&device.serial, &package).await?;
            Ok(success(
                "standby.get",
                serde_json::to_value(&state)?,
                format!("{} bucket={:?}\n", package, state.bucket),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        }
        StandbyCommand::Set { package, bucket } => {
            let package = ctx.package_or_config(package.as_deref())?;
            let state = manager
                .set_standby(&device.serial, &package, bucket)
                .await?;
            Ok(success(
                "standby.set",
                serde_json::to_value(&state)?,
                format!("{} bucket={:?}\n", package, state.bucket),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        }
    }
}

pub async fn battery(ctx: &CliContext, command: &BatteryCommand) -> Result<CommandSuccess> {
    let (session, device) = target(ctx).await?;
    let manager = DeviceStateManager::new(ctx.adb()?);
    let state = match command {
        BatteryCommand::Status => manager.battery(&device.serial).await?,
        BatteryCommand::Unplug => manager.battery_unplug(&device.serial).await?,
        BatteryCommand::Reset => manager.battery_reset(&device.serial).await?,
    };
    Ok(success(
        "battery",
        serde_json::to_value(&state)?,
        format!(
            "level={:?} status={:?} ac={:?} usb={:?}\n",
            state.level, state.status, state.ac_powered, state.usb_powered
        ),
    )
    .with_optional_session(session.map(|session| session.id.clone()))
    .with_device(device.serial.clone()))
}

pub async fn network(ctx: &CliContext, command: &NetworkCommand) -> Result<CommandSuccess> {
    let (session, device) = target(ctx).await?;
    let manager = DeviceStateManager::new(ctx.adb()?);
    let state = match command {
        NetworkCommand::Status => manager.network(&device.serial).await?,
        NetworkCommand::Airplane { enable, disable } => {
            let enabled = bool_switch(*enable, *disable, "airplane")?;
            manager
                .set_airplane(&device.serial, enabled, ctx.yes)
                .await?
        }
        NetworkCommand::Wifi { enable, disable } => {
            let enabled = bool_switch(*enable, *disable, "wifi")?;
            manager.set_wifi(&device.serial, enabled).await?
        }
        NetworkCommand::Data { enable, disable } => {
            let enabled = bool_switch(*enable, *disable, "data")?;
            manager.set_mobile_data(&device.serial, enabled).await?
        }
    };
    Ok(success(
        "network",
        serde_json::to_value(&state)?,
        format!(
            "airplane={:?} wifi={:?} mobile_data={:?} active={:?}\n",
            state.airplane_mode,
            state.wifi_enabled,
            state.mobile_data_enabled,
            state.active_network
        ),
    )
    .with_optional_session(session.map(|session| session.id.clone()))
    .with_device(device.serial.clone()))
}

fn bool_switch(enable: bool, disable: bool, label: &str) -> Result<bool> {
    match (enable, disable) {
        (true, false) => Ok(true),
        (false, true) => Ok(false),
        _ => Err(ForgeError::invalid_argument(format!(
            "choose --enable or --disable for {label}"
        ))),
    }
}

pub async fn time(ctx: &CliContext, command: &TimeCommand) -> Result<CommandSuccess> {
    match command {
        TimeCommand::Get => {
            let (session, device) = target(ctx).await?;
            let manager = DeviceStateManager::new(ctx.adb()?);
            let state = manager.time(&device.serial).await?;
            Ok(success(
                "time.get",
                serde_json::to_value(&state)?,
                serde_json::to_string_pretty(&state).unwrap_or_default() + "\n",
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        }
        TimeCommand::Set(args) => time_set(ctx, args).await,
        TimeCommand::Restore => time_restore(ctx).await,
    }
}

async fn time_set(ctx: &CliContext, args: &TimeSetArgs) -> Result<CommandSuccess> {
    if !ctx.yes {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            "changing device time requires --yes",
        )
        .with_suggestion("Record the previous state with `androidforge time get` first.")
        .with_suggestion("Use `androidforge time restore` afterwards."));
    }
    let epoch = match (&args.epoch, &args.iso) {
        (Some(epoch), None) => *epoch,
        (None, Some(iso)) => DateTime::parse_from_rfc3339(iso)
            .map(|value| value.timestamp())
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%d %H:%M:%S")
                    .map(|value| value.and_utc().timestamp())
            })
            .map_err(|error| {
                ForgeError::invalid_argument(format!("could not parse --iso {iso:?}: {error}"))
            })?,
        _ => {
            return Err(ForgeError::invalid_argument(
                "time set requires exactly one of --epoch or --iso",
            ));
        }
    };
    let mut session = ctx.require_session()?;
    let device = ctx.resolve_device(Some(&session)).await?;
    let manager = DeviceStateManager::new(ctx.adb()?);
    if ctx.dry_run {
        return Ok(success(
            "time.set",
            serde_json::json!({"dry_run": true, "epoch": epoch, "serial": device.serial}),
            format!("Would set device time to {epoch}.\n"),
        )
        .with_session(session.id)
        .with_device(device.serial.clone()));
    }
    let before = manager.time(&device.serial).await?;
    session.record_device_setting(
        &ctx.paths,
        "time:epoch",
        before.epoch_seconds.map(|epoch| epoch.to_string()),
    )?;
    session.record_device_setting(
        &ctx.paths,
        "time:auto_time",
        before.auto_time.map(|value| value.to_string()),
    )?;
    session.record_device_setting(
        &ctx.paths,
        "time:auto_time_zone",
        before.auto_time_zone.map(|value| value.to_string()),
    )?;
    let state = manager.set_time(&device.serial, epoch, ctx.yes).await?;
    Ok(success(
        "time.set",
        serde_json::to_value(&state)?,
        format!(
            "Device time set to {} (epoch {:?}).\n",
            state.device_time, state.epoch_seconds
        ),
    )
    .with_session(session.id)
    .with_device(device.serial.clone()))
}

async fn time_restore(ctx: &CliContext) -> Result<CommandSuccess> {
    let mut session = ctx.require_session()?;
    let device = ctx.resolve_device(Some(&session)).await?;
    let manager = DeviceStateManager::new(ctx.adb()?);
    let saved = session.device_settings.clone();
    let epoch = saved
        .get("time:epoch")
        .and_then(|value| value.as_ref())
        .and_then(|value| value.parse::<i64>().ok());
    let auto_time = saved
        .get("time:auto_time")
        .and_then(|value| value.as_ref())
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(true);
    let auto_time_zone = saved
        .get("time:auto_time_zone")
        .and_then(|value| value.as_ref())
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(true);
    if saved.is_empty() {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            "no recorded time state is available for restore",
        )
        .with_suggestion("`androidforge time set` records the previous state before changing it.")
        .session(session.id));
    }
    let mut warnings = Vec::new();
    if let Err(error) = manager
        .set_auto_time(&device.serial, auto_time, auto_time_zone)
        .await
    {
        warnings.push(format!(
            "could not restore automatic time settings: {}",
            error.message()
        ));
    }
    if !auto_time {
        if let Some(epoch) = epoch {
            if let Err(error) = manager.set_time(&device.serial, epoch, true).await {
                warnings.push(format!(
                    "could not restore previous time: {}",
                    error.message()
                ));
            }
        }
    }
    for key in ["time:epoch", "time:auto_time", "time:auto_time_zone"] {
        session.device_settings.remove(key);
    }
    session.save(&ctx.paths)?;
    let state = manager.time(&device.serial).await?;
    let mut success = success(
        "time.restore",
        serde_json::json!({"state": state, "warnings": warnings}),
        format!("Device time restore attempted: {}\n", state.device_time),
    )
    .with_session(session.id)
    .with_device(device.serial.clone());
    if !warnings.is_empty() {
        success.exit_code = 10;
    }
    Ok(success)
}
