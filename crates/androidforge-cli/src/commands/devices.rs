// The typed scenario API below is exposed for CLI/MCP integration that lands in
// a later workstream, so part of it is intentionally unused until then.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::{Device, DeviceKind};
use androidforge_core::output::CommandSuccess;
use androidforge_core::paths::ForgePaths;
use androidforge_core::session::Session;
use androidforge_core::util::{ensure_dir, write_json_atomic};
use androidforge_diagnostics::{HealthThresholds, collect_health};
use androidforge_emulator::{DevicePrepareOptions, DevicePreparer};
use serde::{Deserialize, Serialize};

use crate::cli::{
    DevicesCommand, DevicesEndpointArgs, DevicesHealthArgs, DevicesPairArgs, DevicesScenarioArgs,
    DevicesTagArgs, DevicesTagCommand, DevicesWaitArgs, PrepareArgs,
};
use crate::commands::common::{device_human, device_json, prepare_options, success};
use crate::context::{CliContext, Risk};

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
        DevicesCommand::Scenario(args) => scenario(ctx, args).await,
        DevicesCommand::Tag { command } => device_tag(ctx, command),
        // Narrow shell/input operations live in device.rs; routing them here
        // keeps `devices` (and its `device` alias) on one typed dispatch path.
        DevicesCommand::Shell(args) => crate::commands::device::shell(ctx, args).await,
        DevicesCommand::Input { command } => crate::commands::device::input(ctx, command).await,
    }
}

/// Resolve the four mutually exclusive scenario flags into a typed request.
fn parse_scenario_args(args: &DevicesScenarioArgs) -> Result<DeviceScenario> {
    let selected = [
        args.night.is_some(),
        args.locale.is_some(),
        args.font.is_some(),
        args.network.is_some(),
    ]
    .into_iter()
    .filter(|selected| *selected)
    .count();
    if selected != 1 {
        return Err(ForgeError::invalid_argument(
            "devices scenario requires exactly one of --night, --locale, --font or --network",
        )
        .with_suggestion("Example: androidforge devices scenario --night yes --json"));
    }
    if let Some(value) = &args.night {
        return Ok(DeviceScenario::NightMode(NightMode::parse(value)?));
    }
    if let Some(value) = &args.locale {
        return Ok(DeviceScenario::Locale(validate_locale(value)?));
    }
    if let Some(value) = &args.font {
        return Ok(DeviceScenario::FontScale(validate_font_scale(value)?));
    }
    if let Some(value) = &args.network {
        return Ok(DeviceScenario::Network(NetworkScenario::parse(value)?));
    }
    unreachable!("exactly one scenario flag was selected")
}

/// `devices scenario`: thin CLI over the typed scenario API. The core records
/// restore state in the session, applies the existing prepare/network
/// primitives and enforces the matching physical-device risk. Dry runs build a
/// plan with no session, ADB or device access and cannot mutate anything.
async fn scenario(ctx: &CliContext, args: &DevicesScenarioArgs) -> Result<CommandSuccess> {
    let scenario = parse_scenario_args(args)?;
    // A preview needs no session: use the explicit serial or the current
    // session's serial when it is already available.
    let preview_serial = ctx.serial_arg.clone().or_else(|| {
        ctx.load_session(false)
            .ok()
            .flatten()
            .and_then(|session| session.serial().ok().map(str::to_string))
    });
    let mut request = ScenarioRequest {
        serial: preview_serial.unwrap_or_default(),
        scenario: scenario.clone(),
        consent: ScenarioConsent::from_context(ctx),
        dry_run: ctx.dry_run,
    };
    request.validate()?;
    let plan = plan_scenario(&request);
    if ctx.dry_run {
        let outcome = scenario_dry_run(&request);
        return Ok(success(
            "devices.scenario",
            serde_json::to_value(&outcome)?,
            format!(
                "Dry run: would {} on {} ({}); no device was changed.\n",
                plan.operation, plan.serial, plan.risk
            ),
        ));
    }
    let mut session = ctx.require_session()?;
    request.serial = session.serial()?.to_string();
    let device = ctx.resolve_device(Some(&session)).await?;
    let outcome = apply_device_scenario(ctx, &mut session, &device, &request).await?;
    let mut human = format!(
        "{} on {}: applied={}\n",
        plan.operation, request.serial, outcome.applied
    );
    for warning in &outcome.warnings {
        human.push_str(&format!("warning: {warning}\n"));
    }
    Ok(
        success("devices.scenario", serde_json::to_value(&outcome)?, human)
            .with_session(session.id)
            .with_device(request.serial),
    )
}

/// `devices tag add/remove/list/clear`: host-only tag manifest operations.
/// No ADB handle is created and no device is contacted at any point.
fn device_tag(ctx: &CliContext, command: &DevicesTagCommand) -> Result<CommandSuccess> {
    let root = device_tag_root(&ctx.paths);
    match command {
        DevicesTagCommand::Add(args) => tag_mutation(
            ctx,
            "devices.tag.add",
            "tag",
            args,
            ctx.dry_run,
            add_device_tag,
        ),
        DevicesTagCommand::Remove(args) => tag_mutation(
            ctx,
            "devices.tag.remove",
            "untag",
            args,
            ctx.dry_run,
            remove_device_tag,
        ),
        DevicesTagCommand::Clear { serial } => {
            if ctx.dry_run {
                return Ok(success(
                    "devices.tag.clear",
                    serde_json::json!({"dry_run": true, "serial": serial, "operation": "clear"}),
                    format!("Would clear every tag on {serial}.\n"),
                ));
            }
            let change = clear_device_tags(&root, serial)?;
            Ok(success(
                "devices.tag.clear",
                serde_json::to_value(&change)?,
                format!(
                    "{} tags on {} (now: {}).\n",
                    if change.changed {
                        "Cleared"
                    } else {
                        "No change to"
                    },
                    serial,
                    join_tags(&change.tags)
                ),
            ))
        }
        DevicesTagCommand::List { serial } => {
            let entries = list_device_tags(&root, serial.as_deref())?;
            let mut human = if entries.is_empty() {
                String::from("No device tags.\n")
            } else {
                String::new()
            };
            for entry in &entries {
                human.push_str(&format!("{}: {}\n", entry.serial, join_tags(&entry.tags)));
            }
            Ok(success(
                "devices.tag.list",
                serde_json::json!({"entries": entries}),
                human,
            ))
        }
    }
}

