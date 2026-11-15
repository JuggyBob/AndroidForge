use androidforge_core::error::Result;
use androidforge_core::output::CommandSuccess;
use androidforge_emulator::DevicePreparer;

use crate::cli::{DevicesCommand, DevicesWaitArgs, PrepareArgs};
use crate::commands::common::{device_human, device_json, prepare_options, success};
use crate::context::CliContext;

pub async fn devices(ctx: &CliContext, command: &DevicesCommand) -> Result<CommandSuccess> {
    match command {
        DevicesCommand::List => {
            let adb = ctx.adb()?;
            let devices = adb.list_devices(true).await?;
            let mut rows = Vec::new();
            let mut json = Vec::new();
            for device in &devices {
                let owner = ctx.owner_for_serial(&device.serial)?;
                rows.push(device_human(device, owner.as_ref()));
                json.push(device_json(device, owner.as_ref()));
            }
            let human = if rows.is_empty() {
                "No devices connected.\n".to_string()
            } else {
                format!("{}\n", rows.join("\n"))
            };
            Ok(success(
                "devices.list",
                serde_json::json!({"devices": json, "count": devices.len()}),
                human,
            ))
        }
        DevicesCommand::Inspect => {
            let session = ctx.load_session(false)?;
            let serial = ctx.resolve_serial(session.as_ref()).await?;
            let adb = ctx.adb()?;
            let device = adb.inspect(&serial).await?;
            let owner = ctx.owner_for_serial(&serial)?;
            let human = format!(
                "{}\nmodel: {}\nmanufacturer: {}\nAndroid: {} API {}\narch: {}\nresolution: {}\ndensity: {}\nboot: {:?}\nlocked: {:?}\nscreen_on: {:?}\n",
                device_human(&device, owner.as_ref()),
                device.model.as_deref().unwrap_or("-"),
                device.manufacturer.as_deref().unwrap_or("-"),
                device.android_version.as_deref().unwrap_or("-"),
                device
                    .api_level
                    .map(|level| level.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                device.architecture.as_deref().unwrap_or("-"),
                device.resolution.as_deref().unwrap_or("-"),
                device
                    .density
                    .map(|density| density.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                device.boot_completed,
                device.locked,
                device.screen_on,
            );
            let mut full = serde_json::to_value(&device)?;
            if let (Some(object), Some(owner)) = (full.as_object_mut(), owner.as_ref()) {
                object.insert(
                    "owner".to_string(),
                    serde_json::json!({
                        "session_id": owner.id,
                        "owner": owner.owner,
                        "kind": owner.kind,
                        "state": owner.state,
                    }),
                );
            }
            Ok(success("devices.inspect", full, human).with_device(serial))
        }
        DevicesCommand::Wait(args) => wait(ctx, args).await,
        DevicesCommand::Prepare(args) => prepare(ctx, args).await,
        DevicesCommand::Restore => restore(ctx).await,
    }
}

async fn wait(ctx: &CliContext, args: &DevicesWaitArgs) -> Result<CommandSuccess> {
    let adb = ctx.adb()?;
    let session = ctx.load_session(false)?;
    let serial = if let Some(session) = &session {
        Some(session.serial()?.to_string())
    } else {
        ctx.serial_arg.clone()
    };
    let timeout = ctx.parse_timeout(args.timeout.as_deref(), ctx.config.file.timeouts.boot_secs)?;
    let device = adb
        .wait_for_device(serial.as_deref(), args.boot, timeout)
        .await?;
    let human = format!("Device ready: {}\n", device.short_summary());
    Ok(success("devices.wait", serde_json::to_value(&device)?, human).with_device(device.serial))
}

async fn prepare(ctx: &CliContext, args: &PrepareArgs) -> Result<CommandSuccess> {
    let mut session = ctx.require_session()?;
    let adb = ctx.adb()?;
    let serial = session.serial()?.to_string();
    let options = prepare_options(args);
    if ctx.dry_run {
        return Ok(success(
            "devices.prepare",
            serde_json::json!({"dry_run": true, "serial": serial, "options": options}),
            "Would apply deterministic device preparation.\n".to_string(),
        )
        .with_session(session.id)
        .with_device(serial));
    }
    let preparer = DevicePreparer::new(adb);
    let result = preparer
        .prepare(&serial, &options, &mut session, &ctx.paths)
        .await?;
    let human = format!(
        "Prepared {}: {} change(s), {} warning(s)\n",
        serial,
        result
            .changes
            .iter()
            .filter(|change| change.applied)
            .count(),
        result.warnings.len()
    );
    Ok(
        success("devices.prepare", serde_json::to_value(&result)?, human)
            .with_session(session.id)
            .with_device(serial),
    )
}

async fn restore(ctx: &CliContext) -> Result<CommandSuccess> {
    let mut session = ctx.require_session()?;
    let serial = session.serial()?.to_string();
    if ctx.dry_run {
        return Ok(success(
            "devices.restore",
            serde_json::json!({"dry_run": true, "serial": serial}),
            "Would restore device settings changed by AndroidForge.\n".to_string(),
        )
        .with_session(session.id)
        .with_device(serial));
    }
    let preparer = DevicePreparer::new(ctx.adb()?);
    let result = preparer.restore(&serial, &mut session, &ctx.paths).await?;
    Ok(success(
        "devices.restore",
        serde_json::to_value(&result)?,
        format!(
            "Restored {} setting(s); {} failed.\n",
            result.restored.len(),
            result.failed.len()
        ),
    )
    .with_session(session.id)
    .with_device(serial))
}
