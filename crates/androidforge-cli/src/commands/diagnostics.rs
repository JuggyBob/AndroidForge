use std::path::Path;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::ArtifactRef;
use androidforge_core::output::CommandSuccess;
use androidforge_core::run::{RunContext, clear_active_run, set_active_run};
use androidforge_core::util::write_atomic;
use androidforge_diagnostics::{
    LogcatStream, Recorder, capture_screenshot, collect_diagnostics, collect_logcat_tail,
    detect_crashes,
};

use crate::cli::{
    CaptureCommand, CaptureScreenshotArgs, DiagnosticsCollectArgs, DiagnosticsCommand,
    LogcatCommand, LogcatStartArgs, LogcatTailArgs, RecordCommand, RecordStartArgs,
};
use crate::commands::common::success;
use crate::context::CliContext;
use crate::run_scope::RunScope;

pub async fn capture(ctx: &CliContext, command: &CaptureCommand) -> Result<CommandSuccess> {
    match command {
        CaptureCommand::Screenshot(args) => screenshot(ctx, args).await,
        CaptureCommand::Record { command } => record(ctx, command).await,
    }
}

async fn screenshot(ctx: &CliContext, args: &CaptureScreenshotArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let serial = device.serial.clone();
    let timeout = ctx.parse_timeout(
        args.timeout.as_deref(),
        ctx.config.file.timeouts.screenshot_secs,
    )?;
    let label = args.label.clone();
    let output = args.output.clone();
    execute_in_run!(
        ctx,
        "capture.screenshot",
        Some("capture screenshot"),
        session.as_ref(),
        Some(&device),
        true,
        |run| async {
            let adb = ctx.adb()?;
            let result = capture_screenshot(&adb, &serial, run, label.as_deref(), timeout).await?;
            let mut output_path = None;
            if let Some(output) = output {
                let path = local_output(ctx, &output);
                let bytes = std::fs::read(&result.local_path).map_err(|error| {
                    ForgeError::new(
                        ErrorCode::ArtifactWriteFailed,
                        format!("could not read captured screenshot: {error}"),
                    )
                })?;
                write_atomic(&path, &bytes)?;
                output_path = Some(path.to_string_lossy().into_owned());
            }
            let mut result_json = serde_json::to_value(&result)?;
            if let Some(object) = result_json.as_object_mut() {
                object.insert(
                    "path".to_string(),
                    serde_json::to_value(&result.artifact.path).unwrap_or(serde_json::Value::Null),
                );
                object.insert(
                    "output_path".to_string(),
                    serde_json::to_value(output_path).unwrap_or(serde_json::Value::Null),
                );
            }
            Ok(success(
                "capture.screenshot",
                result_json,
                format!(
                    "Captured {}x{} screenshot to {}\n",
                    result.width, result.height, result.artifact.path
                ),
            )
            .with_artifacts(vec![result.artifact])
            .with_optional_session(session.as_ref().map(|session| session.id.clone()))
            .with_device(serial))
        },
    )
    .await
}

async fn record(ctx: &CliContext, command: &RecordCommand) -> Result<CommandSuccess> {
    let session = ctx.require_session()?;
    let device = ctx.resolve_device(Some(&session)).await?;
    let recorder = Recorder::new(ctx.adb()?, ctx.paths.clone());
    match command {
        RecordCommand::Start(args) => record_start(ctx, &session, &device, &recorder, args).await,
        RecordCommand::Stop => record_stop(ctx, &session, &recorder).await,
        RecordCommand::Status => {
            let result = recorder.status(&session.id, &device.serial).await?;
            let human = match &result.recording {
                Some(recording) => format!(
                    "recording {} process_alive={} remote_bytes={:?} age={}ms\n",
                    recording.status,
                    result.local_process_alive,
                    result.remote_size_bytes,
                    result.age_ms
                ),
                None => "No active recording.\n".to_string(),
            };
            Ok(success(
                "capture.record.status",
                serde_json::to_value(&result)?,
                human,
            )
            .with_session(session.id)
            .with_device(device.serial))
        }
    }
}