fn tag_mutation<F>(
    ctx: &CliContext,
    command: &str,
    operation: &str,
    args: &DevicesTagArgs,
    dry_run: bool,
    mutate: F,
) -> Result<CommandSuccess>
where
    F: FnOnce(&Path, &str, &str) -> Result<DeviceTagChange>,
{
    if dry_run {
        return Ok(success(
            command,
            serde_json::json!({
                "dry_run": true,
                "operation": operation,
                "serial": args.serial,
                "tag": args.tag,
            }),
            format!(
                "Would {operation} tag '{}' on {} (no host state changed).\n",
                args.tag, args.serial
            ),
        ));
    }
    let change = mutate(&device_tag_root(&ctx.paths), &args.serial, &args.tag)?;
    Ok(success(
        command,
        serde_json::to_value(&change)?,
        format!(
            "{} tag '{}' on {} (tags now: {}).\n",
            if change.changed {
                "Applied"
            } else {
                "No change for"
            },
            args.tag,
            args.serial,
            join_tags(&change.tags)
        ),
    ))
}

fn join_tags(tags: &[String]) -> String {
    if tags.is_empty() {
        "(none)".to_string()
    } else {
        tags.join(", ")
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
    ctx.ensure_risk_allowed(&device, Risk::Data, "preparing a device")?;
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
    ctx.ensure_risk_allowed(&device, Risk::Data, "restoring device settings")?;
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

// ---------------------------------------------------------------------------
// Device scenario controls (issue #3 item 11)
//
// Typed, machine-readable requests for the settings users most often change
// during a scenario run. They are deliberately thin: night mode, locale and
// font scale go through `DevicePreparer` (the same engine as `devices prepare` /
// `devices restore`), and network modes go through the existing network profile
// catalog. Nothing here writes a device setting directly.
// ---------------------------------------------------------------------------

/// Night mode values accepted by `cmd uimode night`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NightMode {
    Yes,
    No,
    Auto,
}

impl NightMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Auto => "auto",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "yes" | "on" | "true" | "1" => Ok(Self::Yes),
            "no" | "off" | "false" | "0" => Ok(Self::No),
            "auto" => Ok(Self::Auto),
            _ => Err(
                ForgeError::invalid_argument(format!("unknown night mode '{value}'"))
                    .with_suggestion("Use yes, no or auto."),
            ),
        }
    }
}

/// Network states exposed as scenario controls. Each maps onto an existing
/// named profile in `system.rs`; there is no separate network implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkScenario {
    Offline,
    Online,
    WifiOnly,
    DataOnly,
}

impl NetworkScenario {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Online => "online",
            Self::WifiOnly => "wifi-only",
            Self::DataOnly => "data-only",
        }
    }

    /// Name of the existing network profile that implements this mode.
    pub fn profile(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Online => "default",
            Self::WifiOnly => "wifi-only",
            Self::DataOnly => "data-only",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "offline" | "airplane" => Ok(Self::Offline),
            "online" | "default" | "normal" => Ok(Self::Online),
            "wifi-only" | "wifi" => Ok(Self::WifiOnly),
            "data-only" | "data" => Ok(Self::DataOnly),
            _ => Err(ForgeError::invalid_argument(format!(
                "unknown network scenario '{value}'"
            ))
            .with_suggestion(
                "Use offline, online, wifi-only or data-only (aliases: airplane, wifi, data).",
            )),
        }
    }
}

/// A single device setting change requested by a scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceScenario {
    NightMode(NightMode),
    Locale(String),
    FontScale(String),
    Network(NetworkScenario),
}

impl DeviceScenario {
    /// Parse `night:yes`, `locale=en-US`, `font:1.15`, `network=offline`, etc.
    /// `:` and `=` are interchangeable; keys are case-insensitive.
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        let (key, argument) = value
            .split_once('=')
            .or_else(|| value.split_once(':'))
            .ok_or_else(|| {
                ForgeError::invalid_argument(format!("invalid device scenario '{value}'"))
                    .with_suggestion(
                        "Use night:<yes|no|auto>, locale:<tag>, font:<scale> or network:<mode>.",
                    )
            })?;
        let key = key.trim().to_ascii_lowercase().replace('_', "-");
        let argument = argument.trim();
        if argument.is_empty() {
            return Err(ForgeError::invalid_argument(format!(
                "device scenario '{key}' needs a value"
            )));
        }
        match key.as_str() {
            "night" | "night-mode" | "dark-mode" => {
                Ok(Self::NightMode(NightMode::parse(argument)?))
            }
            "locale" | "language" => Ok(Self::Locale(validate_locale(argument)?)),
            "font" | "font-scale" => Ok(Self::FontScale(validate_font_scale(argument)?)),
            "network" | "net" => Ok(Self::Network(NetworkScenario::parse(argument)?)),
            _ => Err(
                ForgeError::invalid_argument(format!("unknown device scenario '{key}'"))
                    .with_suggestion("Use night, locale, font or network."),
            ),
        }
    }

    /// Stable, machine-readable form used in plans and session notes.
    pub fn as_str(&self) -> String {
        match self {
            Self::NightMode(mode) => format!("night:{}", mode.as_str()),
            Self::Locale(value) => format!("locale:{value}"),
            Self::FontScale(value) => format!("font:{value}"),
            Self::Network(mode) => format!("network:{}", mode.as_str()),
        }
    }

    /// Human-readable operation description used by the risk gate.
    pub fn operation(&self) -> String {
        match self {
            Self::NightMode(mode) => format!("setting night mode to {}", mode.as_str()),
            Self::Locale(locale) => format!("changing device locale to {locale}"),
            Self::FontScale(scale) => format!("changing device font scale to {scale}"),
            Self::Network(mode) => format!("applying the {mode:?} network scenario"),
        }
    }

    /// Settings changes use `Risk::Data`; network changes use `Risk::Network`.
    pub fn risk(&self) -> Risk {
        match self {
            Self::Network(_) => Risk::Network,
            _ => Risk::Data,
        }
    }

    /// Session keys that must be snapshotted before mutation. Listed in the
    /// same key order `Session::device_settings` uses for restore.
    pub fn restore_keys(&self) -> &'static [&'static str] {
        match self {
            Self::NightMode(_) => &["cmd:uimode_night"],
            Self::Locale(_) => &["system:system_locales"],
            Self::FontScale(_) => &["system:font_scale"],
            Self::Network(_) => &[
                "network:airplane",
                "network:data",
                "network:proxy",
                "network:wifi",
            ],
        }
    }

    /// Empty snapshot with a slot for every key this scenario will record.
    pub fn snapshot(&self) -> ScenarioSnapshot {
        let mut snapshot = ScenarioSnapshot::default();
        for key in self.restore_keys() {
            snapshot.record(*key, None);
        }
        snapshot
    }

    /// Pure dry-run/mutation plan; performs no I/O.
    pub fn plan(&self, serial: &str, dry_run: bool) -> ScenarioPlan {
        ScenarioPlan {
            serial: serial.to_string(),
            scenario: self.as_str(),
            risk: risk_name(self.risk()).to_string(),
            operation: self.operation(),
            restore_keys: self
                .snapshot()
                .keys_in_restore_order()
                .into_iter()
                .map(str::to_string)
                .collect(),
            will_mutate: !dry_run,
            dry_run,
        }
    }

    /// Options for `DevicePreparer::prepare`; `None` for network scenarios.
    pub fn prepare_options(&self) -> Option<DevicePrepareOptions> {
        let mut options = DevicePrepareOptions {
            animations: false,
            keep_awake: false,
            wake: false,
            ..DevicePrepareOptions::default()
        };
        match self {
            Self::Locale(value) => options.locale = Some(value.clone()),
            Self::FontScale(value) => options.font_scale = Some(value.clone()),
            _ => return None,
        }
        Some(options)
    }

    pub fn night_mode(&self) -> Option<NightMode> {
        match self {
            Self::NightMode(mode) => Some(*mode),
            _ => None,
        }
    }

    pub fn network(&self) -> Option<NetworkScenario> {
        match self {
            Self::Network(mode) => Some(*mode),
            _ => None,
        }
    }
}

