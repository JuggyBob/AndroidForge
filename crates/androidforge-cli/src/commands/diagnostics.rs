use std::io::{Cursor, Read};
use std::path::Path;
use std::time::Duration;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::ArtifactRef;
use androidforge_core::output::CommandSuccess;
use androidforge_core::redact::Redactor;
use androidforge_core::retry::ReadOnlyOperation;
use androidforge_core::run::{RunContext, clear_active_run, set_active_run};
use androidforge_core::util::{
    now_rfc3339, parse_duration, read_json, sha256_hex, write_atomic, write_json_atomic,
};
use androidforge_diagnostics::{
    AdbLaunchFrameSource, CaptureLaunchOptions, ColorSampleKind, DEFAULT_DURATION_MS, DEFAULT_FPS,
    DEFAULT_MIN_FRAMES, DEFAULT_TOLERANCE, FirstFrameExpectation, FramesDeltaStatus, FramesReport,
    FramesThresholds, LaunchArtifactSink, LaunchCancellation, LogcatRecord, LogcatResult,
    LogcatStream, PerformanceOptions, PerformanceReport, Recorder, assert_first_frame,
    capture_frame, capture_screenshot, collect_diagnostics, collect_logcat_peek,
    collect_logcat_tail, collect_performance, compare_frames_reports, detect_crashes,
    frames_report_from_gfxinfo, logcat_line_matches_package, parse_threadtime_line,
    sample_launch_capture,
};
use androidforge_ui::a11y as ui_a11y;
use androidforge_ui::color as ui_color;

use crate::cli::{
    BaselineCommand, BaselineCompareArgs, BaselineSaveArgs, CaptureAssertColorArgs, CaptureCommand,
    CaptureContrastArgs, CaptureFrameArgs, CaptureLaunchArgs, CaptureScreenshotArgs,
    DiagnosticsCollectArgs, DiagnosticsCommand, DiagnosticsFramesArgs, DiagnosticsPerformanceArgs,
    LogcatCommand, LogcatPeekArgs, LogcatQueryArgs, LogcatStartArgs, LogcatTailArgs,
    LogcatWatchArgs, RecordCommand, RecordStartArgs,
};
use crate::commands::common::{self, success};
use crate::context::{CliContext, Risk};
use crate::run_scope::RunScope;
use serde_json::json;

pub async fn capture(ctx: &CliContext, command: &CaptureCommand) -> Result<CommandSuccess> {
    match command {
        CaptureCommand::Screenshot(args) => screenshot(ctx, args).await,
        CaptureCommand::Frame(args) => frame(ctx, args).await,
        CaptureCommand::Launch(args) => launch(ctx, args).await,
        CaptureCommand::AssertColor(args) => assert_color(ctx, args).await,
        CaptureCommand::Baseline { command } => baseline(ctx, command).await,
        CaptureCommand::Contrast(args) => capture_contrast(ctx, args).await,
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
            let mut comparison = None;
            let mut comparison_artifact = None;
            if let Some(baseline) = &args.compare {
                let baseline_path = resolve_input_path(ctx, baseline);
                let region = args
                    .region
                    .as_deref()
                    .map(crate::commands::visual::parse_region)
                    .transpose()?;
                let diff_output = args
                    .diff_output
                    .as_ref()
                    .map(|path| local_output(ctx, path))
                    .unwrap_or_else(|| run.dir.join("visual").join("diff.png"));
                let diff = crate::commands::visual::compare_png(
                    &baseline_path,
                    Path::new(&result.local_path),
                    region,
                    args.diff_threshold,
                    args.max_diff_ratio,
                    Some(&diff_output),
                )?;
                comparison_artifact = Some(run.register_artifact(
                    &diff_output,
                    "visual_diff",
                    Some("Visual regression diff visualization"),
                )?);
                comparison = Some(diff);
            }
            let mut result_json = serde_json::to_value(&result)?;
            if let Some(object) = result_json.as_object_mut() {
                object.insert(
                    "path".to_string(),
                    serde_json::to_value(&result.artifact.path).unwrap_or(serde_json::Value::Null),
                );
                object.insert(
                    "output_path".to_string(),
                    serde_json::to_value(&output_path).unwrap_or(serde_json::Value::Null),
                );
                object.insert(
                    "comparison".to_string(),
                    serde_json::to_value(&comparison).unwrap_or(serde_json::Value::Null),
                );
            }
            let mut artifacts = vec![result.artifact.clone()];
            if let Some(artifact) = comparison_artifact.clone() {
                artifacts.push(artifact);
            }
            if let Some(diff) = &comparison
                && !diff.passed
            {
                return Err(ForgeError::new(
                    ErrorCode::UiAssertionFailed,
                    format!(
                        "screenshot differs from baseline: {:.4}% changed pixels (limit {:.4}%)",
                        diff.changed_ratio * 100.0,
                        diff.max_diff_ratio * 100.0
                    ),
                )
                .with_context(
                    "comparison",
                    serde_json::to_value(diff).unwrap_or(serde_json::Value::Null),
                )
                .with_suggestion("Inspect the diff image, then update the baseline only if the change is intended.")
                .with_artifacts(artifacts)
                .device(serial.to_string()));
            }
            let human = match &comparison {
                Some(diff) => format!(
                    "Captured {}x{} screenshot to {}\nvisual diff: {} changed pixel(s) of {} ({:.4}%), passed={}\n",
                    result.width,
                    result.height,
                    result.artifact.path,
                    diff.changed_pixels,
                    diff.compared_pixels,
                    diff.changed_ratio * 100.0,
                    diff.passed
                ),
                None => format!(
                    "Captured {}x{} screenshot to {}\n",
                    result.width, result.height, result.artifact.path
                ),
            };
            Ok(success("capture.screenshot", result_json, human)
                .with_artifacts(artifacts)
                .with_optional_session(session.as_ref().map(|session| session.id.clone()))
                .with_device(serial))
        },
    )
    .await
}

async fn frame(ctx: &CliContext, args: &CaptureFrameArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let serial = device.serial.clone();
    let timeout = ctx.parse_timeout(
        args.timeout.as_deref(),
        ctx.config.file.timeouts.screenshot_secs,
    )?;
    let output = local_output(ctx, &args.output);
    let adb = ctx.adb()?;
    let result = capture_frame(&adb, &serial, &output, timeout).await?;
    Ok(success(
        "capture.frame",
        serde_json::to_value(&result)?,
        format!(
            "Captured preview frame {}x{} to {}\n",
            result.width, result.height, result.path
        ),
    )
    .with_optional_session(session.as_ref().map(|session| session.id.clone()))
    .with_device(serial))
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
        DiagnosticsCommand::Performance(args) => performance(ctx, args).await,
        DiagnosticsCommand::Frames(args) => frames(ctx, args).await,
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

async fn performance(
    ctx: &CliContext,
    args: &DiagnosticsPerformanceArgs,
) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let package = ctx.package_or_config(args.package.as_deref())?;
    let serial = device.serial.clone();
    let settle = ctx.parse_timeout(args.settle.as_deref(), 2)?;
    let timeout = ctx.parse_timeout(
        args.timeout.as_deref(),
        ctx.config.file.timeouts.app_launch_secs,
    )?;
    let frame_window = args
        .frame_window
        .as_deref()
        .map(parse_duration)
        .transpose()
        .map_err(ForgeError::invalid_argument)?;
    let warmup = args
        .warmup
        .as_deref()
        .map(parse_duration)
        .transpose()
        .map_err(ForgeError::invalid_argument)?;
    // Screen preflight is on by default. Waking is only ever an explicit
    // opt-in and is never inferred from device type or `--cold`.
    let require_screen_on = !args.no_screen_check;
    if ctx.dry_run {
        return Ok(success(
            "diagnostics.performance",
            serde_json::json!({
                "dry_run": true,
                "serial": serial,
                "package": package,
                "activity": args.activity,
                "cold": args.cold,
                "settle_ms": settle.as_millis() as u64,
                "wake": args.wake,
                "require_screen_on": require_screen_on,
                "min_frames": args.min_frames,
                "frame_window_ms": frame_window.map(|window| window.as_millis() as u64),
                "warmup_ms": warmup.map(|warmup| warmup.as_millis() as u64),
                "would_collect": ["startup", "jank", "memory", "cpu"],
                "note": if args.cold {
                    "Would force-stop the application before measuring cold startup."
                } else {
                    "Would measure the currently running process without force-stopping it."
                },
            }),
            format!(
                "Would measure {} on {}{} and write performance/report.json\n",
                package,
                serial,
                if args.cold {
                    " after an explicit cold-start force-stop"
                } else {
                    ""
                }
            ),
        )
        .with_optional_session(session.as_ref().map(|session| session.id.clone()))
        .with_device(serial));
    }
    // Measuring startup deliberately launches the app (`am start -W`), and
    // `--cold` force-stops it first. Physical devices therefore need the same
    // app-control consent as an explicit `capture launch` or `app launch`.
    // Dry runs return above and never reach this gate or any state-changing
    // ADB call; emulators are allowed by `ensure_risk_allowed`.
    ctx.ensure_risk_allowed(
        &device,
        Risk::AppControl,
        "collecting startup performance (launches the app)",
    )?;
    execute_in_run!(
        ctx,
        "diagnostics.performance",
        Some("diagnostics performance"),
        session.as_ref(),
        Some(&device),
        true,
        |run| async {
            let adb = ctx.adb()?;
            let report = collect_performance(
                &adb,
                &serial,
                &PerformanceOptions {
                    package: &package,
                    activity: args.activity.as_deref(),
                    cold: args.cold,
                    settle,
                    timeout,
                    require_screen_on,
                    wake: args.wake,
                    min_frames: args.min_frames,
                    frame_window,
                    warmup,
                },
            )
            .await?;
            let json = serde_json::to_string_pretty(&report)?;
            let artifact = run.write_text_artifact(
                "performance/report.json",
                "performance_report",
                Some("Startup, jank, memory and CPU evidence"),
                &json,
            )?;
            let mut result = serde_json::to_value(&report)?;
            if let Some(object) = result.as_object_mut() {
                object.insert("run_id".to_string(), serde_json::json!(run.run_id.clone()));
                object.insert(
                    "run_dir".to_string(),
                    serde_json::json!(run.dir.to_string_lossy()),
                );
                object.insert(
                    "artifacts".to_string(),
                    serde_json::to_value(vec![&artifact])?,
                );
            }
            let mut success = success(
                "diagnostics.performance",
                result,
                performance_human(&report),
            )
            .with_artifacts(vec![artifact])
            .with_optional_session(session.as_ref().map(|session| session.id.clone()))
            .with_device(serial.clone());
            if !report.warnings.is_empty() {
                success.exit_code = 10;
            }
            Ok(success)
        },
    )
    .await
}

