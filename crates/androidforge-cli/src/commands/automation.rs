use std::collections::BTreeMap;
use std::time::Duration;

use androidforge_automation::{
    FixtureAction, FixtureManifest, FixturePhase, FixtureRunOptions, FixtureStateExecution,
    FixtureStatus, apply_fixture_state, list_fixtures, load_fixture, plan_fixture_state,
    reset_fixture_state, run_conformance, run_fixture_with_data_policy, run_hook,
    run_instrumentation, run_maestro, validate_fixture,
};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::DeviceKind;
use androidforge_core::output::CommandSuccess;

use crate::cli::{
    ConformanceCommand, ConformanceRunArgs, FixtureCommand, FixtureRunArgs, FixtureStateArgs,
    HookCommand, TestCommand,
};
use crate::commands::common::success;
use crate::context::{CliContext, Risk};

pub async fn hook(ctx: &CliContext, command: &HookCommand) -> Result<CommandSuccess> {
    match command {
        HookCommand::List => {
            let hooks = ctx
                .config
                .file
                .hooks
                .iter()
                .map(|(name, hook)| {
                    serde_json::json!({
                        "name": name,
                        "kind": hook.kind,
                        "description": hook.description,
                        "idempotent": hook.idempotent,
                        "timeout_secs": hook.timeout_secs,
                    })
                })
                .collect::<Vec<_>>();
            let human = hooks
                .iter()
                .map(|hook| {
                    format!(
                        "{} ({}) - {}\n",
                        hook["name"].as_str().unwrap_or("-"),
                        serde_json::to_string(&hook["kind"]).unwrap_or_default(),
                        hook["description"].as_str().unwrap_or("")
                    )
                })
                .collect::<String>();
            Ok(success(
                "hook.list",
                serde_json::json!({"hooks": hooks}),
                if human.is_empty() {
                    "No hooks configured.\n".to_string()
                } else {
                    human
                },
            ))
        }
        HookCommand::Run { name } => {
            let session = ctx.require_session()?;
            let device = ctx.resolve_device(Some(&session)).await?;
            if !ctx.dry_run {
                ctx.ensure_risk_allowed(&device, Risk::Data, "running a project hook")?;
            }
            let name = name.clone();
            execute_in_run!(
                ctx,
                "hook.run",
                Some("hook run"),
                Some(&session),
                Some(&device),
                false,
                |run| async {
                    let adb = ctx.adb()?;
                    let result = run_hook(
                        &ctx.config,
                        &adb,
                        &session,
                        run,
                        &name,
                        ctx.trace,
                        ctx.dry_run,
                    )
                    .await?;
                    Ok(success(
                        "hook.run",
                        serde_json::to_value(&result)?,
                        format!(
                            "Hook {name}: {}\n",
                            if result.success {
                                "completed"
                            } else {
                                "failed"
                            }
                        ),
                    )
                    .with_session(session.id.clone())
                    .with_device(device.serial.clone()))
                },
            )
            .await
        }
    }
}

pub async fn fixture(ctx: &CliContext, command: &FixtureCommand) -> Result<CommandSuccess> {
    match command {
        FixtureCommand::List => {
            let fixtures = list_fixtures(&ctx.config)?;
            let human = fixtures
                .iter()
                .map(|fixture| {
                    format!(
                        "{} {} - {}\n",
                        if fixture.valid { "ok" } else { "invalid" },
                        fixture.name,
                        fixture.description.as_deref().unwrap_or("")
                    )
                })
                .collect::<String>();
            Ok(success(
                "fixture.list",
                serde_json::json!({"fixtures": fixtures}),
                if human.is_empty() {
                    "No fixtures found.\n".to_string()
                } else {
                    human
                },
            ))
        }
        FixtureCommand::Validate { name } => {
            let validation = validate_fixture(&ctx.config, name)?;
            let human = if validation.valid {
                format!("Fixture {} is valid.\n", validation.name)
            } else {
                format!(
                    "Fixture {} is invalid:\n{}\n",
                    validation.name,
                    validation.errors.join("\n")
                )
            };
            let mut success = success(
                "fixture.validate",
                serde_json::to_value(&validation)?,
                human,
            );
            if !validation.valid {
                success.exit_code = 8;
            }
            Ok(success)
        }
        FixtureCommand::Run(args) => fixture_run(ctx, args).await,
        FixtureCommand::Apply(args) => {
            fixture_state_only(ctx, args, FixtureStateAction::Apply).await
        }
        FixtureCommand::Reset(args) => {
            fixture_state_only(ctx, args, FixtureStateAction::Reset).await
        }
    }
}

