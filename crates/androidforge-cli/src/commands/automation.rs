use std::collections::BTreeMap;

use androidforge_automation::{
    FixturePhase, FixtureRunOptions, FixtureStatus, list_fixtures, run_conformance, run_fixture,
    run_hook, run_instrumentation, run_maestro, validate_fixture,
};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::output::CommandSuccess;

use crate::cli::{
    ConformanceCommand, ConformanceRunArgs, FixtureCommand, FixtureRunArgs, HookCommand,
    TestCommand,
};
use crate::commands::common::success;
use crate::context::CliContext;

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
    }
}

async fn fixture_run(ctx: &CliContext, args: &FixtureRunArgs) -> Result<CommandSuccess> {
    let session = ctx.require_session()?;
    let device = ctx.resolve_device(Some(&session)).await?;
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
            let result =
                run_fixture(&ctx.config, &session, &app, &ui, &ctx.paths, run, &options).await?;
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