/// Validate a single BCP-47-like locale tag before it reaches `settings put`.
/// Underscores are normalised to hyphens; shell metacharacters are rejected.
pub fn validate_locale(value: &str) -> Result<String> {
    let normalized = value.trim().replace('_', "-");
    let parts = normalized.split('-').collect::<Vec<_>>();
    let language = parts[0];
    let invalid = normalized.is_empty()
        || normalized.len() > 35
        || !(2..=3).contains(&language.len())
        || !language.bytes().all(|byte| byte.is_ascii_alphabetic())
        || parts.iter().any(|part| {
            part.is_empty()
                || part.len() > 8
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        });
    if invalid {
        return Err(ForgeError::invalid_argument(format!(
            "unsafe or unsupported locale '{value}'"
        ))
        .with_suggestion("Use a single locale tag such as en-US or fr-FR."));
    }
    Ok(normalized)
}

/// Validate a font scale before it reaches `settings put system font_scale`.
/// Only plain decimal values in the Android-safe range are accepted.
pub fn validate_font_scale(value: &str) -> Result<String> {
    let trimmed = value.trim();
    let mut parts = trimmed.split('.');
    let integer = parts.next().unwrap_or("");
    let fraction = parts.next();
    let shape_ok = !integer.is_empty()
        && integer.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.is_none_or(|fraction| {
            !fraction.is_empty() && fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
        && parts.next().is_none();
    let number = shape_ok
        .then(|| trimmed.parse::<f64>().ok())
        .flatten()
        .unwrap_or(f64::NAN);
    if !shape_ok || !(0.5..=2.0).contains(&number) {
        return Err(
            ForgeError::invalid_argument(format!("unsafe font scale '{value}'"))
                .with_suggestion("Use a decimal font scale between 0.5 and 2.0, for example 1.15."),
        );
    }
    Ok(trimmed.to_string())
}

/// Snapshot of the values a scenario will restore, keyed exactly like
/// `Session::device_settings`. The first value recorded for a key wins, and
/// restore iterates in key order, matching `DevicePreparer::restore`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioSnapshot {
    #[serde(default)]
    settings: BTreeMap<String, Option<String>>,
}

impl ScenarioSnapshot {
    /// Record a previous value. Returns `false` when the key already had a
    /// value, mirroring `Session::record_device_setting`'s first-write rule.
    pub fn record(&mut self, key: impl Into<String>, previous: Option<String>) -> bool {
        use std::collections::btree_map::Entry;
        match self.settings.entry(key.into()) {
            Entry::Vacant(entry) => {
                entry.insert(previous);
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    pub fn get(&self, key: &str) -> Option<Option<&str>> {
        self.settings.get(key).map(|value| value.as_deref())
    }

    /// Keys in the order `DevicePreparer::restore` will process them.
    pub fn keys_in_restore_order(&self) -> Vec<&str> {
        self.settings.keys().map(String::as_str).collect()
    }

    pub fn entries_in_restore_order(&self) -> Vec<(&str, Option<&str>)> {
        self.settings
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_deref()))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.settings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.settings.is_empty()
    }
}

/// Explicit consent supplied by the caller, never read from global flags by
/// the core functions below. Mirrors the scoped physical-mutation flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioConsent {
    /// Same as `--allow-physical-mutation`.
    #[serde(default)]
    pub master: bool,
    /// Same as `--allow-physical-data`.
    #[serde(default)]
    pub data: bool,
    /// Same as `--allow-physical-network`.
    #[serde(default)]
    pub network: bool,
}

impl ScenarioConsent {
    pub fn from_context(ctx: &CliContext) -> Self {
        Self {
            master: ctx.allow_physical_mutation,
            data: ctx.scoped_physical.data,
            network: ctx.scoped_physical.network,
        }
    }

    pub fn allows(self, risk: Risk) -> bool {
        match risk {
            Risk::Data => self.master || self.data,
            Risk::Network => self.master || self.network,
            _ => self.master,
        }
    }
}

/// A complete scenario request, suitable for JSON integration and dry runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioRequest {
    pub serial: String,
    pub scenario: DeviceScenario,
    #[serde(default)]
    pub consent: ScenarioConsent,
    #[serde(default)]
    pub dry_run: bool,
}