async fn fixture_run(ctx: &CliContext, args: &FixtureRunArgs) -> Result<CommandSuccess> {
    let session = ctx.require_session()?;
    let device = ctx.resolve_device(Some(&session)).await?;
    if !ctx.dry_run {
        ctx.ensure_risk_allowed(&device, Risk::Data, "running a fixture")?;
    }
    // Core never infers consent from the device kind. Dry runs plan only, so no
    // data mutation capability is granted; real runs grant it only after the
    // physical-device gate above has accepted the emulator/flag decision.
    let allow_data_mutation = !ctx.dry_run
        && (matches!(device.kind, DeviceKind::Emulator) || ctx.risk_allowed(Risk::Data));
    let mut options = FixtureRunOptions::new(args.name.clone());
    for phase in &args.phase {
        options.phases.push(parse_phase(phase)?);
    }
    options.dry_run = ctx.dry_run;
    options.confirmed = ctx.yes;
    options.vars = parse_vars(&args.vars)?;
    options.apk = args.apk.as_ref().map(|path| {
        if path.is_absolute() {
            path.clone()
        } else {
            ctx.paths.cwd.join(path)
        }
    });
    options.trace = ctx.trace;
    execute_in_run!(
        ctx,
        "fixture.run",
        Some("fixture run"),
        Some(&session),
        Some(&device),
        false,
        |run| async {
            let app = ctx.app_manager()?;
            let ui = ctx.ui_manager()?;
            let result = run_fixture_with_data_policy(
                &ctx.config,
                &session,
                &app,
                &ui,
                &ctx.paths,
                run,
                &options,
                allow_data_mutation,
            )
            .await?;
            if result.status == FixtureStatus::Failed {
                return Err(ForgeError::new(
                    ErrorCode::FixtureFailed,
                    format!("fixture '{}' failed", result.name),
                )
                .with_context(
                    "fixture_result",
                    serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
                )
                .with_suggestion("Inspect the run report and failure artifacts.")
                .with_artifacts(result.artifacts.clone())
                .session(session.id.clone())
                .device(device.serial.clone()));
            }
            if result.status == FixtureStatus::Partial {
                let error = ForgeError::new(
                    ErrorCode::PartialFailure,
                    format!("fixture '{}' had partial failures", result.name),
                )
                .with_context(
                    "fixture_result",
                    serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
                )
                .with_artifacts(result.artifacts.clone())
                .with_suggestion(
                    "Inspect the run report for failed steps marked on_failure=continue.",
                )
                .session(session.id.clone());
                return Err(error);
            }
            let human = format!(
                "Fixture {} {}: {} step(s), {} warning(s).\n",
                result.name,
                if result.dry_run {
                    "planned"
                } else {
                    match result.status {
                        FixtureStatus::Success => "passed",
                        FixtureStatus::Planned => "planned",
                        FixtureStatus::Partial => "partially failed",
                        FixtureStatus::Failed => "failed",
                    }
                },
                result.steps.len(),
                result.warnings.len()
            );
            Ok(
                success("fixture.run", serde_json::to_value(&result)?, human)
                    .with_artifacts(result.artifacts)
                    .with_session(session.id.clone())
                    .with_device(device.serial.clone()),
            )
        },
    )
    .await
}

/// One-shot execution of only the declarative state actions of a fixture.
/// `fixture apply` runs `apply_state`; `fixture reset` runs `reset_state`. Other
/// fixture steps are deliberately never dispatched by these aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureStateAction {
    Apply,
    Reset,
}