fn performance_human(report: &PerformanceReport) -> String {
    let startup = match report.startup.total_time_ms {
        Some(total) => format!(
            "startup: {total}ms total{}{}",
            if report.startup.cold {
                " (cold)"
            } else {
                " (warm)"
            },
            report
                .startup
                .wait_time_ms
                .map(|wait| format!(", {wait}ms wait"))
                .unwrap_or_default()
        ),
        None => format!(
            "startup: unknown ({})",
            report
                .startup
                .unknown_reason
                .as_deref()
                .unwrap_or("no measurement")
        ),
    };
    let jank = if report.jank.sample_status == "insufficient" {
        // Never render a percentage derived from too few frames: one janky
        // frame out of one rendered frame must not look like 100.00% jank.
        format!(
            "jank: insufficient sample ({} frame(s))",
            report.jank.total_frames.unwrap_or(0)
        )
    } else {
        match (report.jank.total_frames, report.jank.janky_frames) {
            (Some(total), Some(janky)) => format!(
                "jank: {janky}/{total} frame(s){} with {} reset {}",
                report
                    .jank
                    .janky_percent
                    .map(|percent| format!(" ({percent:.2}%)"))
                    .unwrap_or_default(),
                if report.jank.reset_before_launch {
                    "counters"
                } else {
                    "counters not"
                },
                if report.jank.reset_before_launch {
                    "before launch"
                } else {
                    "(window unknown)"
                }
            ),
            _ => format!(
                "jank: {} ({})",
                report.jank.sample_status,
                report
                    .jank
                    .comparison_reason
                    .as_deref()
                    .or(report.jank.unknown_reason.as_deref())
                    .unwrap_or("no frame statistics")
            ),
        }
    };
    let memory = match (report.memory.total_pss_kb, report.memory.total_rss_kb) {
        (Some(pss), Some(rss)) => format!("memory: {pss} KiB PSS, {rss} KiB RSS"),
        (Some(pss), None) => format!("memory: {pss} KiB PSS"),
        (None, Some(rss)) => format!("memory: {rss} KiB RSS"),
        (None, None) => format!(
            "memory: unknown ({})",
            report
                .memory
                .unknown_reason
                .as_deref()
                .unwrap_or("no memory statistics")
        ),
    };
    let cpu = match report.cpu.total_percent {
        Some(total) => format!(
            "cpu: {total:.1}%{}{}",
            report
                .cpu
                .process
                .as_deref()
                .map(|process| format!(" in {process}"))
                .unwrap_or_default(),
            report
                .cpu
                .user_percent
                .zip(report.cpu.kernel_percent)
                .map(|(user, kernel)| format!(" ({user:.1}% user + {kernel:.1}% kernel)"))
                .unwrap_or_default()
        ),
        None => format!(
            "cpu: {} ({})",
            report.cpu.status,
            report
                .cpu
                .reason
                .as_deref()
                .or(report.cpu.unknown_reason.as_deref())
                .unwrap_or("no CPU statistics")
        ),
    };
    let screen = match report.screen_on {
        Some(true) => "screen: on",
        Some(false) => "screen: off",
        None => "screen: unknown",
    };
    let mut lines = vec![
        format!("Performance for {} on {}", report.package, report.serial),
        screen.to_string(),
        startup,
        jank,
        memory,
        cpu,
        format!(
            "sample quality: jank {}, cpu {}",
            report.sample_quality.jank.status, report.sample_quality.cpu.status
        ),
    ];
    if let Some(foreground) = report.foreground
        && !foreground
    {
        lines.push("warning: the measured package is not the current focus".to_string());
    }
    for warning in &report.warnings {
        lines.push(format!("warning: {warning}"));
    }
    lines.join("\n") + "\n"
}

fn frames_thresholds(args: &DiagnosticsFramesArgs) -> FramesThresholds {
    FramesThresholds {
        jank_percent_pp: args.max_janky.unwrap_or(0.0),
        p99_ms: args.max_p99.unwrap_or(0.0),
        max_ms: args.max_frame_ms.unwrap_or(0.0),
    }
}

fn validate_frames_thresholds(args: &DiagnosticsFramesArgs) -> Result<()> {
    for (flag, value) in [
        ("--max-janky", args.max_janky),
        ("--max-p99", args.max_p99),
        ("--max-frame-ms", args.max_frame_ms),
    ] {
        if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(ForgeError::invalid_argument(format!(
                "{flag} must be a finite, non-negative number"
            )));
        }
    }
    Ok(())
}

/// Exit contract for `diagnostics frames`: a measured regression is an error
/// (11); partial or non-comparable evidence is a warning (10).
fn frames_exit_code(report: &FramesReport) -> i32 {
    if report
        .comparison
        .as_ref()
        .is_some_and(|comparison| comparison.regression)
    {
        11
    } else if report.warnings.is_empty() {
        0
    } else {
        10
    }
}

fn frames_human(report: &FramesReport) -> String {
    let sample = &report.sample;
    let shown = |value: Option<f64>| {
        value
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    };
    let mut lines = vec![
        format!(
            "Frames for {} on {} (sample: {})",
            report.package,
            report.serial,
            if sample.sample_status.is_empty() {
                "unknown"
            } else {
                &sample.sample_status
            }
        ),
        format!(
            "total: {} janky: {} ({}) p99: {}ms max: {}ms",
            sample
                .total_frames
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            sample
                .janky
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            shown(
                sample
                    .jank_percent()
                    .map(|percent| (percent * 100.0).round() / 100.0)
            ),
            shown(sample.p99_ms),
            shown(sample.max_ms),
        ),
    ];
    if report.reset_before_collection {
        lines.push("gfxinfo counters were reset before collection".to_string());
    }
    if let Some(comparison) = &report.comparison {
        lines.push(format!(
            "baseline comparison: comparable={} regression={}",
            comparison.comparable, comparison.regression
        ));
        for delta in &comparison.deltas {
            let status = match delta.status {
                FramesDeltaStatus::Pass => "pass",
                FramesDeltaStatus::Regression => "regression",
                FramesDeltaStatus::Unknown => "unknown",
            };
            lines.push(format!(
                "  {}: {} -> {} (delta {}, {status})",
                delta.metric,
                shown(delta.baseline),
                shown(delta.current),
                shown(delta.delta),
            ));
        }
        if let Some(reason) = &comparison.reason {
            lines.push(format!("  reason: {reason}"));
        }
    }
    for warning in &report.warnings {
        lines.push(format!("warning: {warning}"));
    }
    lines.join("\n") + "\n"
}