impl ScenarioRequest {
    /// Re-validate values that arrived through serde rather than `parse`.
    pub fn validate(&self) -> Result<()> {
        match &self.scenario {
            DeviceScenario::Locale(value) => {
                validate_locale(value)?;
            }
            DeviceScenario::FontScale(value) => {
                validate_font_scale(value)?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// What a scenario would do, before anything is dispatched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioPlan {
    pub serial: String,
    pub scenario: String,
    pub risk: String,
    pub operation: String,
    /// Session keys snapshotted before mutation, in restore order.
    pub restore_keys: Vec<String>,
    pub will_mutate: bool,
    pub dry_run: bool,
}

/// Result of a planned or applied scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioOutcome {
    pub plan: ScenarioPlan,
    pub applied: bool,
    pub warnings: Vec<String>,
}

impl ScenarioOutcome {
    pub fn dry_run(plan: ScenarioPlan) -> Self {
        Self {
            plan,
            applied: false,
            warnings: Vec::new(),
        }
    }
}

fn risk_name(risk: Risk) -> &'static str {
    match risk {
        Risk::Data => "data",
        Risk::Network => "network",
        Risk::AppControl => "app-control",
        Risk::Install => "install",
        Risk::Input => "input",
    }
}

/// Build a plan without touching a device. Pure: safe for dry-run previews.
pub fn plan_scenario(request: &ScenarioRequest) -> ScenarioPlan {
    request.scenario.plan(&request.serial, request.dry_run)
}

/// Dry-run preview. The returned outcome cannot mutate anything: the function
/// has no device, ADB or session handle, and always clears `will_mutate`.
pub fn scenario_dry_run(request: &ScenarioRequest) -> ScenarioOutcome {
    let mut plan = plan_scenario(request);
    plan.dry_run = true;
    plan.will_mutate = false;
    ScenarioOutcome::dry_run(plan)
}

/// Explicit risk gate for scenario mutations. Emulators are always allowed;
/// physical devices need the narrow flag for the scenario's risk or the
/// `--allow-physical-mutation` superset.
pub fn ensure_scenario_consent(
    device: &Device,
    scenario: &DeviceScenario,
    consent: ScenarioConsent,
) -> Result<()> {
    let risk = scenario.risk();
    if device.kind == DeviceKind::Emulator || consent.allows(risk) {
        return Ok(());
    }
    Err(ForgeError::new(
        ErrorCode::PhysicalDeviceMutationBlocked,
        format!(
            "{} would change physical device {} ({}); consent for the {} risk was not granted",
            scenario.operation(),
            device.serial,
            device.model.as_deref().unwrap_or("unknown model"),
            risk_name(risk),
        ),
    )
    .with_suggestion(format!("Grant {}", risk.flag()))
    .with_suggestion(
        "--allow-physical-mutation grants every physical-device risk; use it only when all are intended.",
    )
    .device(device.serial.clone()))
}

/// Apply a scenario through the existing prepared-state/network primitives.
/// Dry runs return a plan before any consent check, ADB call or session write.
pub async fn apply_device_scenario(
    ctx: &CliContext,
    session: &mut Session,
    device: &Device,
    request: &ScenarioRequest,
) -> Result<ScenarioOutcome> {
    let plan = plan_scenario(request);
    request.validate()?;
    if request.dry_run {
        return Ok(ScenarioOutcome::dry_run(plan));
    }
    ensure_scenario_consent(device, &request.scenario, request.consent)?;
    match &request.scenario {
        DeviceScenario::NightMode(mode) => {
            let preparer = DevicePreparer::new(ctx.adb()?);
            let change = preparer
                .set_night_mode(&request.serial, mode.as_str(), session, &ctx.paths)
                .await?;
            if !change.applied {
                return Ok(ScenarioOutcome {
                    plan,
                    applied: false,
                    warnings: vec![format!(
                        "night mode was already {}; no change recorded",
                        mode.as_str()
                    )],
                });
            }
        }
        DeviceScenario::Locale(_) | DeviceScenario::FontScale(_) => {
            let options = request
                .scenario
                .prepare_options()
                .ok_or_else(|| ForgeError::invalid_argument("scenario needs prepared settings"))?;
            let preparer = DevicePreparer::new(ctx.adb()?);
            let result = preparer
                .prepare(&request.serial, &options, session, &ctx.paths)
                .await?;
            session.add_note(
                &ctx.paths,
                format!(
                    "device scenario {} applied on {}",
                    plan.scenario, request.serial
                ),
            )?;
            return Ok(ScenarioOutcome {
                plan,
                applied: result.prepared,
                warnings: result.warnings,
            });
        }
        DeviceScenario::Network(mode) => {
            return crate::commands::system::apply_network_scenario(
                ctx,
                session,
                device,
                *mode,
                request.consent,
                false,
            )
            .await;
        }
    }
    session.add_note(
        &ctx.paths,
        format!(
            "device scenario {} applied on {}",
            plan.scenario, request.serial
        ),
    )?;
    Ok(ScenarioOutcome {
        plan,
        applied: true,
        warnings: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Host-side device tags (issue #3 item 15)
//
// Tags are local labels attached to a serial in a small JSON manifest under the
// existing state directory. They never touch a device: the typed API below
// takes only a filesystem root, so it cannot call ADB even by accident. The
// `tag:<name>` target expansion is meant for matrix planning, which performs
// its own device checks after resolution. CLI/MCP flags land in a later wave.
// ---------------------------------------------------------------------------

pub const TAG_STORE_FILE: &str = "device-tags.json";
pub const TAG_STORE_SCHEMA_VERSION: u32 = 1;
pub const MAX_TAG_LEN: usize = 64;
pub const MAX_SERIAL_LEN: usize = 128;
pub const MAX_TAGS_PER_SERIAL: usize = 64;
pub const MAX_STORE_BYTES: u64 = 1024 * 1024;
pub const TAG_TARGET_PREFIX: &str = "tag:";

fn tag_schema_version() -> u32 {
    TAG_STORE_SCHEMA_VERSION
}

/// Validate a tag: 1-64 lowercase ASCII letters/digits/`.`/`_`/`-`.
/// Uppercase, whitespace, path separators, shell metacharacters and the dot
/// entries are refused so a tag can never be mistaken for a path or command.
pub fn validate_tag(tag: &str) -> Result<&str> {
    let reject = || {
        ForgeError::invalid_argument(format!(
            "device tag '{tag}' must be 1-{MAX_TAG_LEN} lowercase ASCII characters using only a-z, 0-9, '.', '_' or '-'"
        ))
        .with_suggestion("Use a simple tag such as 'pixel', 'nightly' or 'api-34'.")
    };
    if tag.is_empty() || tag.len() > MAX_TAG_LEN || tag == "." || tag == ".." {
        return Err(reject());
    }
    if !tag.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
    }) {
        return Err(reject());
    }
    Ok(tag)
}

/// Validate a serial used as a tag key: bounded, printable, no whitespace or
/// path separators. Colons and dots are allowed for wireless ADB serials.
pub fn validate_serial(serial: &str) -> Result<&str> {
    let reject = || {
        ForgeError::invalid_argument(format!(
            "device serial '{serial}' must be 1-{MAX_SERIAL_LEN} printable ASCII characters without whitespace or path separators"
        ))
        .with_suggestion("Use the exact serial shown by `androidforge devices list --json`.")
    };
    if serial.is_empty() || serial.len() > MAX_SERIAL_LEN {
        return Err(reject());
    }
    if !serial
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
    {
        return Err(reject());
    }
    Ok(serial)
}

/// The persisted tag manifest. Serials and tags are ordered collections, so
/// every list and resolution result is deterministic without extra sorting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceTagStore {
    #[serde(default = "tag_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub serials: BTreeMap<String, BTreeSet<String>>,
}

impl Default for DeviceTagStore {
    fn default() -> Self {
        Self {
            schema_version: TAG_STORE_SCHEMA_VERSION,
            serials: BTreeMap::new(),
        }
    }
}

impl DeviceTagStore {
    pub fn path(root: &Path) -> PathBuf {
        root.join(TAG_STORE_FILE)
    }

    /// Read the manifest. A missing file is an empty store, so `list` and tag
    /// resolution are safe before the first tag is added. Loaded data is
    /// re-validated: a hand-edited manifest cannot plant an unsafe token.
    pub fn load(root: &Path) -> Result<Self> {
        let path = Self::path(root);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(ForgeError::new(
                    ErrorCode::ConfigInvalid,
                    format!(
                        "could not read device tag store {}: {error}",
                        path.display()
                    ),
                ));
            }
        };
        if bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!(
                    "device tag store is {} bytes; the limit is {MAX_STORE_BYTES}",
                    bytes.len()
                ),
            ));
        }
        let store: Self = serde_json::from_slice(&bytes).map_err(|error| {
            ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!(
                    "could not parse device tag store {}: {error}",
                    path.display()
                ),
            )
        })?;
        if store.schema_version != TAG_STORE_SCHEMA_VERSION {
            return Err(ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!(
                    "device tag store version {} is unsupported (expected {TAG_STORE_SCHEMA_VERSION})",
                    store.schema_version
                ),
            ));
        }
        for (serial, tags) in &store.serials {
            validate_serial(serial)?;
            if tags.len() > MAX_TAGS_PER_SERIAL {
                return Err(ForgeError::new(
                    ErrorCode::ConfigInvalid,
                    format!(
                        "device {serial} has {} tags; the limit is {MAX_TAGS_PER_SERIAL}",
                        tags.len()
                    ),
                ));
            }
            for tag in tags {
                validate_tag(tag)?;
            }
        }
        Ok(store)
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        ensure_dir(root)?;
        write_json_atomic(&Self::path(root), self)
    }

    /// Tags for one serial in ascending order.
    pub fn tags_for(&self, serial: &str) -> Vec<String> {
        self.serials
            .get(serial)
            .map(|tags| tags.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Serials carrying a tag in ascending order. Pure: no I/O, no device.
    pub fn serials_for_tag(&self, tag: &str) -> Vec<String> {
        self.serials
            .iter()
            .filter(|(_, tags)| tags.contains(tag))
            .map(|(serial, _)| serial.clone())
            .collect()
    }

    /// Add a tag to a serial. Returns `false` when it was already present.
    pub fn add(&mut self, serial: &str, tag: &str) -> Result<bool> {
        let serial = validate_serial(serial)?;
        let tag = validate_tag(tag)?;
        let tags = self.serials.entry(serial.to_string()).or_default();
        if tags.contains(tag) {
            return Ok(false);
        }
        if tags.len() >= MAX_TAGS_PER_SERIAL {
            return Err(ForgeError::invalid_argument(format!(
                "device {serial} already has {MAX_TAGS_PER_SERIAL} tags"
            ))
            .with_suggestion("Remove an unused tag before adding another."));
        }
        tags.insert(tag.to_string());
        Ok(true)
    }

    /// Remove a tag from a serial. Returns `false` when it was not present.
    /// The serial key is dropped when its last tag is removed.
    pub fn remove(&mut self, serial: &str, tag: &str) -> Result<bool> {
        let serial = validate_serial(serial)?;
        let tag = validate_tag(tag)?;
        let Some(tags) = self.serials.get_mut(serial) else {
            return Ok(false);
        };
        let changed = tags.remove(tag);
        if tags.is_empty() {
            self.serials.remove(serial);
        }
        Ok(changed)
    }

    /// Remove every tag from a serial. Returns `false` when it had none.
    pub fn clear(&mut self, serial: &str) -> Result<bool> {
        let serial = validate_serial(serial)?;
        Ok(self.serials.remove(serial).is_some())
    }
}

/// One serial's tags. Used by `devices tag list` and by integration callers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceTagEntry {
    pub serial: String,
    pub tags: Vec<String>,
}