impl FixtureStateAction {
    fn command(self) -> &'static str {
        match self {
            Self::Apply => "fixture.apply",
            Self::Reset => "fixture.reset",
        }
    }

    fn kind(self) -> &'static str {
        match self {
            Self::Apply => "apply_state",
            Self::Reset => "reset_state",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Apply => "fixture apply",
            Self::Reset => "fixture reset",
        }
    }

    fn operation(self) -> &'static str {
        match self {
            Self::Apply => "applying fixture declarative state",
            Self::Reset => "resetting fixture declarative state",
        }
    }
}

async fn fixture_state_only(
    ctx: &CliContext,
    args: &FixtureStateArgs,
    action: FixtureStateAction,
) -> Result<CommandSuccess> {
    let vars = parse_vars(&args.vars)?;
    if action == FixtureStateAction::Reset && !ctx.yes && !ctx.dry_run {
        return Err(ForgeError::invalid_argument(
            "fixture reset is destructive and was not confirmed",
        )
        .with_suggestion("Re-run with --yes after reviewing the fixture's declarative state."));
    }
    // Validate the manifest on the host before acquiring a session or device.
    let manifest = load_fixture(&ctx.config, &args.name)?;
    let name = manifest.name.clone();
    let source_path = manifest.source_path.clone().unwrap_or_default();
    let declared_package = state_package(&manifest, action);
    let package = args
        .package
        .clone()
        .or(declared_package)
        .or_else(|| ctx.config.debug_package().map(str::to_string))
        .or_else(|| ctx.config.package().map(str::to_string));

    if ctx.dry_run {
        let value = serde_json::json!({
            "dry_run": true,
            "name": name,
            "action": action.kind(),
            "fixture": source_path,
            "state_declared": manifest.state.is_some(),
            "package": package,
        });
        return Ok(success(
            action.command(),
            value,
            format!(
                "Would run {} of fixture '{}' (dry run).\n",
                action.kind(),
                name
            ),
        ));
    }

    let Some(package) = package else {
        return Err(ForgeError::new(
            ErrorCode::FixtureInvalid,
            format!(
                "{} needs a package and the project adapter has none",
                action.kind()
            ),
        )
        .with_suggestion("Set [project].package or pass --package."));
    };
    let session = ctx.require_session()?;
    let device = ctx.resolve_device(Some(&session)).await?;
    ctx.ensure_risk_allowed(&device, Risk::Data, action.operation())?;
    let allow_data_mutation =
        matches!(device.kind, DeviceKind::Emulator) || ctx.risk_allowed(Risk::Data);
    let serial = session.serial()?.to_string();
    let timeout = Duration::from_secs(ctx.config.file.timeouts.fixture_secs.max(1));
    let kind = action.kind();
    let command = action.command();
    let label = action.label();

    execute_in_run!(
        ctx,
        command,
        Some(label),
        Some(&session),
        Some(&device),
        false,
        |run| async {
            let manifest = load_fixture(&ctx.config, &args.name)?;
            let declared = manifest.state.as_ref().ok_or_else(|| {
                ForgeError::new(
                    ErrorCode::FixtureInvalid,
                    format!("fixture '{}' has no declarative state block", manifest.name),
                )
                .with_suggestion("Run `androidforge fixture validate <name>` to inspect the pack.")
            })?;
            let plan = plan_fixture_state(declared, &vars, &ctx.paths.project_root)?;
            let step = run.start_step(format!("fixture:{kind}"), kind)?;
            let app = ctx.app_manager()?;
            let execution = FixtureStateExecution {
                app: &app,
                serial: &serial,
                package: &package,
                cwd: &ctx.paths.project_root,
                timeout,
                trace: ctx.trace,
            };
            let outcome = match action {
                FixtureStateAction::Apply => {
                    apply_fixture_state(&execution, &plan, allow_data_mutation, run, step).await
                }
                FixtureStateAction::Reset => {
                    reset_fixture_state(&execution, &plan, allow_data_mutation, run, step).await
                }
            };
            let outcome = outcome.map_err(|error| {
                let _ = run.finish_step_failure(step, &error, Vec::new());
                error
                    .with_context_string("fixture", manifest.name.clone())
                    .with_context_string("action", kind.to_string())
            })?;
            let details = serde_json::to_value(&outcome).unwrap_or_default();
            run.finish_step_success(step, details.clone(), Vec::new())?;
            Ok(success(
                command,
                serde_json::json!({
                    "name": manifest.name,
                    "action": kind,
                    "outcome": details,
                    "dry_run": false,
                }),
                format!(
                    "Fixture '{}' {}: {} state path(s).\n",
                    manifest.name,
                    kind,
                    outcome.paths.len()
                ),
            )
            .with_session(session.id.clone())
            .with_device(device.serial.clone()))
        },
    )
    .await
}