async fn record_start(
    ctx: &CliContext,
    session: &androidforge_core::session::Session,
    device: &androidforge_core::model::Device,
    recorder: &Recorder,
    args: &RecordStartArgs,
) -> Result<CommandSuccess> {
    if ctx.dry_run {
        return Ok(success(
            "capture.record.start",
            serde_json::json!({"dry_run": true, "serial": device.serial, "time_limit": args.time_limit, "bit_rate": args.bit_rate, "size": args.size}),
            "Would start a screen recording.\n".to_string(),
        )
        .with_session(session.id.clone())
        .with_device(device.serial.clone()));
    }
    let mut scope = RunScope::begin(
        ctx,
        "capture.record.start",
        Some("record start"),
        Some(session),
        Some(device),
        true,
    )
    .await?;
    let time_limit = args
        .time_limit
        .unwrap_or(ctx.config.file.diagnostics.record_max_seconds);
    let bit_rate = args.bit_rate.unwrap_or(8_000_000);
    let size = args.size.clone();
    let owns_run = scope.implicit;
    let started = recorder
        .start(
            &session.id,
            &device.serial,
            &scope.run,
            time_limit,
            bit_rate,
            size,
            owns_run,
            ctx.trace,
        )
        .await;
    match started {
        Ok(state) => {
            let run_id =
                scope.finish_success_held(serde_json::json!({"recording": state}), Vec::new())?;
            if scope.implicit {
                set_active_run(
                    &ctx.paths,
                    Some(&session.id),
                    &run_id,
                    &ctx.paths.project_root,
                )?;
            }
            Ok(success(
                "capture.record.start",
                serde_json::to_value(&state)?,
                format!(
                    "Recording started for {}. Stop it with `androidforge capture record stop --session {}`.\n",
                    state.serial, session.id
                ),
            )
            .with_session(session.id.clone())
            .with_device(device.serial.clone())
            .with_run(run_id))
        }
        Err(error) => {
            let _ = scope.finish_failure(&error, Vec::new());
            Err(error)
        }
    }
}

async fn record_stop(
    ctx: &CliContext,
    session: &androidforge_core::session::Session,
    recorder: &Recorder,
) -> Result<CommandSuccess> {
    let Some(state) = recorder.load_state(&session.id)? else {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            "no recording is active for this session",
        )
        .with_suggestion("Run `androidforge capture record start` first.")
        .session(session.id.clone()));
    };
    let mut run = RunContext::open(&ctx.paths.artifacts_dir, &state.run_id, ctx.redactor())?;
    let step = run.start_step("capture.record.stop", "capture record stop")?;
    if ctx.dry_run {
        run.finish_step_skipped(step, "dry run")?;
        return Ok(success(
            "capture.record.stop",
            serde_json::json!({"dry_run": true, "run_id": state.run_id}),
            "Would stop the active recording.\n".to_string(),
        )
        .with_session(session.id.clone())
        .with_run(state.run_id));
    }
    let result = recorder.stop(&session.id, &mut run, ctx.trace).await;
    match result {
        Ok(outcome) => {
            let artifact = outcome.artifact.clone();
            run.finish_step_success(step, serde_json::to_value(&outcome)?, vec![artifact])?;
            if state.finalize_on_stop {
                run.finalize_pass()?;
                clear_active_run(&ctx.paths, Some(&session.id))?;
            } else {
                run.save()?;
            }
            Ok(success(
                "capture.record.stop",
                serde_json::to_value(&outcome)?,
                format!(
                    "Recording stopped: {} ({})\n",
                    outcome.local_path, outcome.size_bytes
                ),
            )
            .with_artifacts(vec![outcome.artifact])
            .with_session(session.id.clone())
            .with_device(state.serial)
            .with_run(state.run_id))
        }
        Err(error) => {
            let _ = run.finish_step_failure(step, &error, Vec::new());
            if state.finalize_on_stop {
                let _ = run.finalize_failure(&error);
            } else {
                let _ = run.save();
            }
            Err(error)
        }
    }
}