/// Result of a tag mutation, suitable for a `--json` response later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceTagChange {
    pub operation: String,
    pub serial: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub changed: bool,
    pub tags: Vec<String>,
    pub store_path: String,
}

/// Expansion of matrix device targets. `serials` is deduplicated and sorted;
/// `tags` records which serials each `tag:<name>` target resolved to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceTargetResolution {
    pub targets: Vec<String>,
    pub serials: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, Vec<String>>,
}

/// State-directory root for the tag store. Kept here so callers do not need a
/// new `ForgePaths` method in this workstream.
pub fn device_tag_root(paths: &ForgePaths) -> PathBuf {
    paths.state_dir.clone()
}

fn unknown_tag(tag: &str) -> ForgeError {
    ForgeError::new(
        ErrorCode::InvalidArgument,
        format!("no device is tagged '{tag}'"),
    )
    .with_context_string("tag", tag)
    .with_suggestion("Run `androidforge devices tag list --json` to see the known tags.")
}

/// Add a tag to a serial. Idempotent: adding an existing tag reports
/// `changed: false` and leaves the manifest untouched apart from a rewrite.
pub fn add_device_tag(root: &Path, serial: &str, tag: &str) -> Result<DeviceTagChange> {
    let serial = validate_serial(serial)?;
    let tag = validate_tag(tag)?;
    let mut store = DeviceTagStore::load(root)?;
    let changed = store.add(serial, tag)?;
    store.save(root)?;
    Ok(DeviceTagChange {
        operation: "add".to_string(),
        serial: serial.to_string(),
        tag: Some(tag.to_string()),
        changed,
        tags: store.tags_for(serial),
        store_path: DeviceTagStore::path(root).to_string_lossy().into_owned(),
    })
}

/// Remove a tag from a serial. Idempotent for an absent tag or serial.
pub fn remove_device_tag(root: &Path, serial: &str, tag: &str) -> Result<DeviceTagChange> {
    let serial = validate_serial(serial)?;
    let tag = validate_tag(tag)?;
    let mut store = DeviceTagStore::load(root)?;
    let changed = store.remove(serial, tag)?;
    store.save(root)?;
    Ok(DeviceTagChange {
        operation: "remove".to_string(),
        serial: serial.to_string(),
        tag: Some(tag.to_string()),
        changed,
        tags: store.tags_for(serial),
        store_path: DeviceTagStore::path(root).to_string_lossy().into_owned(),
    })
}