fn state_package(manifest: &FixtureManifest, action: FixtureStateAction) -> Option<String> {
    manifest
        .steps
        .iter()
        .find_map(|step| match (&step.action, action) {
            (FixtureAction::ApplyState { package }, FixtureStateAction::Apply) => package.clone(),
            (FixtureAction::ResetState { package }, FixtureStateAction::Reset) => package.clone(),
            _ => None,
        })
}

pub async fn conformance(ctx: &CliContext, command: &ConformanceCommand) -> Result<CommandSuccess> {
    match command {
        ConformanceCommand::Run(args) => conformance_run(ctx, args).await,
    }
}

async fn conformance_run(ctx: &CliContext, args: &ConformanceRunArgs) -> Result<CommandSuccess> {
    if ctx.dry_run {
        return Ok(success(
            "conformance.run",
            serde_json::json!({"dry_run": true, "flow": args.flow}),
            "Would run the configured conformance flow.\n".to_string(),
        ));
    }
    let session = ctx.require_session()?;
    let device = ctx.resolve_device(Some(&session)).await?;
    ctx.ensure_risk_allowed(&device, Risk::Data, "running a conformance flow")?;
    let flow = args.flow.clone();
    execute_in_run!(
        ctx,
        "conformance.run",
        Some("conformance run"),
        Some(&session),
        Some(&device),
        false,
        |run| async {
            let app = ctx.app_manager()?;
            let ui = ctx.ui_manager()?;
            let result = run_conformance(
                &ctx.config,
                &session,
                &app,
                &ui,
                &ctx.paths,
                run,
                flow.as_deref(),
                ctx.yes,
                ctx.trace,
            )
            .await?;
            Ok(success(
                "conformance.run",
                serde_json::to_value(&result)?,
                format!(
                    "Conformance {} {} in {}ms. {} check(s).\n",
                    result.name,
                    if result.passed { "passed" } else { "failed" },
                    result.duration_ms,
                    result.checks.len()
                ),
            )
            .with_artifacts(result.artifacts)
            .with_session(session.id.clone())
            .with_device(device.serial.clone()))
        },
    )
    .await
}