pub async fn diagnostics(ctx: &CliContext, command: &DiagnosticsCommand) -> Result<CommandSuccess> {
    match command {
        DiagnosticsCommand::Collect(args) => collect(ctx, args).await,
        DiagnosticsCommand::App { package } => {
            collect(
                ctx,
                &DiagnosticsCollectArgs {
                    package: Some(package.clone()),
                    full: true,
                    log_lines: Some(800),
                },
            )
            .await
        }
        DiagnosticsCommand::Logcat { command } => logcat(ctx, command).await,
        DiagnosticsCommand::Crashes { package } => crashes(ctx, package).await,
        DiagnosticsCommand::Inspect(args) => inspect(ctx, args).await,
    }
}

async fn collect(ctx: &CliContext, args: &DiagnosticsCollectArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let package = args
        .package
        .clone()
        .or_else(|| ctx.config.package().map(str::to_string));
    let lines = args
        .log_lines
        .unwrap_or(ctx.config.file.diagnostics.logcat_tail_lines);
    let max_log = ctx.config.file.diagnostics.max_log_bytes;
    let include_dumps = args.full || ctx.config.file.diagnostics.include_dumps;
    let serial = device.serial.clone();
    execute_in_run!(
        ctx,
        "diagnostics.collect",
        Some("diagnostics collect"),
        session.as_ref(),
        Some(&device),
        false,
        |run| async {
            let adb = ctx.adb()?;
            let ui = ctx.ui_manager()?;
            let app = ctx.app_manager()?;
            let result = collect_diagnostics(
                &adb,
                Some(&ui),
                Some(&app),
                run,
                &serial,
                package.as_deref(),
                lines,
                max_log,
                include_dumps,
            )
            .await?;
            let human = format!(
                "Collected {} artifact(s){} with {} warning(s).\n",
                result.artifacts.len(),
                package
                    .as_ref()
                    .map(|package| format!(" for {package}"))
                    .unwrap_or_default(),
                result.warnings.len()
            );
            let mut success = success("diagnostics.collect", serde_json::to_value(&result)?, human)
                .with_artifacts(result.artifacts)
                .with_optional_session(session.as_ref().map(|session| session.id.clone()))
                .with_device(serial.clone());
            if !result.warnings.is_empty() {
                success.exit_code = 10;
            }
            Ok(success)
        },
    )
    .await
}

async fn logcat(ctx: &CliContext, command: &LogcatCommand) -> Result<CommandSuccess> {
    match command {
        LogcatCommand::Tail(args) => logcat_tail(ctx, args).await,
        LogcatCommand::Start(args) => logcat_start(ctx, args).await,
        LogcatCommand::Stop => logcat_stop(ctx).await,
        LogcatCommand::Status => logcat_status(ctx).await,
    }
}

async fn logcat_tail(ctx: &CliContext, args: &LogcatTailArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let package = args
        .package
        .clone()
        .or_else(|| ctx.config.package().map(str::to_string));
    let serial = device.serial.clone();
    let lines = args
        .lines
        .unwrap_or(ctx.config.file.diagnostics.logcat_tail_lines);
    let max_bytes = ctx.config.file.diagnostics.max_log_bytes;
    let duration = args.duration.clone();
    execute_in_run!(
        ctx,
        "diagnostics.logcat",
        Some("diagnostics logcat"),
        session.as_ref(),
        Some(&device),
        true,
        |run| async {
            let adb = ctx.adb()?;
            let result = collect_logcat_tail(
                &adb,
                &serial,
                package.as_deref(),
                Some(lines),
                Some(max_bytes),
            )
            .await?;
            let raw = run.write_text_artifact(
                "logs/logcat-tail.txt",
                "logcat_raw",
                Some("One-shot logcat tail"),
                &result.raw,
            )?;
            let filtered = run.write_text_artifact(
                "logs/logcat-filtered.txt",
                "logcat_filtered",
                Some("Package/process-filtered logcat tail"),
                &result.filtered,
            )?;
            let mut artifacts = vec![raw, filtered];
            if !result.crash_buffer.trim().is_empty() {
                artifacts.push(run.write_text_artifact(
                    "logs/crash-buffer.txt",
                    "crash_buffer",
                    Some("Crash logcat buffer"),
                    &result.crash_buffer,
                )?);
            }
            let mut json = serde_json::to_value(&result)?;
            if let Some(object) = json.as_object_mut() {
                object.insert("raw_artifact".to_string(), serde_json::json!(&artifacts[0].path));
                object.insert("filtered_artifact".to_string(), serde_json::json!(&artifacts[1].path));
                if let Some(duration) = &duration {
                    object.insert(
                        "duration_note".to_string(),
                        serde_json::json!(format!(
                            "duration {duration} requested; this command captures the current tail rather than a timed stream"
                        )),
                    );
                }
            }
            Ok(success(
                "diagnostics.logcat",
                json,
                result.preview.clone(),
            )
            .with_artifacts(artifacts)
            .with_optional_session(session.as_ref().map(|session| session.id.clone()))
            .with_device(serial.clone()))
        },
    )
    .await
}