async fn frames(ctx: &CliContext, args: &DiagnosticsFramesArgs) -> Result<CommandSuccess> {
    validate_frames_thresholds(args)?;
    let duration = args
        .duration
        .as_deref()
        .map(parse_duration)
        .transpose()
        .map_err(ForgeError::invalid_argument)?;
    let thresholds = frames_thresholds(args);
    let timeout_secs = ctx.config.file.timeouts.adb_secs.max(1);
    let timeout = ctx.parse_timeout(args.timeout.as_deref(), timeout_secs)?;
    let duration_ms = duration.map(|value| value.as_millis().min(u128::from(u64::MAX)) as u64);
    let baseline_display = args
        .baseline
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());

    if ctx.dry_run {
        return Ok(success(
            "diagnostics.frames",
            json!({
                "dry_run": true,
                "package": args.package,
                "serial": ctx.serial_arg,
                "reset": args.reset,
                "duration_ms": duration_ms,
                "timeout_secs": timeout.as_secs(),
                "baseline": baseline_display,
                "thresholds": {
                    "jank_percent_pp": thresholds.jank_percent_pp,
                    "p99_ms": thresholds.p99_ms,
                    "max_ms": thresholds.max_ms,
                },
                "would_collect": ["gfxinfo frame statistics"],
                "artifact": "frames/report.json",
                "note": if args.reset {
                    "Would reset gfxinfo counters, wait for the optional window, then collect frame statistics."
                } else {
                    "Would collect the current gfxinfo counters; the sampling window is not known without --reset."
                },
            }),
            format!(
                "Would collect gfxinfo frame statistics for {}{} and write frames/report.json\n",
                args.package,
                duration_ms
                    .map(|ms| format!(" over a {ms}ms window"))
                    .unwrap_or_default()
            ),
        )
        .with_optional_session(ctx.session_id()?));
    }

    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let serial = device.serial.clone();
    let package = args.package.clone();
    let reset = args.reset;
    let baseline = args.baseline.clone();

    execute_in_run!(
        ctx,
        "diagnostics.frames",
        Some("diagnostics frames"),
        session.as_ref(),
        Some(&device),
        true,
        |run| async {
            let adb = ctx.adb()?;
            if reset {
                adb.shell_args(
                    &serial,
                    &[
                        "dumpsys".to_string(),
                        "gfxinfo".to_string(),
                        package.clone(),
                        "reset".to_string(),
                    ],
                    Some(timeout),
                    None,
                )
                .await?;
            }
            if let Some(duration) = duration {
                tokio::time::sleep(duration).await;
            }
            let text = adb
                .shell_args(
                    &serial,
                    &[
                        "dumpsys".to_string(),
                        "gfxinfo".to_string(),
                        package.clone(),
                    ],
                    Some(timeout),
                    Some(16 * 1024 * 1024),
                )
                .await?
                .stdout_text();
            let mut report = frames_report_from_gfxinfo(
                &text,
                &package,
                &serial,
                &now_rfc3339(),
                DEFAULT_MIN_FRAMES,
            );
            report.reset_before_collection = reset;
            report.run_id = Some(run.run_id.clone());
            report.run_dir = Some(run.dir.to_string_lossy().into_owned());
            if let Some(path) = &baseline {
                let baseline_path = resolve_input_path(ctx, path);
                let bytes = std::fs::read(&baseline_path).map_err(|error| {
                    ForgeError::new(
                        ErrorCode::FileNotFound,
                        format!(
                            "could not read baseline frame report {}: {error}",
                            baseline_path.display()
                        ),
                    )
                })?;
                let previous: FramesReport = serde_json::from_slice(&bytes).map_err(|error| {
                    ForgeError::invalid_argument(format!(
                        "baseline {} is not a frame report: {error}",
                        baseline_path.display()
                    ))
                })?;
                report.comparison = Some(compare_frames_reports(&previous, &report, thresholds));
                report.baseline_source = Some(path.to_string_lossy().into_owned());
                if let Some(reason) = report
                    .comparison
                    .as_ref()
                    .and_then(|comparison| comparison.reason.as_ref())
                {
                    let warning = format!("baseline comparison unavailable: {reason}");
                    if !report.warnings.contains(&warning) {
                        report.warnings.push(warning);
                    }
                }
            }
            let artifact = run.write_text_artifact(
                "frames/report.json",
                "frames_report",
                Some("gfxinfo frame timing evidence"),
                &serde_json::to_string_pretty(&report)?,
            )?;
            let exit_code = frames_exit_code(&report);
            let mut result = serde_json::to_value(&report)?;
            if let Some(object) = result.as_object_mut() {
                object.insert("run_id".to_string(), json!(run.run_id.clone()));
                object.insert(
                    "run_dir".to_string(),
                    json!(run.dir.to_string_lossy()),
                );
                object.insert("artifacts".to_string(), json!(vec![&artifact]));
                object.insert("duration_ms".to_string(), json!(duration_ms));
                object.insert("path".to_string(), json!(artifact.path));
            }
            if let Some(comparison) = &report.comparison
                && comparison.regression
            {
                let message = comparison
                    .deltas
                    .iter()
                    .find(|delta| delta.status == androidforge_diagnostics::FramesDeltaStatus::Regression)
                    .map(|delta| {
                        format!(
                            "frame regression on {}: {} -> {} (delta {})",
                            delta.metric,
                            delta.baseline.map(|value| value.to_string()).unwrap_or_else(|| "unknown".to_string()),
                            delta.current.map(|value| value.to_string()).unwrap_or_else(|| "unknown".to_string()),
                            delta.delta.map(|value| value.to_string()).unwrap_or_else(|| "unknown".to_string()),
                        )
                    })
                    .unwrap_or_else(|| "frame metrics regressed against the baseline".to_string());
                return Err(ForgeError::new(ErrorCode::PerformanceRegression, message)
                    .with_context("comparison", serde_json::to_value(comparison)?)
                    .with_context("baseline_source", json!(&report.baseline_source))
                    .with_context("artifact", json!(&artifact.path))
                    .with_context("sample_status", json!(&report.sample.sample_status))
                    .with_artifacts(vec![artifact])
                    .device(serial.clone())
                    .with_suggestion(
                        "Inspect frames/report.json and the delta evidence; thresholds are explicit.",
                    ));
            }
            let mut success = success("diagnostics.frames", result, frames_human(&report))
                .with_artifacts(vec![artifact])
                .with_optional_session(session.as_ref().map(|session| session.id.clone()))
                .with_device(serial.clone());
            success.exit_code = exit_code;
            Ok(success)
        },
    )
    .await
}

fn launch_options(
    package: &str,
    args: &CaptureLaunchArgs,
    duration: Option<Duration>,
) -> CaptureLaunchOptions {
    let expected_color_sample = match args.color_sample.as_deref() {
        Some("dominant") => ColorSampleKind::Dominant,
        _ => ColorSampleKind::Centre,
    };
    CaptureLaunchOptions {
        fps: args.fps.unwrap_or(DEFAULT_FPS),
        duration_ms: duration
            .map(|value| value.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(DEFAULT_DURATION_MS),
        package: package.to_string(),
        activity: args.activity.clone(),
        use_ui_dump_package: args.ui_dump_package,
        expect_first_frame: args.expect_first_frame,
        no_splash: args.no_splash,
        expected_color: args.expected_color.clone(),
        expected_color_sample,
        tolerance: args.tolerance.unwrap_or(DEFAULT_TOLERANCE),
        ..CaptureLaunchOptions::default()
    }
}

async fn launch(ctx: &CliContext, args: &CaptureLaunchArgs) -> Result<CommandSuccess> {
    let duration = args
        .duration
        .as_deref()
        .map(parse_duration)
        .transpose()
        .map_err(ForgeError::invalid_argument)?;
    let options = launch_options(&args.package, args, duration);
    if let Some(expected) = &options.expected_color {
        ui_color::parse_hex(expected)?;
    }
    if ctx.dry_run {
        return Ok(success(
            "capture.launch",
            json!({
                "dry_run": true,
                "package": options.package,
                "serial": ctx.serial_arg,
                "fps": options.fps,
                "duration_ms": options.duration_ms,
                "activity": options.activity,
                "expect_first_frame": options.expect_first_frame,
                "no_splash": options.no_splash,
                "expected_color": options.expected_color,
                "expected_color_sample": options.expected_color_sample.as_str(),
                "tolerance": options.tolerance,
                "use_ui_dump_package": options.use_ui_dump_package,
                "would_launch": true,
                "artifact": "capture-launch/frame-*.png",
                "note": "Would launch the package, then sample fresh screencap frames; no device file is created.",
            }),
            format!(
                "Would launch {} and sample launch frames at {} fps for {}ms\n",
                options.package, options.fps, options.duration_ms
            ),
        )
        .with_optional_session(ctx.session_id()?));
    }

    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    // Launching the app is the one deliberate device action here. It keeps the
    // existing scoped physical-device consent rules and adds no new flag.
    ctx.ensure_risk_allowed(&device, Risk::AppControl, "launching an app or intent")?;
    let serial = device.serial.clone();
    let package = options.package.clone();
    let activity = options.activity.clone();

    execute_in_run!(
        ctx,
        "capture.launch",
        Some("capture launch"),
        session.as_ref(),
        Some(&device),
        true,
        |run| async {
            let app = ctx.app_manager()?;
            let launched = app
                .launch(&serial, &package, activity.as_deref(), true)
                .await?;
            let adb = ctx.adb()?;
            let mut source =
                AdbLaunchFrameSource::new(&adb, &serial, &package, options.use_ui_dump_package);
            let cancellation = LaunchCancellation::new();
            let result = sample_launch_capture(
                &mut source,
                Some(run as &mut dyn LaunchArtifactSink),
                &serial,
                &options,
                &cancellation,
            )
            .await?;
            let expectation = FirstFrameExpectation::from(&options);
            let assertion = match assert_first_frame(&result, &expectation) {
                Ok(assertion) => assertion,
                Err(error) => {
                    return Err(error
                        .with_context("status", json!(result.status.as_str()))
                        .with_context("frames_captured", json!(result.frames_captured))
                        .with_context("first_app_frame_ms", json!(result.first_app_frame_ms))
                        .with_context("first_frame_rgb", json!(&result.first_frame_rgb))
                        .with_context("dominant_color", json!(&result.dominant_color))
                        .with_context("no_frame_reason", json!(&result.no_frame_reason)));
                }
            };
            let artifacts = run
                .report
                .artifacts
                .iter()
                .filter(|artifact| {
                    result
                        .artifacts
                        .iter()
                        .any(|frame| frame.path == artifact.path)
                })
                .cloned()
                .collect::<Vec<ArtifactRef>>();
            let mut result_json = serde_json::to_value(&result)?;
            if let Some(object) = result_json.as_object_mut() {
                object.insert("assertion".to_string(), serde_json::to_value(&assertion)?);
                object.insert("launched".to_string(), serde_json::to_value(&launched)?);
                object.insert("run_id".to_string(), json!(run.run_id.clone()));
                object.insert("run_dir".to_string(), json!(run.dir.to_string_lossy()));
            }
            let human = format!(
                "Launch capture for {}: status={} first_app_frame_ms={} colour={} assertion={}\n",
                result.package,
                result.status.as_str(),
                result
                    .first_app_frame_ms
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                result
                    .first_frame_rgb
                    .as_ref()
                    .map(|color| color.hex.clone())
                    .unwrap_or_else(|| "unknown".to_string()),
                assertion.passed
            );
            Ok(success("capture.launch", result_json, human)
                .with_artifacts(artifacts)
                .with_optional_session(session.as_ref().map(|session| session.id.clone()))
                .with_device(serial.clone()))
        },
    )
    .await
}

fn parse_point(value: &str, flag: &str) -> Result<(u32, u32)> {
    let parts = value.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err(ForgeError::invalid_argument(format!(
            "{flag} must be two device-pixel integers: x,y"
        )));
    }
    let x = parts[0].parse::<u32>().map_err(|_| {
        ForgeError::invalid_argument(format!("{flag} must be two device-pixel integers: x,y"))
    })?;
    let y = parts[1].parse::<u32>().map_err(|_| {
        ForgeError::invalid_argument(format!("{flag} must be two device-pixel integers: x,y"))
    })?;
    Ok((x, y))
}