/// Clear all tags from a serial. Idempotent for a serial with no tags.
pub fn clear_device_tags(root: &Path, serial: &str) -> Result<DeviceTagChange> {
    let serial = validate_serial(serial)?;
    let mut store = DeviceTagStore::load(root)?;
    let changed = store.clear(serial)?;
    store.save(root)?;
    Ok(DeviceTagChange {
        operation: "clear".to_string(),
        serial: serial.to_string(),
        tag: None,
        changed,
        tags: store.tags_for(serial),
        store_path: DeviceTagStore::path(root).to_string_lossy().into_owned(),
    })
}

/// List tags for one serial, or every tagged serial when `serial` is `None`.
/// A serial with no tags is returned with an empty list rather than an error.
pub fn list_device_tags(root: &Path, serial: Option<&str>) -> Result<Vec<DeviceTagEntry>> {
    let store = DeviceTagStore::load(root)?;
    match serial {
        Some(serial) => {
            let serial = validate_serial(serial)?;
            Ok(vec![DeviceTagEntry {
                serial: serial.to_string(),
                tags: store.tags_for(serial),
            }])
        }
        None => Ok(store
            .serials
            .iter()
            .map(|(serial, tags)| DeviceTagEntry {
                serial: serial.clone(),
                tags: tags.iter().cloned().collect(),
            })
            .collect()),
    }
}

/// Resolve one `tag:<name>` to its serials. Never contacts a device.
pub fn resolve_tag_serials(root: &Path, tag: &str) -> Result<Vec<String>> {
    let tag = validate_tag(tag)?;
    let serials = DeviceTagStore::load(root)?.serials_for_tag(tag);
    if serials.is_empty() {
        return Err(unknown_tag(tag));
    }
    Ok(serials)
}

/// Expand matrix device targets to concrete serials. Targets starting with
/// `tag:` resolve through the host-side store; all others are literal serials.
/// Results are deduplicated and sorted, and an unknown tag is an error rather
/// than a silent empty expansion. No device or ADB handle is involved.
pub fn expand_device_targets(root: &Path, targets: &[String]) -> Result<DeviceTargetResolution> {
    let store = DeviceTagStore::load(root)?;
    let mut serials = BTreeSet::new();
    let mut tags = BTreeMap::new();
    for target in targets {
        if let Some(name) = target.strip_prefix(TAG_TARGET_PREFIX) {
            let name = validate_tag(name)?;
            let matched = store.serials_for_tag(name);
            if matched.is_empty() {
                return Err(unknown_tag(name));
            }
            tags.insert(name.to_string(), matched.clone());
            serials.extend(matched);
        } else {
            serials.insert(validate_serial(target)?.to_string());
        }
    }
    Ok(DeviceTargetResolution {
        targets: targets.to_vec(),
        serials: serials.into_iter().collect(),
        tags,
    })
}

#[cfg(test)]
mod scenario_tests {
    use androidforge_core::error::ErrorCode;
    use androidforge_core::model::{Device, DeviceKind, DeviceState};

    use super::*;

    fn physical_device() -> Device {
        let mut device = Device::basic("R58M123456A", DeviceState::Device);
        device.kind = DeviceKind::Physical;
        device.model = Some("Test Phone".to_string());
        device
    }

    #[test]
    fn scenario_requests_parse_all_supported_forms() {
        assert_eq!(
            DeviceScenario::parse("night:yes").unwrap(),
            DeviceScenario::NightMode(NightMode::Yes)
        );
        assert_eq!(
            DeviceScenario::parse("NIGHT=AUTO").unwrap(),
            DeviceScenario::NightMode(NightMode::Auto)
        );
        assert_eq!(
            DeviceScenario::parse("locale=en-US").unwrap(),
            DeviceScenario::Locale("en-US".to_string())
        );
        assert_eq!(
            DeviceScenario::parse("font:1.15").unwrap(),
            DeviceScenario::FontScale("1.15".to_string())
        );
        assert_eq!(
            DeviceScenario::parse("network:offline").unwrap(),
            DeviceScenario::Network(NetworkScenario::Offline)
        );
        assert_eq!(
            DeviceScenario::parse("network=wifi").unwrap(),
            DeviceScenario::Network(NetworkScenario::WifiOnly)
        );
        assert_eq!(NetworkScenario::Online.profile(), "default");
        assert_eq!(NetworkScenario::Offline.profile(), "offline");

        for invalid in ["", "night", "night:", "bogus=1", "locale=", "network=other"] {
            assert!(DeviceScenario::parse(invalid).is_err(), "{invalid}");
        }

        let json: ScenarioRequest = serde_json::from_str(
            r#"{"serial":"S","scenario":{"font-scale":"1.2"},"dry_run":true}"#,
        )
        .expect("parse JSON scenario request");
        assert_eq!(json.scenario, DeviceScenario::FontScale("1.2".to_string()));
        assert!(json.dry_run);
        assert_eq!(json.consent, ScenarioConsent::default());
    }

    #[test]
    fn locale_and_font_values_are_bounded_and_shell_safe() {
        assert_eq!(validate_locale("en-US").unwrap(), "en-US");
        assert_eq!(validate_locale("fr_FR").unwrap(), "fr-FR");
        assert_eq!(validate_locale("zh-Hant-TW").unwrap(), "zh-Hant-TW");
        for unsafe_locale in [
            "",
            "x",
            "en-US;rm -rf /",
            "../../etc",
            "en_US,fr_FR",
            &"a".repeat(64),
        ] {
            assert!(
                validate_locale(unsafe_locale).is_err(),
                "locale {unsafe_locale} must be rejected"
            );
        }

        assert_eq!(validate_font_scale("1.0").unwrap(), "1.0");
        assert_eq!(validate_font_scale("0.5").unwrap(), "0.5");
        assert_eq!(validate_font_scale("2.0").unwrap(), "2.0");
        for unsafe_scale in ["0.49", "2.01", "nan", "1..2", "1.2;cmd", "", "-1"] {
            assert!(
                validate_font_scale(unsafe_scale).is_err(),
                "font scale {unsafe_scale} must be rejected"
            );
        }
        assert!(
            DeviceScenario::parse("font:9").is_err(),
            "parsing must apply the same bounds"
        );
        let unsafe_request = ScenarioRequest {
            serial: "S".to_string(),
            scenario: DeviceScenario::FontScale("9.0".to_string()),
            consent: ScenarioConsent::default(),
            dry_run: false,
        };
        assert!(unsafe_request.validate().is_err());
    }