async fn logcat_start(ctx: &CliContext, args: &LogcatStartArgs) -> Result<CommandSuccess> {
    let session = ctx.require_session()?;
    let device = ctx.resolve_device(Some(&session)).await?;
    if ctx.dry_run {
        return Ok(success(
            "diagnostics.logcat.start",
            serde_json::json!({"dry_run": true, "serial": device.serial}),
            "Would start continuous logcat capture.\n".to_string(),
        )
        .with_session(session.id)
        .with_device(device.serial));
    }
    let stream = LogcatStream::new(ctx.adb()?, ctx.paths.clone());
    let mut scope = RunScope::begin(
        ctx,
        "diagnostics.logcat.start",
        Some("logcat start"),
        Some(&session),
        Some(&device),
        true,
    )
    .await?;
    let result = stream
        .start(
            &session.id,
            &device.serial,
            args.package.as_deref().or(ctx.config.package()),
            &scope.run,
            ctx.trace,
            ctx.config.file.diagnostics.logcat_max_seconds,
            ctx.config.file.diagnostics.max_log_bytes,
        )
        .await;
    match result {
        Ok(state) => {
            let run_id =
                scope.finish_success_held(serde_json::json!({"capture": state}), Vec::new())?;
            if scope.implicit {
                set_active_run(
                    &ctx.paths,
                    Some(&session.id),
                    &run_id,
                    &ctx.paths.project_root,
                )?;
            }
            Ok(success(
                "diagnostics.logcat.start",
                serde_json::to_value(&state)?,
                format!("Logcat capture started for {}. Stop it with `androidforge diagnostics logcat stop --session {}`.\n", state.serial, session.id),
            )
            .with_session(session.id)
            .with_device(device.serial)
            .with_run(run_id))
        }
        Err(error) => {
            let _ = scope.finish_failure(&error, Vec::new());
            Err(error)
        }
    }
}

async fn logcat_stop(ctx: &CliContext) -> Result<CommandSuccess> {
    let session = ctx.require_session()?;
    let stream = LogcatStream::new(ctx.adb()?, ctx.paths.clone());
    let Some(state) = stream.load(&session.id)? else {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            "no logcat capture is active for this session",
        )
        .session(session.id.clone()));
    };
    let mut run = RunContext::open(&ctx.paths.artifacts_dir, &state.run_id, ctx.redactor())?;
    let step = run.start_step("diagnostics.logcat.stop", "diagnostics logcat stop")?;
    if ctx.dry_run {
        run.finish_step_skipped(step, "dry run")?;
        return Ok(success(
            "diagnostics.logcat.stop",
            serde_json::json!({"dry_run": true, "run_id": state.run_id}),
            "Would stop logcat capture.\n".to_string(),
        )
        .with_session(session.id)
        .with_run(state.run_id));
    }
    let result = stream.stop(&session.id, &mut run).await;
    match result {
        Ok(outcome) => {
            let artifacts = outcome_artifacts(
                &run,
                &[&outcome.raw_artifact, &outcome.filtered_artifact],
                outcome.crash_artifact.as_deref(),
            );
            run.finish_step_success(step, serde_json::to_value(&outcome)?, artifacts.clone())?;
            run.finalize_pass()?;
            clear_active_run(&ctx.paths, Some(&session.id))?;
            Ok(success(
                "diagnostics.logcat.stop",
                serde_json::to_value(&outcome)?,
                format!(
                    "Logcat capture stopped: {} raw byte(s), {} filtered byte(s).\n",
                    outcome.raw_bytes, outcome.filtered_bytes
                ),
            )
            .with_artifacts(artifacts)
            .with_session(session.id)
            .with_device(state.serial)
            .with_run(state.run_id))
        }
        Err(error) => {
            let _ = run.finish_step_failure(step, &error, Vec::new());
            let _ = run.finalize_failure(&error);
            Err(error)
        }
    }
}