fn parse_region_wh(value: &str) -> Result<ui_color::Region> {
    let parts = value.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() != 4 {
        return Err(ForgeError::invalid_argument(
            "--region must be four device-pixel integers: x,y,w,h",
        ));
    }
    let parsed = parts
        .iter()
        .map(|part| part.parse::<u32>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| {
            ForgeError::invalid_argument("--region must be four device-pixel integers: x,y,w,h")
        })?;
    ui_color::Region::new(parsed[0], parsed[1], parsed[2], parsed[3])
}

/// Resolve the colour to assert from a decoded screenshot. Kept pure so
/// pass/fail sampling can be tested without a device.
fn sample_color(
    image: &androidforge_ui::visual::Image,
    at: Option<(u32, u32)>,
    region: Option<ui_color::Region>,
    majority: bool,
) -> Result<ui_color::Rgb> {
    if let Some((x, y)) = at {
        return ui_color::sample_rgb(image, x, y);
    }
    if let Some(region) = region {
        return if majority {
            ui_color::majority_color(image, region)
        } else {
            ui_color::average_color(image, region)
        };
    }
    if majority {
        ui_color::majority_color(image, ui_color::Region::full(image))
    } else {
        ui_color::sample_rgb(image, image.width / 2, image.height / 2)
    }
}

async fn assert_color(ctx: &CliContext, args: &CaptureAssertColorArgs) -> Result<CommandSuccess> {
    let expected = ui_color::parse_hex(&args.rgb)?;
    let tolerance = args.tolerance.unwrap_or(DEFAULT_TOLERANCE);
    let at = args
        .at
        .as_deref()
        .map(|value| parse_point(value, "--at"))
        .transpose()?;
    let region = args.region.as_deref().map(parse_region_wh).transpose()?;
    let timeout = ctx.parse_timeout(
        args.timeout.as_deref(),
        ctx.config.file.timeouts.screenshot_secs,
    )?;
    let sample_source = if region.is_some() {
        if args.majority {
            "region_majority"
        } else {
            "region_average"
        }
    } else if at.is_some() {
        "at"
    } else if args.majority {
        "majority"
    } else {
        "centre"
    };

    if ctx.dry_run {
        return Ok(success(
            "capture.assert-color",
            json!({
                "dry_run": true,
                "serial": ctx.serial_arg,
                "expected": expected.hex(),
                "tolerance": tolerance,
                "sample_source": sample_source,
                "at": at,
                "region": region.map(|region| json!({
                    "x": region.x, "y": region.y,
                    "width": region.width, "height": region.height,
                })),
                "majority": args.majority,
                "timeout_secs": timeout.as_secs(),
                "would_capture": "fresh host-only screencap",
                "note": "No device session is required for a dry run; no device state is changed.",
            }),
            format!(
                "Would capture a fresh screenshot and check {} ({}) with tolerance {}\n",
                expected.hex(),
                sample_source,
                tolerance
            ),
        )
        .with_optional_session(ctx.session_id()?));
    }

    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let serial = device.serial.clone();
    let adb = ctx.adb()?;
    let output = adb
        .exec_out_args(
            &serial,
            &["screencap".to_string(), "-p".to_string()],
            Some(timeout),
            Some(64 * 1024 * 1024),
        )
        .await?;
    if output.stdout_truncated {
        return Err(ForgeError::new(
            ErrorCode::CommandFailed,
            "screenshot exceeded the 64 MiB capture limit",
        )
        .device(serial.clone()));
    }
    let bytes = output.stdout;
    let image = androidforge_ui::visual::decode_reader(Cursor::new(&bytes), "fresh screenshot")?;
    let actual = sample_color(&image, at, region, args.majority)?;
    let comparison = ui_color::compare(expected, actual, tolerance);
    let actual_hex = actual.hex();
    let result = json!({
        "schema_version": 1,
        "sample_source": sample_source,
        "actual": {"hex": actual_hex, "r": actual.r, "g": actual.g, "b": actual.b},
        "expected": {"hex": expected.hex(), "r": expected.r, "g": expected.g, "b": expected.b},
        "tolerance": tolerance,
        "max_channel_delta": comparison.max_channel_delta,
        "passed": comparison.passed,
        "width": image.width,
        "height": image.height,
        "bytes": bytes.len(),
        "sha256": sha256_hex(&bytes),
        "serial": serial,
        "captured_at": now_rfc3339(),
    });
    if !comparison.passed {
        return Err(ForgeError::new(
            ErrorCode::UiAssertionFailed,
            format!(
                "screenshot {sample_source} colour {actual_hex} did not match expected {} (delta {}, tolerance {tolerance})",
                expected.hex(),
                comparison.max_channel_delta
            ),
        )
        .with_context("actual", json!(actual_hex))
        .with_context("expected", json!(expected.hex()))
        .with_context("tolerance", json!(tolerance))
        .with_context("max_channel_delta", json!(comparison.max_channel_delta))
        .with_context("sample_source", json!(sample_source))
        .with_context("result", result)
        .device(serial.clone()));
    }
    Ok(success(
        "capture.assert-color",
        result,
        format!(
            "Screenshot {sample_source} colour {actual_hex} matched {} (delta {}, tolerance {})\n",
            expected.hex(),
            comparison.max_channel_delta,
            tolerance
        ),
    )
    .with_optional_session(session.as_ref().map(|session| session.id.clone()))
    .with_device(serial))
}

// ---------------------------------------------------------------------------
// Named screenshot baselines
// ---------------------------------------------------------------------------

/// Resolve the run-scoped baseline store. The root is project-relative; the
/// run id is recorded in the manifest as the scope of each update.
fn baseline_store(
    ctx: &CliContext,
    run_id: &str,
) -> crate::commands::visual_baseline::BaselineStore {
    crate::commands::visual_baseline::BaselineStore::for_project(&ctx.paths, run_id)
}

async fn baseline(ctx: &CliContext, command: &BaselineCommand) -> Result<CommandSuccess> {
    match command {
        BaselineCommand::List => {
            let run_id = ctx
                .run_arg
                .clone()
                .or_else(|| std::env::var("ANDROIDFORGE_RUN_ID").ok())
                .unwrap_or_default();
            let store = baseline_store(ctx, &run_id);
            let baselines = store.list()?;
            let count = baselines.len();
            Ok(success(
                "capture.baseline.list",
                json!({
                    "schema_version": 1,
                    "store_root": store.root().to_string_lossy(),
                    "baselines": baselines,
                }),
                format!("{count} named baseline(s).\n"),
            ))
        }
        BaselineCommand::Save(args) => baseline_save(ctx, args, false).await,
        BaselineCommand::Update(args) => baseline_save(ctx, args, true).await,
        BaselineCommand::Compare(args) => baseline_compare(ctx, args).await,
    }
}

/// Capture (or accept) one PNG and save it under a validated name. `update`
/// is an explicit replacement policy; plain `save` refuses an existing name.
async fn baseline_save(
    ctx: &CliContext,
    args: &BaselineSaveArgs,
    update: bool,
) -> Result<CommandSuccess> {
    let name = crate::commands::visual_baseline::validate_name(&args.name)?.to_string();
    let source = args
        .source
        .as_ref()
        .map(|path| resolve_input_path(ctx, path));
    let command = if update {
        "capture.baseline.update"
    } else {
        "capture.baseline.save"
    };
    if ctx.dry_run {
        return Ok(success(
            command,
            json!({
                "dry_run": true,
                "name": name,
                "action": if update { "update" } else { "save" },
                "source": source.as_ref().map(|path| path.to_string_lossy().into_owned()),
                "would_capture": source.is_none(),
            }),
            format!(
                "Would {} baseline '{name}'.\n",
                if update { "update" } else { "save" }
            ),
        ));
    }
    let serial_hint = ctx.serial_arg.clone();
    execute_in_run!(
        ctx,
        command,
        Some(if update {
            "capture baseline update"
        } else {
            "capture baseline save"
        }),
        None,
        None,
        true,
        |run| async {
            let store = baseline_store(ctx, &run.run_id);
            let (path, artifact) = match source {
                Some(path) => (path, None),
                None => {
                    let timeout = ctx.parse_timeout(
                        args.timeout.as_deref(),
                        ctx.config.file.timeouts.screenshot_secs,
                    )?;
                    let adb = ctx.adb()?;
                    let session = ctx.load_session(false)?;
                    let device = ctx.resolve_device(session.as_ref()).await?;
                    let output = adb
                        .exec_out_args(
                            &device.serial,
                            &["screencap".to_string(), "-p".to_string()],
                            Some(timeout),
                            Some(64 * 1024 * 1024),
                        )
                        .await?;
                    if output.stdout_truncated {
                        return Err(ForgeError::new(
                            ErrorCode::CommandFailed,
                            "screenshot exceeded the 64 MiB capture limit",
                        )
                        .device(device.serial.clone()));
                    }
                    let relative = format!("screens/baseline-{name}.png");
                    let artifact = run.write_binary_artifact(
                        &relative,
                        "screenshot",
                        Some("Fresh screenshot saved as a named baseline"),
                        &output.stdout,
                    )?;
                    (Path::new(&artifact.path).to_path_buf(), Some(artifact))
                }
            };
            let entry = if update {
                store.update(&name, &path)?
            } else {
                store.save(&name, &path)?
            };
            if let Some(output) = &args.output {
                let destination = local_output(ctx, output);
                let bytes = std::fs::read(&path).map_err(|error| {
                    ForgeError::new(
                        ErrorCode::ArtifactWriteFailed,
                        format!("could not read baseline PNG {}: {error}", path.display()),
                    )
                })?;
                write_atomic(&destination, &bytes)?;
            }
            let artifacts = artifact.into_iter().collect::<Vec<_>>();
            Ok(success(
                command,
                json!({
                    "schema_version": 1,
                    "name": entry.name,
                    "entry": entry,
                    "store_root": store.root().to_string_lossy(),
                    "run_id": run.run_id,
                    "serial": serial_hint,
                    "updated": update,
                }),
                format!(
                    "{} baseline '{}' ({}x{}, {} bytes).\n",
                    if update { "Updated" } else { "Saved" },
                    entry.name,
                    entry.width,
                    entry.height,
                    entry.size_bytes
                ),
            )
            .with_artifacts(artifacts))
        },
    )
    .await
}

