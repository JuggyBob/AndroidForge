use std::io::Read;

use androidforge_core::error::{ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_diagnostics::{HealthThresholds, collect_health};
use androidforge_emulator::DevicePreparer;

use crate::cli::{
    DevicesCommand, DevicesEndpointArgs, DevicesHealthArgs, DevicesPairArgs, DevicesWaitArgs,
    PrepareArgs,
};
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
        DevicesCommand::Pair(args) => pair(ctx, args).await,
        DevicesCommand::Connect(args) => connect(ctx, args).await,
        DevicesCommand::Disconnect(args) => disconnect(ctx, args).await,
        DevicesCommand::Health(args) => health(ctx, args).await,
    }
}

async fn pair(ctx: &CliContext, args: &DevicesPairArgs) -> Result<CommandSuccess> {
    if ctx.dry_run {
        return Ok(success(
            "devices.pair",
            serde_json::json!({
                "dry_run": true,
                "endpoint": args.endpoint,
                "would": "pair with the wireless-debugging endpoint using the six-digit code",
                "code_stored": false,
            }),
            format!(
                "Would pair with {} without storing the pairing code.\n",
                args.endpoint
            ),
        ));
    }
    let code = if args.code_stdin {
        let mut value = String::new();
        std::io::stdin().read_to_string(&mut value).map_err(|error| {
            ForgeError::invalid_argument(format!(
                "could not read the pairing code from stdin: {error}"
            ))
            .with_suggestion("Pipe the six-digit code, for example: echo 123456 | androidforge devices pair HOST:PORT --code-stdin --json")
        })?;
        value.trim().to_string()
    } else {
        args.code.clone().ok_or_else(|| {
            ForgeError::invalid_argument("a wireless pairing code is required")
                .with_suggestion("Pass --code-stdin and pipe the six-digit code.")
                .with_suggestion(
                    "--code is accepted for compatibility, but the code may be visible to other local processes.",
                )
        })?
    };
    let adb = ctx.adb()?;
    let result = adb.pair(&args.endpoint, &code).await?;
    Ok(success(
        "devices.pair",
        serde_json::to_value(&result)?,
        format!(
            "Paired with {}.\nThe pairing code was not stored, logged or included in the result.\n",
            result.endpoint
        ),
    ))
}

async fn connect(ctx: &CliContext, args: &DevicesEndpointArgs) -> Result<CommandSuccess> {
    if ctx.dry_run {
        return Ok(success(
            "devices.connect",
            serde_json::json!({"dry_run": true, "endpoint": args.endpoint}),
            format!("Would connect to {}.\n", args.endpoint),
        ));
    }
    let adb = ctx.adb()?;
    let result = adb.connect(&args.endpoint).await?;
    let human = format!(
        "{}{}.\nserial: {}\n",
        if result.already_connected {
            "Already connected to "
        } else {
            "Connected to "
        },
        result.endpoint,
        result.serial.as_deref().unwrap_or("-")
    );
    Ok(success(
        "devices.connect",
        serde_json::to_value(&result)?,
        human,
    ))
}

async fn disconnect(ctx: &CliContext, args: &DevicesEndpointArgs) -> Result<CommandSuccess> {
    if ctx.dry_run {
        return Ok(success(
            "devices.disconnect",
            serde_json::json!({"dry_run": true, "endpoint": args.endpoint}),
            format!("Would disconnect {}.\n", args.endpoint),
        ));
    }
    let adb = ctx.adb()?;
    let result = adb.disconnect(&args.endpoint).await?;
    Ok(success(
        "devices.disconnect",
        serde_json::to_value(&result)?,
        format!("Disconnected {}.\n", result.endpoint),
    ))
}

async fn health(ctx: &CliContext, args: &DevicesHealthArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(false)?;
    let serial = ctx.resolve_serial(session.as_ref()).await?;
    let adb = ctx.adb()?;
    let device = adb.inspect(&serial).await?;
    let thresholds = HealthThresholds::from(&ctx.config.file.health);
    let mut report = collect_health(&adb, &device, &thresholds).await;
    if args.no_evidence {
        report.evidence.clear();
    }
    let human = format!(
        "Device health for {}\nADB state: {}\nbattery: {}%{} (status: {}, temperature: {})\nthermal: {}\nstorage: {} MiB free of {} MiB\nmemory: {} avail of {}\nnetwork: airplane_mode={} wifi={} interface={}\nscreen: interactive={} locked={}\nuptime: {}\n{}\n",
        report.serial,
        report.adb_state,
        report
            .battery
            .level_percent
            .map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_string()),
        if report.battery.charging == Some(true) {
            " charging"
        } else {
            ""
        },
        report.battery.status.as_deref().unwrap_or("unknown"),
        report
            .battery
            .temperature_c
            .map(|value| format!("{value:.1}C"))
            .unwrap_or_else(|| "unknown".to_string()),
        report.thermal.status.as_deref().unwrap_or("unknown"),
        report
            .storage
            .available_bytes
            .map(|value| value / (1024 * 1024))
            .map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_string()),
        report
            .storage
            .total_bytes
            .map(|value| (value / (1024 * 1024)).to_string())
            .unwrap_or_else(|| "?".to_string()),
        report
            .memory
            .available_bytes
            .map(|value| format!("{} MiB", value / (1024 * 1024)))
            .unwrap_or_else(|| "unknown".to_string()),
        report
            .memory
            .total_bytes
            .map(|value| format!("{} MiB", value / (1024 * 1024)))
            .unwrap_or_else(|| "unknown".to_string()),
        report
            .network
            .airplane_mode
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        report
            .network
            .wifi_enabled
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        report
            .network
            .active_interface
            .as_deref()
            .unwrap_or("unknown"),
        report
            .screen
            .interactive
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        report
            .screen
            .locked
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        report
            .uptime_seconds
            .map(|value| format!("{value}s"))
            .unwrap_or_else(|| "unknown".to_string()),
        report
            .warnings
            .iter()
            .map(|warning| format!("warning: {warning}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    Ok(
        success("devices.health", serde_json::to_value(&report)?, human)
            .with_optional_session(session.map(|session| session.id))
            .with_device(serial),
    )
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
    let device = ctx.resolve_device(Some(&session)).await?;
    ctx.ensure_physical_mutation_allowed(&device, "preparing a device")?;
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
    let device = ctx.resolve_device(Some(&session)).await?;
    ctx.ensure_physical_mutation_allowed(&device, "restoring device settings")?;
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