async fn logcat_status(ctx: &CliContext) -> Result<CommandSuccess> {
    let session = ctx.require_session()?;
    let stream = LogcatStream::new(ctx.adb()?, ctx.paths.clone());
    let status = stream.status(&session.id).await?;
    Ok(success(
        "diagnostics.logcat.status",
        serde_json::to_value(&status)?,
        match &status.capture {
            Some(capture) => format!(
                "logcat {} process_alive={} bytes={:?} age={}ms\n",
                capture.status, status.process_alive, status.local_size_bytes, status.age_ms
            ),
            None => "No active logcat capture.\n".to_string(),
        },
    )
    .with_session(session.id))
}

async fn crashes(ctx: &CliContext, package: &str) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let serial = device.serial.clone();
    let package = package.to_string();
    execute_in_run!(
        ctx,
        "diagnostics.crashes",
        Some("diagnostics crashes"),
        session.as_ref(),
        Some(&device),
        true,
        |run| async {
            let adb = ctx.adb()?;
            let report = detect_crashes(&adb, &serial, &package).await?;
            let artifact = run.write_text_artifact(
                "diagnostics/crashes.json",
                "crash_report",
                Some("Crash/ANR/process-death report"),
                &serde_json::to_string_pretty(&report).unwrap_or_default(),
            )?;
            Ok(success(
                "diagnostics.crashes",
                serde_json::to_value(&report)?,
                format!(
                    "{} crash/ANR event(s) for {package} (best effort).\n",
                    report.events.len()
                ),
            )
            .with_artifacts(vec![artifact])
            .with_optional_session(session.as_ref().map(|session| session.id.clone()))
            .with_device(serial.clone()))
        },
    )
    .await
}

async fn inspect(
    ctx: &CliContext,
    args: &crate::cli::DiagnosticsInspectArgs,
) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let service = match args.service.as_str() {
        "jobs" | "jobscheduler" => "jobscheduler",
        "alarms" | "alarm" => "alarm",
        "notifications" | "notification" => "notification",
        "activity" => "activity",
        "package" => "package",
        "deviceidle" => "deviceidle",
        "battery" => "battery",
        "connectivity" => "connectivity",
        other => other,
    };
    let serial = device.serial.clone();
    execute_in_run!(
        ctx,
        "diagnostics.inspect",
        Some("diagnostics inspect"),
        session.as_ref(),
        Some(&device),
        true,
        |run| async {
            let adb = ctx.adb()?;
            let text = adb.dumpsys(&serial, service).await?;
            let artifact = run.write_text_artifact(
                &format!("diagnostics/{service}.txt"),
                "dumpsys",
                Some(service),
                &text,
            )?;
            let preview = text.lines().take(80).collect::<Vec<_>>().join("\n");
            Ok(success(
                "diagnostics.inspect",
                serde_json::json!({
                    "service": service,
                    "path": artifact.path,
                    "bytes": artifact.size_bytes,
                    "preview": preview,
                    "note": "raw dumpsys output is preserved; structured parsers are used by dedicated commands where available"
                }),
                format!("Captured dumpsys {service} to {}\n", artifact.path),
            )
            .with_artifacts(vec![artifact])
            .with_optional_session(session.as_ref().map(|session| session.id.clone()))
            .with_device(serial.clone()))
        },
    )
    .await
}

fn outcome_artifacts(run: &RunContext, paths: &[&str], crash: Option<&str>) -> Vec<ArtifactRef> {
    let mut paths = paths.to_vec();
    if let Some(crash) = crash {
        paths.push(crash);
    }
    run.report
        .artifacts
        .iter()
        .filter(|artifact| paths.contains(&artifact.path.as_str()))
        .cloned()
        .collect()
}

fn local_output(ctx: &CliContext, output: &Path) -> std::path::PathBuf {
    if output.is_absolute() {
        output.to_path_buf()
    } else {
        ctx.paths.cwd.join(output)
    }
}