async fn baseline_compare(ctx: &CliContext, args: &BaselineCompareArgs) -> Result<CommandSuccess> {
    let name = crate::commands::visual_baseline::validate_name(&args.name)?.to_string();
    let current = args
        .current
        .as_ref()
        .map(|path| resolve_input_path(ctx, path));
    let region = args
        .region
        .as_deref()
        .map(crate::commands::visual::parse_region)
        .transpose()?;
    if ctx.dry_run {
        return Ok(success(
            "capture.baseline.compare",
            json!({
                "dry_run": true,
                "name": name,
                "current": current.as_ref().map(|path| path.to_string_lossy().into_owned()),
                "region": region,
                "max_diff_ratio": args.max_diff_ratio,
                "diff_threshold": args.diff_threshold,
                "would_capture": current.is_none(),
            }),
            format!("Would compare a fresh screenshot with baseline '{name}'.\n"),
        ));
    }
    let serial_hint = ctx.serial_arg.clone();
    execute_in_run!(
        ctx,
        "capture.baseline.compare",
        Some("capture baseline compare"),
        None,
        None,
        true,
        |run| async {
            let store = baseline_store(ctx, &run.run_id);
            let (path, artifact) = match current {
                Some(path) => (path, None),
                None => {
                    let timeout = ctx.parse_timeout(
                        args.timeout.as_deref(),
                        ctx.config.file.timeouts.screenshot_secs,
                    )?;
                    let adb = ctx.adb()?;
                    let session = ctx.load_session(false)?;
                    let device = ctx.resolve_device(session.as_ref()).await?;
                    let output = adb
                        .exec_out_args(
                            &device.serial,
                            &["screencap".to_string(), "-p".to_string()],
                            Some(timeout),
                            Some(64 * 1024 * 1024),
                        )
                        .await?;
                    if output.stdout_truncated {
                        return Err(ForgeError::new(
                            ErrorCode::CommandFailed,
                            "screenshot exceeded the 64 MiB capture limit",
                        )
                        .device(device.serial.clone()));
                    }
                    let artifact = run.write_binary_artifact(
                        "screens/baseline-current.png",
                        "screenshot",
                        Some("Fresh screenshot compared with a named baseline"),
                        &output.stdout,
                    )?;
                    (Path::new(&artifact.path).to_path_buf(), Some(artifact))
                }
            };
            if let Some(output) = &args.output {
                let destination = local_output(ctx, output);
                let bytes = std::fs::read(&path).map_err(|error| {
                    ForgeError::new(
                        ErrorCode::ArtifactWriteFailed,
                        format!("could not read screenshot {}: {error}", path.display()),
                    )
                })?;
                write_atomic(&destination, &bytes)?;
            }
            let diff_output = args
                .diff_output
                .as_ref()
                .map(|path| local_output(ctx, path))
                .unwrap_or_else(|| run.dir.join("visual").join("baseline-diff.png"));
            let comparison = store.compare(
                &name,
                &path,
                region,
                args.diff_threshold,
                args.max_diff_ratio,
                Some(&diff_output),
            )?;
            let mut artifacts = Vec::new();
            if let Some(artifact) = artifact {
                artifacts.push(artifact);
            }
            artifacts.push(run.register_artifact(
                &diff_output,
                "visual_diff",
                Some("Named baseline visual diff"),
            )?);
            if !comparison.passed {
                let mut error = ForgeError::new(
                    ErrorCode::UiAssertionFailed,
                    format!(
                        "baseline '{name}' differs from the current screenshot: {:.4}% changed pixels (limit {:.4}%)",
                        comparison.diff.changed_ratio * 100.0,
                        comparison.diff.max_diff_ratio * 100.0
                    ),
                )
                .with_context("name", json!(name))
                .with_context(
                    "comparison",
                    serde_json::to_value(&comparison).unwrap_or(json!(null)),
                )
                .with_context(
                    "diff",
                    serde_json::to_value(&comparison.diff).unwrap_or(json!(null)),
                )
                .with_suggestion(
                    "Inspect the diff image, then update the baseline only if the change is intended.",
                )
                .with_artifacts(artifacts);
                if let Some(serial) = serial_hint {
                    error = error.device(serial);
                }
                return Err(error);
            }
            Ok(success(
                "capture.baseline.compare",
                json!({
                    "schema_version": 1,
                    "name": name,
                    "comparison": comparison,
                    "run_id": run.run_id,
                    "serial": serial_hint,
                }),
                format!(
                    "Baseline '{}' matched: {} changed pixel(s) of {}.\n",
                    name, comparison.diff.changed_pixels, comparison.diff.compared_pixels
                ),
            )
            .with_artifacts(artifacts))
        },
    )
    .await
}

/// WCAG contrast assertion for a fresh (or supplied) screenshot. Sampling uses
/// the shared colour helpers; the finding uses the shared a11y helpers.
async fn capture_contrast(ctx: &CliContext, args: &CaptureContrastArgs) -> Result<CommandSuccess> {
    let foreground = ui_color::parse_hex(&args.rgb)?;
    let at = args
        .at
        .as_deref()
        .map(|value| parse_point(value, "--at"))
        .transpose()?;
    let region = args.region.as_deref().map(parse_region_wh).transpose()?;
    let threshold = args
        .threshold
        .unwrap_or(if args.large_text { 3.0 } else { 4.5 });
    let sample_source = if region.is_some() {
        if args.majority {
            "region_majority"
        } else {
            "region_average"
        }
    } else if at.is_some() {
        "at"
    } else if args.majority {
        "majority"
    } else {
        "centre"
    };
    if ctx.dry_run {
        return Ok(success(
            "capture.contrast",
            json!({
                "dry_run": true,
                "foreground": foreground.hex(),
                "threshold": threshold,
                "sample_source": sample_source,
                "at": at,
                "region": region.map(|region| json!({
                    "x": region.x, "y": region.y,
                    "width": region.width, "height": region.height,
                })),
                "large_text": args.large_text,
                "would_capture": args.source.is_none(),
            }),
            format!(
                "Would check contrast of {} ({}, threshold {:.2}:1).\n",
                foreground.hex(),
                sample_source,
                threshold
            ),
        ));
    }
    let bytes = if let Some(path) = &args.source {
        let source = resolve_input_path(ctx, path);
        std::fs::read(&source).map_err(|error| {
            ForgeError::new(
                ErrorCode::FileNotFound,
                format!(
                    "could not read contrast source {}: {error}",
                    source.display()
                ),
            )
        })?
    } else {
        let timeout = ctx.parse_timeout(
            args.timeout.as_deref(),
            ctx.config.file.timeouts.screenshot_secs,
        )?;
        let session = ctx.load_session(true)?;
        let device = ctx.resolve_device(session.as_ref()).await?;
        let output = ctx
            .adb()?
            .exec_out_args(
                &device.serial,
                &["screencap".to_string(), "-p".to_string()],
                Some(timeout),
                Some(64 * 1024 * 1024),
            )
            .await?;
        if output.stdout_truncated {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                "screenshot exceeded the 64 MiB capture limit",
            )
            .device(device.serial.clone()));
        }
        output.stdout
    };
    let image = androidforge_ui::visual::decode_reader(Cursor::new(bytes), "contrast screenshot")?;
    let check = if let Some((x, y)) = at {
        ui_a11y::check_contrast(foreground, ui_color::sample_rgb(&image, x, y)?, threshold)
    } else if let Some(region) = region {
        if args.majority {
            ui_a11y::audit_region_contrast(&image, region, foreground, threshold)?
        } else {
            ui_a11y::check_contrast(
                foreground,
                ui_color::average_color(&image, region)?,
                threshold,
            )
        }
    } else if args.majority {
        ui_a11y::audit_region_contrast(
            &image,
            ui_color::Region::full(&image),
            foreground,
            threshold,
        )?
    } else {
        ui_a11y::check_contrast(
            foreground,
            ui_color::sample_rgb(&image, image.width / 2, image.height / 2)?,
            threshold,
        )
    };
    let sampled = region.unwrap_or_else(|| ui_color::Region::full(&image));
    let node = androidforge_ui::UiNode {
        id: 0,
        parent: None,
        children: Vec::new(),
        depth: 0,
        text: args.label.clone(),
        content_desc: None,
        hint: None,
        resource_id: None,
        class: None,
        package: None,
        bounds: androidforge_ui::Rect {
            left: sampled.x as i32,
            top: sampled.y as i32,
            right: sampled.x.saturating_add(sampled.width) as i32,
            bottom: sampled.y.saturating_add(sampled.height) as i32,
        },
        bounds_raw: String::new(),
        clickable: false,
        long_clickable: false,
        enabled: true,
        focused: false,
        selected: false,
        scrollable: false,
        checked: false,
        password: false,
        displayed: Some(true),
        index_attr: None,
        path: "contrast".to_string(),
    };
    let finding = ui_a11y::contrast_finding(&node, &check, args.large_text);
    let result = json!({
        "schema_version": 1,
        "sample_source": sample_source,
        "foreground": check.foreground_hex,
        "background": check.background_hex,
        "ratio": check.ratio,
        "threshold": check.threshold,
        "large_text": args.large_text,
        "passed": check.passed,
        "finding": finding,
        "width": image.width,
        "height": image.height,
        "captured_at": now_rfc3339(),
    });
    if !check.passed {
        return Err(ForgeError::new(
            ErrorCode::UiAssertionFailed,
            format!(
                "contrast {:.2}:1 is below the required {:.2}:1 ({} on {})",
                check.ratio, check.threshold, check.foreground_hex, check.background_hex
            ),
        )
        .with_context("result", result.clone())
        .with_context(
            "finding",
            serde_json::to_value(&finding).unwrap_or(json!(null)),
        ));
    }
    Ok(success(
        "capture.contrast",
        result,
        format!(
            "Contrast passed: {:.2}:1 ({} on {}, threshold {:.2}:1).\n",
            check.ratio, check.foreground_hex, check.background_hex, check.threshold
        ),
    ))
}