    #[test]
    fn restore_snapshot_keeps_first_value_and_session_key_order() {
        let mut snapshot = ScenarioSnapshot::default();
        assert!(snapshot.record("system:font_scale", Some("1.0".to_string())));
        assert!(snapshot.record("cmd:uimode_night", None));
        assert!(!snapshot.record("system:font_scale", Some("9.9".to_string())));
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot.get("system:font_scale"), Some(Some("1.0")));
        assert_eq!(snapshot.entries_in_restore_order()[0].1, None);
        assert_eq!(
            snapshot.keys_in_restore_order(),
            vec!["cmd:uimode_night", "system:font_scale"]
        );

        let network_plan = DeviceScenario::Network(NetworkScenario::Offline).plan("S", false);
        assert_eq!(
            network_plan.restore_keys,
            vec![
                "network:airplane",
                "network:data",
                "network:proxy",
                "network:wifi"
            ]
        );
        assert_eq!(network_plan.risk, "network");
        assert_eq!(
            DeviceScenario::NightMode(NightMode::Auto)
                .plan("S", false)
                .restore_keys,
            vec!["cmd:uimode_night"]
        );
    }

    #[test]
    fn dry_run_plan_never_mutates() {
        let request = ScenarioRequest {
            serial: "S".to_string(),
            scenario: DeviceScenario::parse("night:auto").unwrap(),
            // No consent, no device and no session are needed for a preview.
            consent: ScenarioConsent::default(),
            dry_run: false,
        };
        let outcome = scenario_dry_run(&request);
        assert!(!outcome.applied);
        assert!(outcome.plan.dry_run);
        assert!(!outcome.plan.will_mutate);
        assert!(outcome.warnings.is_empty());
        assert_eq!(outcome.plan.scenario, "night:auto");
        assert_eq!(outcome.plan.restore_keys, vec!["cmd:uimode_night"]);

        let session = Session::new(
            "tester",
            androidforge_core::session::SessionKind::Attached,
            Some("S".to_string()),
            None,
            None,
            false,
        );
        let before = session.device_settings.clone();
        let _ = scenario_dry_run(&request);
        assert_eq!(session.device_settings, before);
    }

    #[test]
    fn physical_mutations_require_matching_explicit_consent() {
        let device = physical_device();
        let locale = DeviceScenario::Locale("fr-FR".to_string());
        let network = DeviceScenario::Network(NetworkScenario::Offline);

        let error = ensure_scenario_consent(&device, &locale, ScenarioConsent::default())
            .expect_err("data scenario must be gated");
        assert_eq!(error.code(), ErrorCode::PhysicalDeviceMutationBlocked);
        assert!(
            error
                .body()
                .suggestions
                .iter()
                .any(|suggestion| suggestion.contains("--allow-physical-data")),
            "{error:?}"
        );
        assert_eq!(locale.risk(), Risk::Data);
        assert_eq!(network.risk(), Risk::Network);

        let data_only = ScenarioConsent {
            data: true,
            ..ScenarioConsent::default()
        };
        assert!(ensure_scenario_consent(&device, &locale, data_only).is_ok());
        assert!(ensure_scenario_consent(&device, &network, data_only).is_err());
        let network_only = ScenarioConsent {
            network: true,
            ..ScenarioConsent::default()
        };
        assert!(ensure_scenario_consent(&device, &locale, network_only).is_err());
        assert!(ensure_scenario_consent(&device, &network, network_only).is_ok());
        let master = ScenarioConsent {
            master: true,
            ..ScenarioConsent::default()
        };
        assert!(ensure_scenario_consent(&device, &locale, master).is_ok());
        assert!(ensure_scenario_consent(&device, &network, master).is_ok());

        let mut emulator = device;
        emulator.kind = DeviceKind::Emulator;
        assert!(ensure_scenario_consent(&emulator, &network, ScenarioConsent::default()).is_ok());
    }

    #[test]
    fn scenarios_map_onto_existing_primitives() {
        let night = DeviceScenario::NightMode(NightMode::Auto);
        assert_eq!(night.night_mode(), Some(NightMode::Auto));
        assert!(night.prepare_options().is_none());

        let locale = DeviceScenario::Locale("fr-FR".to_string());
        let options = locale.prepare_options().expect("locale uses prepare");
        assert_eq!(options.locale.as_deref(), Some("fr-FR"));
        assert!(!options.animations && !options.keep_awake && !options.wake);

        let font = DeviceScenario::FontScale("1.3".to_string());
        let options = font.prepare_options().expect("font uses prepare");
        assert_eq!(options.font_scale.as_deref(), Some("1.3"));
        assert!(!options.animations && !options.keep_awake && !options.wake);

        assert_eq!(
            DeviceScenario::parse("network=airplane").unwrap().network(),
            Some(NetworkScenario::Offline)
        );
    }
}

#[cfg(test)]
mod device_tag_tests {
    use androidforge_core::error::ErrorCode;

    use super::*;

    fn targets(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn add_remove_and_list_tags() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        let added = add_device_tag(root, "emulator-5554", "nightly").unwrap();
        assert_eq!(added.operation, "add");
        assert_eq!(added.serial, "emulator-5554");
        assert_eq!(added.tag.as_deref(), Some("nightly"));
        assert!(added.changed);
        assert_eq!(added.tags, vec!["nightly".to_string()]);
        assert_eq!(
            added.store_path,
            DeviceTagStore::path(root).to_string_lossy()
        );

        add_device_tag(root, "emulator-5554", "api-34").unwrap();
        add_device_tag(root, "R58M123456A", "nightly").unwrap();

        let one = list_device_tags(root, Some("emulator-5554")).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(
            one[0].tags,
            vec!["api-34".to_string(), "nightly".to_string()]
        );

        let all = list_device_tags(root, None).unwrap();
        assert_eq!(
            all,
            vec![
                DeviceTagEntry {
                    serial: "R58M123456A".to_string(),
                    tags: vec!["nightly".to_string()],
                },
                DeviceTagEntry {
                    serial: "emulator-5554".to_string(),
                    tags: vec!["api-34".to_string(), "nightly".to_string()],
                },
            ]
        );

        let removed = remove_device_tag(root, "emulator-5554", "api-34").unwrap();
        assert!(removed.changed);
        assert_eq!(removed.tags, vec!["nightly".to_string()]);
        let cleared = clear_device_tags(root, "emulator-5554").unwrap();
        assert!(cleared.changed);
        assert!(cleared.tags.is_empty());

        // A serial with no tags is still listable, and is gone from the full
        // listing because empty keys are not retained in the manifest.
        assert_eq!(
            list_device_tags(root, Some("emulator-5554")).unwrap()[0].tags,
            Vec::<String>::new()
        );
        assert_eq!(list_device_tags(root, None).unwrap().len(), 1);
    }