pub async fn test(ctx: &CliContext, command: &TestCommand) -> Result<CommandSuccess> {
    match command {
        TestCommand::Maestro { flow, timeout } => {
            let session = ctx.require_session()?;
            let device = ctx.resolve_device(Some(&session)).await?;
            if ctx.dry_run {
                return Ok(success(
                    "test.maestro",
                    serde_json::json!({
                        "dry_run": true,
                        "flow": flow.to_string_lossy(),
                    }),
                    format!("Would run Maestro flow {}.\n", flow.display()),
                )
                .with_session(session.id.clone())
                .with_device(device.serial.clone()));
            }
            ctx.ensure_risk_allowed(&device, Risk::Data, "running a Maestro flow")?;
            let flow = if flow.is_absolute() {
                flow.clone()
            } else {
                ctx.paths.cwd.join(flow)
            };
            let timeout =
                ctx.parse_timeout(timeout.as_deref(), ctx.config.file.maestro.timeout_secs)?;
            execute_in_run!(
                ctx,
                "test.maestro",
                Some("test maestro"),
                Some(&session),
                Some(&device),
                false,
                |run| async {
                    let adb = ctx.adb()?;
                    let result =
                        run_maestro(&ctx.config, &adb, &session, run, &flow, timeout, ctx.trace)
                            .await?;
                    Ok(success(
                        "test.maestro",
                        serde_json::to_value(&result)?,
                        format!(
                            "Maestro flow {} in {}ms.\n",
                            if result.success { "passed" } else { "failed" },
                            result.duration_ms
                        ),
                    )
                    .with_artifacts(result.artifacts)
                    .with_session(session.id.clone())
                    .with_device(device.serial.clone()))
                },
            )
            .await
        }
        TestCommand::Instrumentation {
            package,
            runner,
            class,
            method,
            extras,
            timeout,
        } => {
            let session = ctx.require_session()?;
            let device = ctx.resolve_device(Some(&session)).await?;
            if ctx.dry_run {
                return Ok(success(
                    "test.instrumentation",
                    serde_json::json!({"dry_run": true, "package": package}),
                    format!("Would run instrumentation for {package}.\n"),
                )
                .with_session(session.id.clone())
                .with_device(device.serial.clone()));
            }
            ctx.ensure_risk_allowed(&device, Risk::Data, "running an instrumentation test")?;
            let package = package.clone();
            let runner = runner
                .clone()
                .unwrap_or_else(|| "androidx.test.runner.AndroidJUnitRunner".to_string());
            let mut extra_pairs = parse_extras(extras)?;
            if let Some(class) = class {
                extra_pairs.push(("class".to_string(), class.clone()));
            }
            if let Some(method) = method {
                extra_pairs.push(("method".to_string(), method.clone()));
            }
            let timeout =
                ctx.parse_timeout(timeout.as_deref(), ctx.config.file.timeouts.command_secs)?;
            execute_in_run!(
                ctx,
                "test.instrumentation",
                Some("test instrumentation"),
                Some(&session),
                Some(&device),
                false,
                |run| async {
                    let adb = ctx.adb()?;
                    let result = run_instrumentation(
                        &adb,
                        &session,
                        run,
                        &package,
                        &runner,
                        &extra_pairs,
                        timeout,
                        ctx.trace,
                    )
                    .await?;
                    Ok(success(
                        "test.instrumentation",
                        serde_json::to_value(&result)?,
                        format!(
                            "Instrumentation {} in {}ms.\n",
                            if result.success { "passed" } else { "failed" },
                            result.duration_ms
                        ),
                    )
                    .with_artifacts(result.artifacts)
                    .with_session(session.id.clone())
                    .with_device(device.serial.clone()))
                },
            )
            .await
        }
    }
}

fn parse_phase(value: &str) -> Result<FixturePhase> {
    match value {
        "pre-install" => Ok(FixturePhase::PreInstall),
        "post-install" => Ok(FixturePhase::PostInstall),
        "pre-test" => Ok(FixturePhase::PreTest),
        "post-test" => Ok(FixturePhase::PostTest),
        "cleanup" => Ok(FixturePhase::Cleanup),
        other => Err(
            ForgeError::invalid_argument(format!("unknown fixture phase '{other}'"))
                .with_suggestion("Use pre-install, post-install, pre-test, post-test or cleanup."),
        ),
    }
}

fn parse_vars(values: &[String]) -> Result<BTreeMap<String, String>> {
    let mut vars = BTreeMap::new();
    for value in values {
        let Some((key, value)) = value.split_once('=') else {
            return Err(ForgeError::invalid_argument(format!(
                "--var must use KEY=VALUE, got {value:?}"
            )));
        };
        vars.insert(key.to_string(), value.to_string());
    }
    Ok(vars)
}

fn parse_extras(values: &[String]) -> Result<Vec<(String, String)>> {
    let mut extras = Vec::new();
    for value in values {
        let Some((key, value)) = value.split_once('=') else {
            return Err(ForgeError::invalid_argument(format!(
                "--extra must use KEY=VALUE, got {value:?}"
            )));
        };
        extras.push((key.to_string(), value.to_string()));
    }
    Ok(extras)
}