/// Extra redaction is strictly opt-in. With no `--redact-patterns` the original
/// byte-for-byte behavior is preserved: no extra rules are applied at all.
fn logcat_redactor(ctx: &CliContext, patterns: &[String]) -> Option<Redactor> {
    if patterns.is_empty() {
        return None;
    }
    Some(Redactor::with_literal_prefixes(
        true,
        patterns,
        &ctx.config.file.redaction.patterns,
    ))
}

fn redact_logcat_result(result: &mut LogcatResult, redactor: &Redactor) {
    result.raw = redactor.redact(&result.raw);
    result.filtered = redactor.redact(&result.filtered);
    result.crash_buffer = redactor.redact(&result.crash_buffer);
    result.preview = redactor.redact(&result.preview);
    result.raw_bytes = result.raw.len() as u64;
    result.filtered_bytes = result.filtered.len() as u64;
    result.crash_bytes = result.crash_buffer.len() as u64;
}

fn redaction_sidecar_path(ctx: &CliContext, session_id: &str) -> std::path::PathBuf {
    let safe = session_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    ctx.paths
        .state_dir
        .join("logcat-redact")
        .join(format!("{safe}.json"))
}

async fn logcat(ctx: &CliContext, command: &LogcatCommand) -> Result<CommandSuccess> {
    match command {
        LogcatCommand::Tail(args) => logcat_tail(ctx, args).await,
        LogcatCommand::Peek(args) => logcat_peek(ctx, args).await,
        LogcatCommand::Query(args) => logcat_query(ctx, args).await,
        LogcatCommand::Watch(_) => Err(ForgeError::new(
            ErrorCode::UnsupportedOperation,
            "logcat watch is an NDJSON stream and is handled directly by the CLI entry point",
        )),
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
    let redactor = logcat_redactor(ctx, &args.redact_patterns);
    execute_in_run!(
        ctx,
        "diagnostics.logcat",
        Some("diagnostics logcat"),
        session.as_ref(),
        Some(&device),
        true,
        |run| async {
            let (mut result, retry_report) = common::retry_read_only(
                ReadOnlyOperation::LogcatTail,
                args.retries,
                || async {
                    let adb = ctx.adb()?;
                    collect_logcat_tail(
                        &adb,
                        &serial,
                        package.as_deref(),
                        Some(lines),
                        Some(max_bytes),
                    )
                    .await
                },
            )
            .await?;
            if let Some(redactor) = &redactor {
                redact_logcat_result(&mut result, redactor);
            }
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
                if redactor.is_some() {
                    object.insert("redaction_applied".to_string(), serde_json::json!(true));
                }
                object.insert("raw_artifact".to_string(), serde_json::json!(&artifacts[0].path));
                object.insert("filtered_artifact".to_string(), serde_json::json!(&artifacts[1].path));
                if args.retries > 0 {
                    object.insert(
                        "retry".to_string(),
                        serde_json::to_value(&retry_report).unwrap_or(serde_json::Value::Null),
                    );
                }
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

async fn logcat_peek(ctx: &CliContext, args: &LogcatPeekArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    // Read-only peek feeds the desktop live view. Do not silently fall back to
    // the project package: an empty filter must mean "all processes" so the
    // panel can show system and crash logs too.
    let package = args.package.clone();
    let lines = args
        .lines
        .unwrap_or(ctx.config.file.diagnostics.logcat_tail_lines)
        .clamp(1, 20_000);
    let redactor = logcat_redactor(ctx, &args.redact_patterns);
    let (mut result, retry_report) =
        common::retry_read_only(ReadOnlyOperation::LogcatPeek, args.retries, || async {
            collect_logcat_peek(
                &ctx.adb()?,
                &device.serial,
                package.as_deref(),
                Some(lines),
                Some(ctx.config.file.diagnostics.max_log_bytes),
                args.level.as_deref(),
                args.since.as_deref(),
            )
            .await
        })
        .await?;
    if let Some(redactor) = &redactor {
        result.lines = result
            .lines
            .iter()
            .map(|line| redactor.redact(line))
            .collect::<Vec<_>>();
        result.notes = result
            .notes
            .iter()
            .map(|note| redactor.redact(note))
            .collect::<Vec<_>>();
    }
    let preview = result
        .lines
        .iter()
        .rev()
        .take(40)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let mut json = serde_json::to_value(&result)?;
    if let Some(object) = json.as_object_mut() {
        if redactor.is_some() {
            object.insert("redaction_applied".to_string(), serde_json::json!(true));
        }
        if args.retries > 0 {
            object.insert(
                "retry".to_string(),
                serde_json::to_value(&retry_report).unwrap_or(serde_json::Value::Null),
            );
        }
    }
    Ok(success(
        "diagnostics.logcat.peek",
        json,
        format!(
            "{} log line(s) from {}{}\n{preview}\n",
            result.line_count,
            device.serial,
            package
                .as_deref()
                .map(|package| format!(" for {package}"))
                .unwrap_or_default()
        ),
    )
    .with_optional_session(session.as_ref().map(|session| session.id.clone()))
    .with_device(device.serial))
}

async fn logcat_query(ctx: &CliContext, args: &LogcatQueryArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let lines = args.lines.clamp(1, 20_000);
    let redactor = logcat_redactor(ctx, &args.redact_patterns);
    let (mut result, retry_report) =
        common::retry_read_only(ReadOnlyOperation::LogcatPeek, args.retries, || async {
            collect_logcat_peek(
                &ctx.adb()?,
                &device.serial,
                args.package.as_deref(),
                Some(lines),
                Some(ctx.config.file.diagnostics.max_log_bytes),
                args.level.as_deref(),
                args.since.as_deref(),
            )
            .await
        })
        .await?;
    if let Some(redactor) = &redactor {
        result.lines = result
            .lines
            .iter()
            .map(|line| redactor.redact(line))
            .collect::<Vec<_>>();
        result.notes = result
            .notes
            .iter()
            .map(|note| redactor.redact(note))
            .collect::<Vec<_>>();
    }
    let regex = args
        .grep
        .as_deref()
        .map(regex::Regex::new)
        .transpose()
        .map_err(|error| ForgeError::invalid_argument(format!("invalid --grep: {error}")))?;
    let mut records = result
        .lines
        .iter()
        .filter_map(|line| parse_threadtime_line(line))
        .filter(|record| {
            args.until
                .as_deref()
                .is_none_or(|until| record.timestamp.as_str() <= until)
                && regex.as_ref().is_none_or(|regex| {
                    regex.is_match(&record.message) || regex.is_match(&record.tag)
                })
        })
        .collect::<Vec<LogcatRecord>>();
    let matched = records.len();
    if records.len() > args.limit {
        records = records.split_off(records.len() - args.limit);
    }
    let mut value = serde_json::json!({
        "schema_version": 1,
        "serial": device.serial,
        "package": args.package,
        "level": args.level,
        "since": args.since,
        "until": args.until,
        "grep": args.grep,
        "records": records,
        "matched": matched,
        "returned": records.len(),
        "line_count": result.line_count,
        "pid_filter": result.pid_filter,
        "truncated": result.truncated,
        "captured_at": result.captured_at,
        "notes": result.notes,
        "retry": if args.retries > 0 { serde_json::to_value(&retry_report).unwrap_or(serde_json::Value::Null) } else { serde_json::Value::Null },
    });
    if redactor.is_some()
        && let Some(object) = value.as_object_mut()
    {
        object.insert("redaction_applied".to_string(), serde_json::json!(true));
    }
    let human = records
        .iter()
        .map(|record| {
            format!(
                "{} {} {}: {}",
                record.timestamp, record.level, record.tag, record.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(success(
        "diagnostics.logcat.query",
        value,
        format!(
            "{} matching record(s), {} returned from {} line(s)\n{}\n",
            matched,
            records.len(),
            result.line_count,
            human
        ),
    )
    .with_optional_session(session.as_ref().map(|session| session.id.clone()))
    .with_device(device.serial))
}

pub async fn stream_logcat_watch(cli: &crate::cli::Cli, args: &LogcatWatchArgs) -> Result<()> {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};
    let ctx = CliContext::new(cli)?;
    let redactor = logcat_redactor(&ctx, &args.redact_patterns);
    let session = ctx.require_session()?;
    let device = ctx.resolve_device(Some(&session)).await?;
    let adb = ctx.adb()?;
    let timeout = ctx.parse_timeout(Some(&args.timeout), 60)?;
    let regex = args
        .grep
        .as_deref()
        .map(regex::Regex::new)
        .transpose()
        .map_err(|error| ForgeError::invalid_argument(format!("invalid --grep: {error}")))?;
    let level = args.level.as_deref().and_then(priority).unwrap_or(0);
    let package = args.package.as_deref();
    let pids = if let Some(package) = package {
        adb.shell_args(
            &device.serial,
            &["pidof".to_string(), package.to_string()],
            Some(Duration::from_secs(5)),
            None,
        )
        .await
        .ok()
        .map(|output| {
            output
                .stdout_text()
                .split_whitespace()
                .filter_map(|value| value.parse::<u32>().ok())
                .collect::<Vec<u32>>()
        })
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    if package.is_some() && pids.is_empty() {
        eprintln!(
            "note: the package process was not running; only lines mentioning the package name will be kept"
        );
    }
    let mut child = tokio::process::Command::new(&adb.adb)
        .arg("-s")
        .arg(&device.serial)
        .arg("shell")
        .arg("logcat")
        .arg("-v")
        .arg("threadtime")
        .arg("-T")
        .arg("1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            ForgeError::new(
                ErrorCode::CommandFailed,
                format!("could not start logcat stream: {error}"),
            )
            .with_suggestion("Verify ADB is available and the device is online.")
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ForgeError::new(ErrorCode::CommandFailed, "logcat stream has no stdout"))?;
    let mut lines = BufReader::new(stdout).lines();
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut emitted = 0usize;
    loop {
        let line = tokio::select! {
            _ = &mut deadline => break,
            line = lines.next_line() => line,
        };
        let Ok(Some(line)) = line else {
            break;
        };
        let Some(mut record) = parse_threadtime_line(&line) else {
            continue;
        };
        if priority(&record.level).unwrap_or(0) < level {
            continue;
        }
        let crash = record.level == "F" || record.message.contains("FATAL EXCEPTION");
        let anr = record.message.contains("ANR in ");
        if !watch_record_allowed(
            &record,
            package,
            &pids,
            args.crash,
            args.anr,
            regex.as_ref(),
        ) {
            continue;
        }
        if let Some(redactor) = &redactor {
            record.message = redactor.redact(&record.message);
            record.tag = redactor.redact(&record.tag);
            record.raw = redactor.redact(&record.raw);
        }
        let event = if crash {
            "crash"
        } else if anr {
            "anr"
        } else {
            "log"
        };
        let mut event_json = serde_json::json!({
            "schema_version": 1,
            "event": event,
            "serial": device.serial,
            "session_id": session.id,
            "timestamp": record.timestamp,
            "pid": record.pid,
            "tid": record.tid,
            "level": record.level,
            "tag": record.tag,
            "message": record.message,
            "raw": record.raw,
        });
        if redactor.is_some()
            && let Some(object) = event_json.as_object_mut()
        {
            object.insert("redaction_applied".to_string(), serde_json::json!(true));
        }
        println!("{}", serde_json::to_string(&event_json)?);
        use std::io::Write;
        let _ = std::io::stdout().flush();
        emitted += 1;
        if emitted >= args.max_events.max(1) {
            break;
        }
    }
    let _ = child.kill().await;
    Ok(())
}

fn priority(value: &str) -> Option<u8> {
    match value.trim().to_ascii_lowercase().as_str() {
        "v" | "verbose" => Some(0),
        "d" | "debug" => Some(1),
        "i" | "info" => Some(2),
        "w" | "warn" | "warning" => Some(3),
        "e" | "error" => Some(4),
        "f" | "fatal" | "assert" => Some(5),
        _ => None,
    }
}

/// Decide whether a streamed record should be emitted. Any explicit filter
/// (`--package`, `--crash`, `--anr`, `--grep`) narrows the stream, and a record
/// matching any of the enabled filters is kept.
fn watch_record_allowed(
    record: &LogcatRecord,
    package: Option<&str>,
    pids: &[u32],
    crash_only: bool,
    anr_only: bool,
    regex: Option<&regex::Regex>,
) -> bool {
    let crash = record.level == "F" || record.message.contains("FATAL EXCEPTION");
    let anr = record.message.contains("ANR in ");
    let grep_match =
        regex.is_some_and(|regex| regex.is_match(&record.message) || regex.is_match(&record.tag));
    let package_match = package
        .is_some_and(|package| logcat_line_matches_package(&record.raw, record.pid, package, pids));
    (!crash_only && !anr_only && regex.is_none() && package.is_none())
        || (crash_only && crash)
        || (anr_only && anr)
        || grep_match
        || package_match
}

async fn logcat_start(ctx: &CliContext, args: &LogcatStartArgs) -> Result<CommandSuccess> {
    let session = ctx.require_session()?;
    let device = ctx.resolve_device(Some(&session)).await?;
    if ctx.dry_run {
        let mut value = serde_json::json!({"dry_run": true, "serial": device.serial});
        if !args.redact_patterns.is_empty()
            && let Some(object) = value.as_object_mut()
        {
            object.insert(
                "redact_patterns".to_string(),
                serde_json::to_value(&args.redact_patterns)?,
            );
        }
        return Ok(success(
            "diagnostics.logcat.start",
            value,
            "Would start continuous logcat capture.\n".to_string(),
        )
        .with_session(session.id)
        .with_device(device.serial));
    }
    // Persist the sidecar before starting the detached capture so a failed
    // write can never leave an unredacted capture with no record of the prefixes.
    if !args.redact_patterns.is_empty() {
        let sidecar = redaction_sidecar_path(ctx, &session.id);
        if let Some(parent) = sidecar.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                ForgeError::new(
                    ErrorCode::ArtifactWriteFailed,
                    format!("could not create {}: {error}", parent.display()),
                )
            })?;
        }
        write_json_atomic(
            &sidecar,
            &serde_json::json!({
                "schema_version": 1,
                "session_id": session.id,
                "patterns": args.redact_patterns,
                "created_at": now_rfc3339(),
            }),
        )?;
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
            // `LogcatStream` owns the detached capture and its state file. The
            // CLI-owned sidecar written above is consumed by `logcat stop`.
            let mut details = serde_json::json!({"capture": state});
            if !args.redact_patterns.is_empty()
                && let Some(object) = details.as_object_mut()
            {
                object.insert(
                    "redact_patterns".to_string(),
                    serde_json::to_value(&args.redact_patterns)?,
                );
                object.insert("redaction_applied".to_string(), serde_json::json!(true));
            }
            let run_id = scope.finish_success_held(details, Vec::new())?;
            if scope.implicit {
                set_active_run(
                    &ctx.paths,
                    Some(&session.id),
                    &run_id,
                    &ctx.paths.project_root,
                )?;
            }
            let human = if state.status == "completed" {
                format!(
                    "Logcat capture completed at its {} byte cap for {}. Run `androidforge diagnostics logcat stop --session {}` to finalize the artifacts.\n",
                    state.max_bytes, state.serial, session.id
                )
            } else {
                format!(
                    "Logcat capture started for {}. Stop it with `androidforge diagnostics logcat stop --session {}`.\n",
                    state.serial, session.id
                )
            };
            Ok(success(
                "diagnostics.logcat.start",
                serde_json::to_value(&state)?,
                human,
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
    // The detached capture writes its raw file continuously. Stop that process
    // first, then apply the sidecar prefixes to the raw file *before*
    // `LogcatStream::stop` reads it, redacts with configured rules and registers
    // checksummed artifacts. This keeps the custom prefixes out of both stored
    // capture text and the run report without changing the stream's state schema.
    let sidecar = redaction_sidecar_path(ctx, &session.id);
    let mut extra_redaction = false;
    if sidecar.is_file() {
        let value: serde_json::Value = read_json(&sidecar)?;
        let patterns = value
            .get("patterns")
            .and_then(serde_json::Value::as_array)
            .and_then(|values| {
                values
                    .iter()
                    .map(|value| value.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| {
                ForgeError::new(
                    ErrorCode::UnsupportedOperation,
                    "logcat redaction sidecar is invalid; the capture will not be finalized",
                )
                .with_suggestion("Restart the capture with `diagnostics logcat start`.")
                .session(session.id.clone())
            })?;
        extra_redaction = !patterns.is_empty();
        if extra_redaction {
            let state = stream.load(&session.id)?.ok_or_else(|| {
                ForgeError::new(
                    ErrorCode::SessionExpired,
                    "logcat redaction sidecar has no matching capture state",
                )
                .session(session.id.clone())
            })?;
            androidforge_process::kill_tree(state.local_pid);
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let raw_path = std::path::PathBuf::from(&state.run_dir).join(&state.raw_relative_path);
            if raw_path.is_file() {
                let file = std::fs::File::open(&raw_path).map_err(|error| {
                    ForgeError::new(
                        ErrorCode::FileNotFound,
                        format!("could not open {}: {error}", raw_path.display()),
                    )
                })?;
                let limit = (state.max_bytes as usize).saturating_add(4096);
                let mut bytes = Vec::new();
                file.take(limit as u64)
                    .read_to_end(&mut bytes)
                    .map_err(|error| {
                        ForgeError::new(
                            ErrorCode::InvalidArgument,
                            format!("could not read {}: {error}", raw_path.display()),
                        )
                    })?;
                let text = String::from_utf8_lossy(&bytes);
                let redactor = Redactor::with_literal_prefixes(
                    true,
                    &patterns,
                    &ctx.config.file.redaction.patterns,
                );
                write_atomic(&raw_path, redactor.redact(&text).as_bytes())?;
            }
        }
    }
    let result = stream.stop(&session.id, &mut run).await;
    match result {
        Ok(mut outcome) => {
            if extra_redaction {
                let _ = std::fs::remove_file(&sidecar);
            }
            let artifacts = outcome_artifacts(
                &run,
                &[&outcome.raw_artifact, &outcome.filtered_artifact],
                outcome.crash_artifact.as_deref(),
            );
            if extra_redaction {
                outcome.notes.push(
                    "custom literal-prefix redaction was applied to the captured text".to_string(),
                );
            }
            let mut value = serde_json::to_value(&outcome)?;
            if extra_redaction && let Some(object) = value.as_object_mut() {
                object.insert("redaction_applied".to_string(), serde_json::json!(true));
            }
            run.finish_step_success(step, value.clone(), artifacts.clone())?;
            run.finalize_pass()?;
            clear_active_run(&ctx.paths, Some(&session.id))?;
            Ok(success(
                "diagnostics.logcat.stop",
                value,
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

/// Resolve a user-supplied baseline path. Relative paths are tried against the
/// working directory first, then the artifact directory so reports can refer to
/// `runs/<id>/screens/...` without repeating the project root.
fn resolve_input_path(ctx: &CliContext, path: &Path) -> std::path::PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let cwd = ctx.paths.cwd.join(path);
    if cwd.is_file() {
        return cwd;
    }
    let artifact = ctx.paths.artifacts_dir.join(path);
    if artifact.is_file() {
        return artifact;
    }
    cwd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pid: Option<u32>, tag: &str, message: &str) -> LogcatRecord {
        let process = pid.unwrap_or(0);
        LogcatRecord {
            timestamp: "11-15 10:00:00.000".to_string(),
            pid,
            tid: Some(1300),
            level: "I".to_string(),
            tag: tag.to_string(),
            message: message.to_string(),
            raw: format!("11-15 10:00:00.000  {process}  1300 I/{tag}: {message}"),
        }
    }

    #[test]
    fn watch_package_filter_keeps_process_and_named_lines() {
        let process = record(Some(1234), "Tag", "ordinary process line");
        let named = record(Some(9999), "Tag", "starting com.example");
        let other = record(Some(9999), "Tag", "unrelated line");
        assert!(watch_record_allowed(
            &process,
            Some("com.example"),
            &[1234],
            false,
            false,
            None
        ));
        assert!(watch_record_allowed(
            &named,
            Some("com.example"),
            &[1234],
            false,
            false,
            None
        ));
        assert!(!watch_record_allowed(
            &other,
            Some("com.example"),
            &[1234],
            false,
            false,
            None
        ));
    }

    #[test]
    fn watch_package_filter_without_running_process_still_matches_name() {
        let named = record(Some(9999), "Tag", "starting com.example");
        let other = record(Some(9999), "Tag", "unrelated line");
        assert!(watch_record_allowed(
            &named,
            Some("com.example"),
            &[],
            false,
            false,
            None
        ));
        assert!(!watch_record_allowed(
            &other,
            Some("com.example"),
            &[],
            false,
            false,
            None
        ));
    }

    #[test]
    fn watch_filters_are_combined_as_or() {
        let other = record(Some(9999), "Tag", "unrelated line");
        let fatal = record(Some(9999), "AndroidRuntime", "FATAL EXCEPTION: main");
        let grep = regex::Regex::new("unrelated").unwrap();
        assert!(watch_record_allowed(
            &other,
            Some("com.example"),
            &[1234],
            false,
            false,
            Some(&grep)
        ));
        assert!(watch_record_allowed(
            &fatal,
            Some("com.example"),
            &[1234],
            true,
            false,
            None
        ));
        assert!(!watch_record_allowed(
            &other,
            Some("com.example"),
            &[1234],
            true,
            true,
            None
        ));
        assert!(watch_record_allowed(&other, None, &[], false, false, None));
    }

    fn base_performance_report() -> PerformanceReport {
        PerformanceReport {
            schema_version: 1,
            package: "com.example".to_string(),
            serial: "emulator-5554".to_string(),
            collected_at: "2026-01-01T00:00:00.000Z".to_string(),
            cold_start_requested: false,
            settle_ms: 0,
            screen_on: Some(true),
            foreground: None,
            activity: None,
            startup: Default::default(),
            jank: Default::default(),
            memory: Default::default(),
            cpu: Default::default(),
            sample_quality: Default::default(),
            commands: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn performance_human_reports_insufficient_sample_instead_of_percentage() {
        let mut report = base_performance_report();
        report.jank = androidforge_diagnostics::JankMeasurement {
            total_frames: Some(1),
            janky_frames: Some(1),
            janky_percent: Some(100.0),
            sample_status: "insufficient".to_string(),
            comparison_reason: Some("insufficient sample (1 frame(s))".to_string()),
            ..Default::default()
        };
        report.cpu.status = "unavailable".to_string();
        report.cpu.reason = Some("cpuinfo window is stale".to_string());
        let human = performance_human(&report);
        assert!(
            human.contains("jank: insufficient sample (1 frame(s))"),
            "human: {human}"
        );
        assert!(!human.contains("100.00%"), "human: {human}");
        assert!(human.contains("cpu: unavailable"), "human: {human}");
        assert!(human.contains("screen: on"), "human: {human}");
    }

    const GFXINFO: &str = "Graphics info for pid 1234 [com.example]\n\
        Total frames rendered: 120\n\
        Janky frames: 9 (7.50%)\n\
        99th percentile: 40ms\n\
        Max: 72ms\n";

    fn frames_args() -> DiagnosticsFramesArgs {
        DiagnosticsFramesArgs {
            package: "com.example".to_string(),
            reset: false,
            duration: None,
            timeout: None,
            baseline: None,
            max_janky: None,
            max_p99: None,
            max_frame_ms: None,
        }
    }

    #[test]
    fn frames_threshold_flags_map_to_explicit_tolerances() {
        let mut args = frames_args();
        assert_eq!(frames_thresholds(&args), FramesThresholds::default());
        args.max_janky = Some(2.5);
        args.max_p99 = Some(5.0);
        args.max_frame_ms = Some(10.0);
        let thresholds = frames_thresholds(&args);
        assert_eq!(thresholds.jank_percent_pp, 2.5);
        assert_eq!(thresholds.p99_ms, 5.0);
        assert_eq!(thresholds.max_ms, 10.0);
        assert!(validate_frames_thresholds(&args).is_ok());

        args.max_janky = Some(-1.0);
        assert!(validate_frames_thresholds(&args).is_err());
        args.max_janky = Some(f64::NAN);
        assert!(validate_frames_thresholds(&args).is_err());
    }

    #[test]
    fn frames_parsing_and_threshold_comparison_without_a_device() {
        let baseline = frames_report_from_gfxinfo(
            GFXINFO,
            "com.example",
            "emulator-5554",
            "2026-01-01T00:00:00.000Z",
            DEFAULT_MIN_FRAMES,
        );
        assert_eq!(baseline.sample.total_frames, Some(120));
        assert_eq!(baseline.sample.janky_percent, Some(7.5));
        assert_eq!(frames_exit_code(&baseline), 0);

        let current_text = GFXINFO
            .replace("Janky frames: 9 (7.50%)", "Janky frames: 12 (10.00%)")
            .replace("99th percentile: 40ms", "99th percentile: 48ms");
        let current = frames_report_from_gfxinfo(
            &current_text,
            "com.example",
            "emulator-5554",
            "2026-01-02T00:00:00.000Z",
            DEFAULT_MIN_FRAMES,
        );
        let strict = compare_frames_reports(&baseline, &current, FramesThresholds::default());
        assert!(strict.regression);
        let mut warned = baseline.clone();
        warned.warnings.push("partial evidence".to_string());
        assert_eq!(frames_exit_code(&warned), 10);
        let mut regressed = baseline.clone();
        regressed.comparison = Some(androidforge_diagnostics::FramesComparison {
            comparable: true,
            regression: true,
            ..Default::default()
        });
        assert_eq!(frames_exit_code(&regressed), 11);

        let tolerant = compare_frames_reports(
            &baseline,
            &current,
            FramesThresholds {
                jank_percent_pp: 3.0,
                p99_ms: 10.0,
                max_ms: 10.0,
            },
        );
        assert!(tolerant.comparable);
        assert!(!tolerant.regression);
    }

    #[test]
    fn launch_options_map_cli_flags_to_core_options() {
        let args = CaptureLaunchArgs {
            package: "com.example".to_string(),
            activity: Some(".Main".to_string()),
            fps: Some(12),
            duration: Some("5s".to_string()),
            ui_dump_package: true,
            expect_first_frame: true,
            no_splash: true,
            expected_color: Some("#FF0000".to_string()),
            color_sample: Some("dominant".to_string()),
            tolerance: Some(8),
        };
        let options = launch_options(&args.package, &args, Some(Duration::from_millis(5000)));
        assert_eq!(options.fps, 12);
        assert_eq!(options.duration_ms, 5000);
        assert_eq!(options.activity.as_deref(), Some(".Main"));
        assert!(options.use_ui_dump_package);
        assert!(options.expect_first_frame);
        assert!(options.no_splash);
        assert_eq!(options.expected_color.as_deref(), Some("#FF0000"));
        assert_eq!(options.expected_color_sample, ColorSampleKind::Dominant);
        assert_eq!(options.tolerance, 8);

        let defaults = launch_options("com.example", &frames_launch_defaults(), None);
        assert_eq!(defaults.fps, DEFAULT_FPS);
        assert_eq!(defaults.duration_ms, DEFAULT_DURATION_MS);
        assert_eq!(defaults.expected_color_sample, ColorSampleKind::Centre);
        assert_eq!(defaults.tolerance, DEFAULT_TOLERANCE);
    }

    fn frames_launch_defaults() -> CaptureLaunchArgs {
        CaptureLaunchArgs {
            package: "com.example".to_string(),
            activity: None,
            fps: None,
            duration: None,
            ui_dump_package: false,
            expect_first_frame: false,
            no_splash: false,
            expected_color: None,
            color_sample: None,
            tolerance: None,
        }
    }

    fn png_with(pixels: &[[u8; 3]], width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let raw = pixels.iter().flatten().copied().collect::<Vec<_>>();
            writer.write_image_data(&raw).unwrap();
        }
        bytes
    }

    #[test]
    fn color_assertion_samples_pass_and_fail_with_structured_values() {
        let bytes = png_with(
            &[[10, 20, 30], [10, 20, 30], [10, 20, 30], [10, 20, 30]],
            2,
            2,
        );
        let image = androidforge_ui::visual::decode_reader(Cursor::new(&bytes), "test").unwrap();
        let centre = sample_color(&image, None, None, false).unwrap();
        assert_eq!(centre.hex(), "#0A141E");
        let at = sample_color(&image, Some((1, 1)), None, false).unwrap();
        assert_eq!(at.hex(), "#0A141E");
        let region = sample_color(
            &image,
            None,
            Some(ui_color::Region::new(0, 0, 2, 2).unwrap()),
            true,
        )
        .unwrap();
        assert_eq!(region.hex(), "#0A141E");

        let expected = ui_color::parse_hex("#0A141E").unwrap();
        let passed = ui_color::compare(expected, centre, 0);
        assert!(passed.passed);
        assert_eq!(passed.max_channel_delta, 0);
        let failed = ui_color::compare(ui_color::parse_hex("#FF0000").unwrap(), centre, 2);
        assert!(!failed.passed);
        assert_eq!(failed.max_channel_delta, 245);
        assert_eq!(failed.expected.hex(), "#FF0000");
        assert_eq!(failed.actual.hex(), "#0A141E");
        assert_eq!(failed.tolerance, 2);
    }
}