    #[test]
    fn duplicate_add_and_missing_removal_are_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        assert!(
            add_device_tag(root, "emulator-5554", "pixel")
                .unwrap()
                .changed
        );
        let repeat = add_device_tag(root, "emulator-5554", "pixel").unwrap();
        assert!(!repeat.changed);
        assert_eq!(repeat.tags, vec!["pixel".to_string()]);

        let missing = remove_device_tag(root, "emulator-5554", "absent").unwrap();
        assert!(!missing.changed);
        let absent_serial = remove_device_tag(root, "emulator-5556", "pixel").unwrap();
        assert!(!absent_serial.changed);
        assert!(!clear_device_tags(root, "emulator-5556").unwrap().changed);

        let store = DeviceTagStore::load(root).unwrap();
        assert_eq!(store.tags_for("emulator-5554"), vec!["pixel".to_string()]);
        assert!(!store.serials.contains_key("emulator-5556"));
    }

    #[test]
    fn invalid_tags_and_serials_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        for tag in [
            "",
            "UPPER",
            "has space",
            "a/b",
            "a\\b",
            "semi;colon",
            "$tag",
            "..",
            ".",
            "tab\there",
            "é",
        ] {
            assert!(validate_tag(tag).is_err(), "tag {tag:?} must be rejected");
            assert!(
                add_device_tag(root, "emulator-5554", tag).is_err(),
                "adding tag {tag:?} must fail"
            );
        }
        let too_long = "a".repeat(MAX_TAG_LEN + 1);
        assert!(validate_tag(&too_long).is_err());
        assert!(add_device_tag(root, "emulator-5554", &too_long).is_err());
        assert!(validate_tag(&"a".repeat(MAX_TAG_LEN)).is_ok());

        for serial in ["", "has space", "a/b", "a\\b", "x;y", "$(cmd)", "a\nb"] {
            assert!(
                validate_serial(serial).is_err(),
                "serial {serial:?} must be rejected"
            );
            assert!(add_device_tag(root, serial, "pixel").is_err());
        }
        assert!(validate_serial(&"s".repeat(MAX_SERIAL_LEN + 1)).is_err());
        for valid in ["emulator-5554", "R58M123456A", "192.168.1.20:37521"] {
            assert!(validate_serial(valid).is_ok(), "serial {valid}");
        }

        // No rejected input was persisted.
        assert!(!DeviceTagStore::path(root).exists());
    }

    #[test]
    fn tags_persist_across_reloads_and_reject_unsafe_manifests() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        add_device_tag(root, "emulator-5554", "nightly").unwrap();
        add_device_tag(root, "R58M123456A", "nightly").unwrap();

        let reloaded = DeviceTagStore::load(root).unwrap();
        assert_eq!(reloaded.schema_version, TAG_STORE_SCHEMA_VERSION);
        assert_eq!(
            reloaded.serials_for_tag("nightly"),
            vec!["R58M123456A", "emulator-5554"]
        );

        // A fresh root with no file is an empty store.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            DeviceTagStore::load(empty.path()).unwrap(),
            DeviceTagStore::default()
        );
        assert!(list_device_tags(empty.path(), None).unwrap().is_empty());

        fs::write(
            DeviceTagStore::path(root),
            r#"{"schema_version":1,"serials":{"emulator-5554":["../escape"]}}"#,
        )
        .unwrap();
        assert_eq!(
            DeviceTagStore::load(root).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );

        fs::write(
            DeviceTagStore::path(root),
            r#"{"schema_version":99,"serials":{}}"#,
        )
        .unwrap();
        assert_eq!(
            DeviceTagStore::load(root).unwrap_err().code(),
            ErrorCode::ConfigInvalid
        );
    }

    #[test]
    fn tag_targets_resolve_to_deterministic_unique_serials() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        add_device_tag(root, "emulator-5554", "nightly").unwrap();
        add_device_tag(root, "R58M123456A", "nightly").unwrap();
        add_device_tag(root, "R58M123456A", "pixel").unwrap();
        add_device_tag(root, "192.168.1.20:37521", "pixel").unwrap();

        assert_eq!(
            resolve_tag_serials(root, "nightly").unwrap(),
            vec!["R58M123456A".to_string(), "emulator-5554".to_string()]
        );

        // Two tags and a literal serial produce one sorted, deduplicated set.
        let resolution = expand_device_targets(
            root,
            &targets(&[
                "tag:pixel",
                "tag:nightly",
                "R58M123456A",
                "tag:pixel",
                "emulator-5556",
            ]),
        )
        .unwrap();
        assert_eq!(
            resolution.serials,
            vec![
                "192.168.1.20:37521".to_string(),
                "R58M123456A".to_string(),
                "emulator-5554".to_string(),
                "emulator-5556".to_string(),
            ]
        );
        assert_eq!(
            resolution.tags.get("nightly").unwrap(),
            &vec!["R58M123456A".to_string(), "emulator-5554".to_string()]
        );
        assert_eq!(
            resolution.tags.get("pixel").unwrap(),
            &vec!["192.168.1.20:37521".to_string(), "R58M123456A".to_string()]
        );
        assert_eq!(resolution.targets.len(), 5);
    }

    #[test]
    fn unknown_tag_is_an_error_not_an_empty_plan() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        add_device_tag(root, "emulator-5554", "pixel").unwrap();

        let error = resolve_tag_serials(root, "missing").unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
        assert!(error.message().contains("missing"));
        assert!(error.body().context.contains_key("tag"));

        let error =
            expand_device_targets(root, &targets(&["emulator-5554", "tag:missing"])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
        assert!(error.message().contains("missing"));

        let error = expand_device_targets(root, &targets(&["tag:UPPER"])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn tag_operations_only_touch_the_state_file() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        // The typed API takes only a filesystem root: no CliContext, ADB
        // client, toolchain or device can be reached from these functions.
        add_device_tag(root, "emulator-5554", "pixel").unwrap();
        let resolved = expand_device_targets(root, &targets(&["tag:pixel", "tag:pixel"])).unwrap();
        assert_eq!(resolved.serials, vec!["emulator-5554".to_string()]);

        let names = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, vec![TAG_STORE_FILE.to_string()]);
    }
}
