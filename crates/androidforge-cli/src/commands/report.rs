use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use androidforge_core::analysis::{
    PerformanceBudget, analyze_flaky, compare_reports, compare_with_budget,
};
use androidforge_core::diagnosis::{DiagnosticExplanation, explain_report};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::BuildIdentity;
use androidforge_core::output::CommandSuccess;
use androidforge_core::recurrence::{
    RecurrenceFilter, RecurrenceStatus, build_history, filter_history, find_fingerprint_in_project,
};
use androidforge_core::run::{
    RunContext, RunOptions, RunReport, RunStatus, clear_active_run, get_active_run, list_runs,
    resolve_run_dir, set_active_run,
};
use androidforge_core::util::{parse_duration, read_json};
use serde::Serialize;

use crate::cli::{
    ReportCommand, ReportCompareArgs, ReportExplainArgs, ReportFlakyArgs, ReportHistoryArgs,
    ReportReconcileArgs, ReportRecurrencesArgs, ReportReproArgs, RunCheckpointArgs, RunCommand,
    RunEndArgs, RunEventsArgs, RunExecArgs, RunResumeArgs, RunStartArgs,
};
use crate::commands::bundle;
use crate::commands::common::success;
use crate::context::CliContext;
use crate::junit::import_junit;
use crate::run_scope::tool_versions;

pub async fn run_command(ctx: &CliContext, command: &RunCommand) -> Result<CommandSuccess> {
    match command {
        RunCommand::Start(args) => run_start(ctx, args).await,
        RunCommand::End(args) => run_end(ctx, args).await,
        RunCommand::Status => run_status_command(ctx).await,
        RunCommand::Exec(args) => run_exec(ctx, args).await,
        RunCommand::Checkpoint(args) => run_checkpoint(ctx, args).await,
        RunCommand::Resume(args) => run_resume(ctx, args).await,
        RunCommand::Events(_) => Err(ForgeError::new(
            ErrorCode::UnsupportedOperation,
            "run events is an NDJSON stream and is handled directly by the CLI entry point",
        )),
    }
}

async fn run_checkpoint(ctx: &CliContext, args: &RunCheckpointArgs) -> Result<CommandSuccess> {
    let (run_id, session_id, _) = resolve_active_run(ctx)?;
    let mut run = RunContext::open(&ctx.paths.artifacts_dir, &run_id, ctx.redactor())?;
    let checkpoint = run.checkpoint(&args.name, args.note.as_deref())?;
    let value = serde_json::json!({
        "run_id": run_id,
        "checkpoint": checkpoint,
        "events_path": run.events_path(),
    });
    Ok(success(
        "run.checkpoint",
        value,
        format!(
            "Recorded checkpoint '{}' at step {}.\n",
            args.name,
            run.report
                .checkpoints
                .last()
                .map(|checkpoint| checkpoint.step_index)
                .unwrap_or(0)
        ),
    )
    .with_optional_session(session_id)
    .with_run(run_id))
}

async fn run_resume(ctx: &CliContext, args: &RunResumeArgs) -> Result<CommandSuccess> {
    let run_id = target_run_id(ctx, args.run.as_deref())?;
    let mut run = RunContext::open(&ctx.paths.artifacts_dir, &run_id, ctx.redactor())?;
    let checkpoint = run.resume(args.from_checkpoint.as_deref())?;
    let value = serde_json::json!({
        "run_id": run_id,
        "resumed": true,
        "from_checkpoint": checkpoint,
        "resume_count": run.report.resume_count,
        "session_id": run.report.session_id,
        "events_path": run.events_path(),
    });
    Ok(success(
        "run.resume",
        value,
        format!(
            "Resumed run {run_id} from checkpoint '{}'. Continue with --run {run_id}.\n",
            checkpoint.name
        ),
    )
    .with_optional_session(run.report.session_id.clone())
    .with_run(run_id))
}

pub async fn stream_events(cli: &crate::cli::Cli, args: &RunEventsArgs) -> Result<()> {
    use std::io::Write;
    let ctx = CliContext::new(cli)?;
    let run_id = target_run_id(&ctx, args.run.as_deref())?;
    let dir = androidforge_core::run::resolve_run_dir(&ctx.paths.artifacts_dir, &run_id)?;
    let events_path = dir.join("events.ndjson");
    let timeout = ctx.parse_timeout(Some(&args.timeout), 300)?;
    let started = std::time::Instant::now();
    let mut printed = 0usize;
    loop {
        if let Ok(text) = std::fs::read_to_string(&events_path) {
            for line in text.lines().skip(printed) {
                println!("{line}");
            }
            printed = text.lines().count();
            let _ = std::io::stdout().flush();
        }
        if !args.follow {
            break;
        }
        let terminal = std::fs::read(dir.join("report.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<RunReport>(&bytes).ok())
            .is_some_and(|report| report.status != RunStatus::Running);
        if terminal && printed > 0 {
            break;
        }
        if started.elapsed() >= timeout {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(())
}

async fn report_repro(ctx: &CliContext, args: &ReportReproArgs) -> Result<CommandSuccess> {
    let dir = androidforge_core::run::resolve_run_dir(&ctx.paths.artifacts_dir, &args.run)?;
    let report: RunReport = read_json(&dir.join("report.json"))?;
    let mut script = String::new();
    script.push_str("# AndroidForge reproduction script\n");
    script.push_str(&format!("# run: {}\n", report.run_id));
    script.push_str(&format!("# command: {}\n", report.command));
    script.push_str(&format!("# project: {}\n", report.project_root));
    script.push_str(&format!("# status: {:?}\n\n", report.status));
    if let Some(invocation) = &report.invocation {
        script.push_str(&format!(
            "Set-Location -LiteralPath '{}'\n",
            quote_powershell(&invocation.cwd)
        ));
        script.push_str("& ");
        for (index, arg) in invocation.argv.iter().enumerate() {
            if index > 0 {
                script.push(' ');
            }
            script.push('\'');
            script.push_str(&quote_powershell(arg));
            script.push('\'');
        }
        script.push('\n');
    } else {
        script.push_str("# Legacy run: no exact argv was recorded.\n");
        for step in &report.steps {
            script.push_str(&format!(
                "# step {} [{}]: {}\n",
                step.id,
                format!("{:?}", step.status).to_ascii_lowercase(),
                step.command
            ));
        }
        script.push_str(&format!(
            "# Re-run the failing commands with --run {}\n",
            report.run_id
        ));
    }
    let output = args.output.as_ref().map(|path| {
        if path.is_absolute() {
            path.clone()
        } else {
            ctx.paths.cwd.join(path)
        }
    });
    if let Some(path) = &output {
        androidforge_core::util::write_atomic(path, script.as_bytes())?;
    }
    Ok(success(
        "report.repro",
        serde_json::json!({
            "run_id": report.run_id,
            "output": output,
            "script": script,
            "exact_argv": report.invocation.is_some(),
        }),
        if let Some(path) = output {
            format!("Reproduction script written to {}\n", path.display())
        } else {
            script.clone()
        },
    )
    .with_run(report.run_id))
}

fn quote_powershell(value: &str) -> String {
    value.replace('\'', "''")
}

/// Normalise and bound free-form report labels. Empty, oversized and control
/// character tags are rejected so stored evidence cannot be corrupted by
/// terminal control sequences.
pub(crate) fn validate_report_tags(tags: &[String]) -> Result<Vec<String>> {
    tags.iter()
        .map(|tag| {
            let tag = tag.trim();
            if tag.is_empty() {
                return Err(ForgeError::invalid_argument("--tag must not be empty"));
            }
            if tag.chars().count() > 64 {
                return Err(ForgeError::invalid_argument(
                    "--tag must be at most 64 characters",
                ));
            }
            if tag.chars().any(char::is_control) {
                return Err(ForgeError::invalid_argument(
                    "--tag must not contain control characters",
                ));
            }
            Ok(tag.to_string())
        })
        .collect()
}

/// `--since` accepts an absolute RFC3339 instant or a relative duration such
/// as `30d`, `2h` or `500ms`. Relative values are resolved once, at parse time.
pub(crate) fn parse_since_filter(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Err(ForgeError::invalid_argument("--since must not be empty"));
    }
    if let Ok(instant) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(Some(instant.with_timezone(&chrono::Utc).to_rfc3339()));
    }
    let duration = parse_duration(value).map_err(ForgeError::invalid_argument)?;
    let since = chrono::Utc::now()
        - chrono::Duration::from_std(duration)
            .map_err(|_| ForgeError::invalid_argument("--since is outside the supported range"))?;
    Ok(Some(since.to_rfc3339()))
}

const MAX_META_KEY_LEN: usize = 64;
const MAX_META_VALUE_LEN: usize = 1024;
const SECRET_KEY_MARKERS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "api-key",
    "credential",
    "private_key",
    "private-key",
    "access_key",
    "access-key",
    "authorization",
];

/// Parse repeatable `--meta KEY=VALUE` pairs. Keys are restricted to a small
/// identifier alphabet and checked against secret-looking names so metadata
/// never becomes an invitation to record credentials in reports.
pub(crate) fn parse_run_meta(pairs: &[String]) -> Result<BTreeMap<String, String>> {
    let mut meta = BTreeMap::new();
    for pair in pairs {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(ForgeError::invalid_argument(format!(
                "--meta '{pair}' must use KEY=VALUE form"
            ))
            .with_suggestion("Example: --meta build=1.2.3 --meta branch=main"));
        };
        let key = key.trim();
        if key.is_empty() || key.chars().count() > MAX_META_KEY_LEN {
            return Err(ForgeError::invalid_argument(format!(
                "--meta key must be between 1 and {MAX_META_KEY_LEN} characters"
            )));
        }
        if !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        {
            return Err(ForgeError::invalid_argument(format!(
                "--meta key '{key}' may contain only letters, digits, '_', '-' and '.'"
            )));
        }
        let lowered = key.to_ascii_lowercase();
        if SECRET_KEY_MARKERS
            .iter()
            .any(|marker| lowered.contains(marker))
        {
            return Err(ForgeError::invalid_argument(format!(
                "--meta key '{key}' looks like a secret and is not allowed"
            ))
            .with_suggestion("Metadata is stored in reports; keep credentials out of it."));
        }
        let value = value.trim();
        if value.is_empty() || value.chars().count() > MAX_META_VALUE_LEN {
            return Err(ForgeError::invalid_argument(format!(
                "--meta value for '{key}' must be between 1 and {MAX_META_VALUE_LEN} characters"
            )));
        }
        if value.chars().any(char::is_control) {
            return Err(ForgeError::invalid_argument(format!(
                "--meta value for '{key}' must not contain control characters"
            )));
        }
        meta.insert(key.to_string(), value.to_string());
    }
    Ok(meta)
}

fn budget_number(value: f64, label: &str, allow_negative: bool) -> Result<f64> {
    if !value.is_finite() || (!allow_negative && value < 0.0) {
        return Err(ForgeError::invalid_argument(format!(
            "{label} must be a finite {}number",
            if allow_negative { "" } else { "non-negative " }
        )));
    }
    Ok(value)
}

fn validate_budget(budget: &PerformanceBudget) -> Result<()> {
    for (name, value) in [
        ("startup_ms", budget.startup_ms),
        ("jank_percent", budget.jank_percent),
        ("janky_frames", budget.janky_frames),
        ("max_frame_ms", budget.max_frame_ms),
        ("apk_size_bytes", budget.apk_size_bytes),
        ("failed_tests", budget.failed_tests),
    ] {
        if let Some(value) = value {
            budget_number(value, &format!("budget field '{name}'"), false)?;
        }
    }
    for (name, value) in [
        ("startup_delta_ms", budget.startup_delta_ms),
        ("jank_percent_delta", budget.jank_percent_delta),
        ("max_frame_delta_ms", budget.max_frame_delta_ms),
        ("apk_size_delta_bytes", budget.apk_size_delta_bytes),
        ("failed_tests_delta", budget.failed_tests_delta),
    ] {
        if let Some(value) = value {
            budget_number(value, &format!("budget field '{name}'"), true)?;
        }
    }
    for (name, value) in &budget.metrics {
        budget_number(*value, &format!("budget metric '{name}'"), false)?;
    }
    for (name, value) in &budget.metric_deltas {
        budget_number(*value, &format!("budget metric delta '{name}'"), true)?;
    }
    Ok(())
}

/// Build the optional budget from `--budget` JSON plus convenience thresholds.
/// Convenience flags override the corresponding JSON fields; no flags means
/// the caller keeps the original no-budget comparison behavior.
fn build_compare_budget(args: &ReportCompareArgs) -> Result<Option<PerformanceBudget>> {
    let mut budget = match &args.budget {
        Some(text) => serde_json::from_str::<PerformanceBudget>(text).map_err(|error| {
            ForgeError::invalid_argument(format!("invalid --budget JSON: {error}"))
        })?,
        None => PerformanceBudget::default(),
    };
    if args.max_p99_ms.is_some() && args.max_frame_ms.is_some() {
        return Err(ForgeError::invalid_argument(
            "pass either --max-p99-ms or --max-frame-ms; use --budget for separate frame thresholds",
        ));
    }
    if let Some(value) = args.max_startup_ms {
        budget.startup_ms = Some(budget_number(value, "--max-startup-ms", false)?);
    }
    if let Some(value) = args.max_jank_percent {
        budget.jank_percent = Some(budget_number(value, "--max-jank-percent", false)?);
    }
    if let Some(value) = args.max_p99_ms.or(args.max_frame_ms) {
        let label = if args.max_p99_ms.is_some() {
            "--max-p99-ms"
        } else {
            "--max-frame-ms"
        };
        budget.max_frame_ms = Some(budget_number(value, label, false)?);
    }
    if let Some(value) = args.max_apk_size_bytes {
        budget.apk_size_bytes = Some(budget_number(value, "--max-apk-size-bytes", false)?);
    }
    if let Some(value) = args.max_failed_tests {
        budget.failed_tests = Some(budget_number(value, "--max-failed-tests", false)?);
    }
    if budget.is_empty() {
        return Ok(None);
    }
    validate_budget(&budget)?;
    Ok(Some(budget))
}

fn target_run_id(ctx: &CliContext, explicit: Option<&str>) -> Result<String> {
    if let Some(run) = explicit.filter(|run| !run.trim().is_empty()) {
        return Ok(run.to_string());
    }
    if let Some(run) = ctx.run_arg.as_deref().filter(|run| !run.trim().is_empty()) {
        return Ok(run.to_string());
    }
    if let Ok(run) = std::env::var("ANDROIDFORGE_RUN_ID")
        && !run.trim().is_empty()
    {
        return Ok(run);
    }
    let session_id = ctx.session_id()?;
    get_active_run(&ctx.paths, session_id.as_deref())?
        .map(|run| run.run_id)
        .ok_or_else(|| {
            ForgeError::new(ErrorCode::InvalidArgument, "no run was specified or active")
                .with_suggestion("Pass a run id or start one with `androidforge run start`.")
        })
}

async fn run_start(ctx: &CliContext, args: &RunStartArgs) -> Result<CommandSuccess> {
    let tags = validate_report_tags(&args.tags)?;
    let meta = parse_run_meta(&args.meta)?;
    if ctx.dry_run {
        return Ok(success(
            "run.start",
            serde_json::json!({
                "dry_run": true,
                "name": args.name,
                "tags": tags,
                "meta": meta,
            }),
            format!("Would start run '{}'.\n", args.name),
        ));
    }
    let session = ctx.load_session(false)?;
    let mut options = RunOptions::new(
        ctx.paths.artifacts_dir.clone(),
        ctx.paths.project_root.clone(),
        ctx.config.project_name(),
        "run.start",
    );
    options.name = Some(args.name.clone());
    options.tags = tags.clone();
    options.meta = meta.clone();
    options.project_config = ctx.paths.project_config_path.clone();
    options.session_id = session.as_ref().map(|session| session.id.clone());
    options.build = crate::run_scope::build_identity_for_run(ctx, session.as_ref()).await;
    options.tool_versions = tool_versions(ctx);
    let run = RunContext::create(options, ctx.redactor())?;
    set_active_run(
        &ctx.paths,
        session.as_ref().map(|session| session.id.as_str()),
        &run.run_id,
        &ctx.paths.project_root,
    )?;
    let human = format!(
        "Run {}\nreport: {}\n\nUse --run {} on subsequent commands, or export:\n  ANDROIDFORGE_RUN_ID={}\n",
        run.run_id,
        run.report_path().display(),
        run.run_id,
        run.run_id
    );
    Ok(success(
        "run.start",
        serde_json::json!({
            "run_id": run.run_id,
            "report_path": run.report_path(),
            "artifact_directory": run.dir,
            "tags": tags,
            "meta": meta,
            "env": {"ANDROIDFORGE_RUN_ID": run.run_id},
        }),
        human,
    )
    .with_optional_session(session.map(|session| session.id))
    .with_run(run.run_id))
}

async fn run_end(ctx: &CliContext, args: &RunEndArgs) -> Result<CommandSuccess> {
    let (run_id, session_id, _) = resolve_active_run(ctx)?;
    let mut run = RunContext::open(&ctx.paths.artifacts_dir, &run_id, ctx.redactor())?;
    if run.report.status != RunStatus::Running {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            format!("run {run_id} is already {:?}", run.report.status),
        )
        .with_suggestion("Start a new run with `androidforge run start --name <name>`.")
        .with_context_string("run_id", run_id));
    }
    let failed_steps = run.report.failed_steps();
    let requested = args.status.as_deref();
    let status = match requested {
        None if failed_steps > 0 => RunStatus::Failed,
        None => RunStatus::Passed,
        Some("passed" | "pass" | "success") if failed_steps > 0 && !args.allow_failures => {
            return Err(ForgeError::invalid_argument(format!(
                "run {run_id} has {failed_steps} failed step(s); refusing to finalize it as passed"
            ))
            .with_context_string("run_id", &run_id)
            .with_context("failed_steps", serde_json::json!(failed_steps))
            .with_suggestion("Inspect the failed steps with `androidforge report show <run>`.")
            .with_suggestion("Use `--status failed` to record the failure.")
            .with_suggestion(
                "Use `--allow-failures` only when deliberately keeping a failed run marked passed.",
            ));
        }
        Some("passed" | "pass" | "success") => RunStatus::Passed,
        Some("failed" | "fail" | "failure") => RunStatus::Failed,
        Some("partial") => RunStatus::Partial,
        Some("cancelled" | "canceled") => RunStatus::Cancelled,
        Some("interrupted") => RunStatus::Interrupted,
        Some("timed-out" | "timed_out" | "timeout") => RunStatus::TimedOut,
        Some("abandoned") => RunStatus::Abandoned,
        Some(other) => {
            return Err(ForgeError::invalid_argument(format!(
                "unknown run status '{other}'; use passed, failed, partial, cancelled, interrupted, timed-out or abandoned"
            )));
        }
    };
    let default_exit = match status {
        RunStatus::Passed => 0,
        RunStatus::Failed => 6,
        RunStatus::Partial => 10,
        RunStatus::Cancelled => 130,
        RunStatus::Interrupted | RunStatus::Abandoned => 11,
        RunStatus::TimedOut => 124,
        RunStatus::Running => 0,
    };
    let exit_code = args
        .exit_code
        .filter(|code| *code != 0 || status == RunStatus::Passed)
        .unwrap_or(default_exit);
    run.finalize(status, exit_code)?;
    // `--run` may name the session's active run. Clear the pointer only when
    // it still points at the run we just finalized; ending one run must not
    // forget a different active run in the same session.
    if get_active_run(&ctx.paths, session_id.as_deref())?
        .is_some_and(|active| active.run_id == run_id)
    {
        clear_active_run(&ctx.paths, session_id.as_deref())?;
    }
    let report = serde_json::to_value(&run.report)?;
    if status != RunStatus::Passed {
        let code = if status == RunStatus::Partial {
            ErrorCode::PartialFailure
        } else {
            ErrorCode::CommandFailed
        };
        return Err(ForgeError::new(
            code,
            format!(
                "run {run_id} finalized as {:?} with {} failed step(s)",
                status, failed_steps
            ),
        )
        .with_context("report", report)
        .with_context_string("report_path", run.report_path().to_string_lossy())
        .with_context_string("run_id", &run_id)
        .with_suggestion("Inspect `report.json` and the failure artifacts before retrying.")
        .with_exit_code(exit_code));
    }
    Ok(success(
        "run.end",
        report,
        format!(
            "Run {run_id} finalized as {:?} (exit {}).\nReport: {}\n",
            status,
            exit_code,
            run.report_path().display()
        ),
    )
    .with_optional_session(session_id)
    .with_run(run_id))
}

async fn run_status_command(ctx: &CliContext) -> Result<CommandSuccess> {
    let (run_id, session_id, _) = resolve_active_run(ctx)?;
    let run = RunContext::open(&ctx.paths.artifacts_dir, &run_id, ctx.redactor())?;
    let summary = summary_json(&run.report, &run.dir);
    Ok(
        success("run.status", summary, report_human(&run.report, &run.dir))
            .with_optional_session(session_id)
            .with_run(run_id),
    )
}

async fn run_exec(ctx: &CliContext, args: &RunExecArgs) -> Result<CommandSuccess> {
    let session = ctx.require_session()?;
    let device = ctx.resolve_device(Some(&session)).await?;
    let mut options =
        androidforge_automation::ExternalRunOptions::new(args.program.clone(), args.name.clone());
    options.timeout = ctx.parse_timeout(
        args.timeout.as_deref(),
        ctx.config.file.timeouts.command_secs,
    )?;
    options.capture_logcat = args.capture_logcat;
    options.capture_screen = args.capture_screen;
    options.trace = ctx.trace;
    options.package = ctx.config.package().map(str::to_string);
    execute_in_run!(
        ctx,
        "run.exec",
        Some("run exec"),
        Some(&session),
        Some(&device),
        false,
        |run| async {
            let app = ctx.app_manager()?;
            let ui = ctx.ui_manager()?;
            let result = androidforge_automation::run_external(
                &app.adb,
                &app,
                &ui,
                &session,
                run,
                &ctx.paths.project_root,
                &options,
            )
            .await?;
            if !result.success {
                return Err(ForgeError::new(
                    if result.timed_out {
                        ErrorCode::CommandTimeout
                    } else {
                        ErrorCode::CommandFailed
                    },
                    format!(
                        "external command '{}' {}",
                        result.command,
                        if result.timed_out {
                            "timed out"
                        } else {
                            "failed"
                        }
                    ),
                )
                .with_context(
                    "external_result",
                    serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
                )
                .with_artifacts(result.artifacts.clone())
                .with_suggestion("Inspect stdout/stderr artifacts and the run report.")
                .session(session.id.clone())
                .device(device.serial.clone()));
            }
            Ok(success(
                "run.exec",
                serde_json::to_value(&result)?,
                format!(
                    "External command completed in {}ms (exit {}).\n",
                    result.duration_ms,
                    result
                        .exit_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                ),
            )
            .with_artifacts(result.artifacts)
            .with_session(session.id.clone())
            .with_device(device.serial.clone()))
        },
    )
    .await
}

pub async fn report(ctx: &CliContext, command: &ReportCommand) -> Result<CommandSuccess> {
    match command {
        ReportCommand::List {
            limit,
            all,
            tags,
            since,
        } => {
            let scope = resolve_report_scope(ctx, Some(tags), since.as_deref())?;
            let mut runs: Vec<(androidforge_core::run::RunSummary, PathBuf)> = if *all {
                let mut merged: BTreeMap<String, (androidforge_core::run::RunSummary, PathBuf)> =
                    BTreeMap::new();
                for root in candidate_artifact_roots(ctx) {
                    for run in list_runs(&root).unwrap_or_default() {
                        let path = root.join(&run.path);
                        merged.entry(run.run_id.clone()).or_insert((run, path));
                    }
                }
                merged.into_values().collect()
            } else {
                let root = &ctx.paths.artifacts_dir;
                list_runs(root)?
                    .into_iter()
                    .map(|run| {
                        let path = root.join(&run.path);
                        (run, path)
                    })
                    .collect()
            };
            // RunSummary does not carry scope fields, so apply the same filter
            // the latest-run resolver uses to the stored report itself.
            runs.retain(|(_, path)| {
                read_json::<RunReport>(&path.join("report.json"))
                    .is_ok_and(|report| scope.matches(&report))
            });
            runs.sort_by(|(left, _), (right, _)| {
                right
                    .started_at
                    .cmp(&left.started_at)
                    .then_with(|| right.run_id.cmp(&left.run_id))
            });
            let total = runs.len();
            let mut human = String::new();
            if total == 0 {
                human.push_str("No runs found.\n");
            } else {
                human.push_str(&format!(
                    "{} run(s) in {}{}\n",
                    total,
                    if *all {
                        "all discovered artifact stores".to_string()
                    } else {
                        ctx.paths.artifacts_dir.join("runs").display().to_string()
                    },
                    scope.describe()
                ));
                for (run, _) in runs.iter().take(*limit) {
                    human.push_str(&format!(
                        "  {}  {:?}  failed={}  {}\n",
                        run.run_id, run.status, run.failed_steps, run.command
                    ));
                }
            }
            let runs = runs
                .into_iter()
                .take(*limit)
                .map(|(run, _)| run)
                .collect::<Vec<_>>();
            Ok(success(
                "report.list",
                serde_json::json!({
                    "count": total,
                    "limit": limit,
                    "all": all,
                    "tags": scope.tags,
                    "since": scope.since_rfc3339,
                    "runs": runs,
                }),
                human,
            ))
        }
        ReportCommand::Show { run } => {
            let dir = resolve_report_dir(ctx, run)?;
            let report: RunReport = androidforge_core::util::read_json(&dir.join("report.json"))?;
            Ok(success(
                "report.show",
                serde_json::to_value(&report)?,
                report_human(&report, &dir),
            ))
        }
        ReportCommand::Path { run } => {
            let dir = resolve_report_dir(ctx, run)?;
            Ok(success(
                "report.path",
                serde_json::json!({
                    "run_id": dir.file_name().map(|name| name.to_string_lossy().into_owned()),
                    "path": dir,
                    "report": dir.join("report.json"),
                    "manifest": dir.join("manifest.json"),
                    "metadata": dir.join("metadata.json"),
                }),
                format!("{}\n", dir.display()),
            ))
        }
        ReportCommand::Prune(args) => prune(ctx, args).await,
        ReportCommand::Reconcile(args) => reconcile(ctx, args).await,
        ReportCommand::Compare(args) => compare(ctx, args).await,
        ReportCommand::Flaky(args) => flaky(ctx, args).await,
        ReportCommand::Explain(args) => explain(ctx, args).await,
        ReportCommand::History(args) => report_history(ctx, args).await,
        ReportCommand::Recurrences(args) => report_recurrences(ctx, args).await,
        ReportCommand::Bundle(args) => bundle::bundle_command(ctx, args).await,
        ReportCommand::Repro(args) => report_repro(ctx, args).await,
        ReportCommand::Import { command } => match command {
            crate::cli::ReportImportCommand::Junit { paths } => {
                import_junit_report(ctx, paths).await
            }
        },
    }
}

async fn import_junit_report(
    ctx: &CliContext,
    inputs: &[std::path::PathBuf],
) -> Result<CommandSuccess> {
    let files = collect_junit_files(ctx, inputs)?;
    let active = resolve_optional_run(ctx)?;
    if ctx.dry_run {
        return Ok(success(
            "report.import.junit",
            serde_json::json!({"dry_run": true, "files": files}),
            format!("Would import {} JUnit XML file(s).\n", files.len()),
        ));
    }
    let mut implicit = false;
    let mut run = if let Some(run_id) = &active {
        RunContext::open(&ctx.paths.artifacts_dir, run_id, ctx.redactor())?
    } else {
        let mut options = RunOptions::new(
            ctx.paths.artifacts_dir.clone(),
            ctx.paths.project_root.clone(),
            ctx.config.project_name(),
            "report.import.junit",
        );
        options.name = Some("junit-import".to_string());
        options.project_config = ctx.paths.project_config_path.clone();
        options.build = BuildIdentity {
            package: ctx.config.debug_package().map(str::to_string),
            ..BuildIdentity::default()
        };
        options.tool_versions = tool_versions(ctx);
        implicit = true;
        RunContext::create(options, ctx.redactor())?
    };
    let mut passed = true;
    let mut suites = 0usize;
    let mut tests = 0usize;
    let mut failures = 0usize;
    let mut errors = 0usize;
    let mut skipped = 0usize;
    let mut cases = Vec::new();
    let mut artifacts = Vec::new();
    for file in &files {
        let summary = import_junit(&mut run, file)?;
        passed &= summary.passed;
        suites += summary.suites;
        tests += summary.tests;
        failures += summary.failures;
        errors += summary.errors;
        skipped += summary.skipped;
        cases.extend(summary.cases);
        artifacts.push(summary.artifact);
    }
    let result = serde_json::json!({
        "passed": passed,
        "files": artifacts,
        "suites": suites,
        "tests": tests,
        "failures": failures,
        "errors": errors,
        "skipped": skipped,
        "cases": cases,
    });
    if !passed {
        let error = ForgeError::new(
            ErrorCode::UiAssertionFailed,
            format!("JUnit import reports {failures} failure(s) and {errors} error(s)"),
        )
        .with_context("junit", result.clone())
        .with_artifacts(artifacts.clone())
        .with_suggestion("Inspect the merged run report and the original JUnit XML.")
        .with_context_string("run_id", run.run_id.clone());
        if implicit {
            run.finalize_failure(&error)?;
        }
        return Err(error);
    }
    if implicit {
        run.finalize_pass()?;
    }
    Ok(success(
        "report.import.junit",
        result,
        format!(
            "Imported {tests} JUnit test(s) from {} file(s): {failures} failure(s), {errors} error(s), {skipped} skipped.\n",
            files.len()
        ),
    )
    .with_artifacts(artifacts)
    .with_run(run.run_id))
}

fn collect_junit_files(
    ctx: &CliContext,
    inputs: &[std::path::PathBuf],
) -> Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();
    for input in inputs {
        let path = if input.is_absolute() {
            input.clone()
        } else {
            ctx.paths.cwd.join(input)
        };
        if path.is_dir() {
            collect_xml_files(&path, &mut files)?;
        } else if path.is_file() {
            files.push(path);
        } else {
            return Err(ForgeError::new(
                ErrorCode::JunitInvalid,
                format!(
                    "JUnit input {} was not found; pass an XML file or a directory",
                    path.display()
                ),
            )
            .with_suggestion(
                "Gradle writes reports under app/build/test-results/<variant>/ as a directory of XML files.",
            ));
        }
    }
    files.sort();
    files.dedup();
    if files.is_empty() {
        return Err(ForgeError::new(
            ErrorCode::JunitInvalid,
            "no JUnit XML files were found",
        ));
    }
    Ok(files)
}

fn collect_xml_files(
    directory: &std::path::Path,
    files: &mut Vec<std::path::PathBuf>,
) -> Result<()> {
    let entries = std::fs::read_dir(directory).map_err(|error| {
        ForgeError::new(
            ErrorCode::JunitInvalid,
            format!(
                "could not read JUnit directory {}: {error}",
                directory.display()
            ),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            ForgeError::new(
                ErrorCode::JunitInvalid,
                format!("could not read a JUnit directory entry: {error}"),
            )
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_xml_files(&path, files)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("xml") {
            files.push(path);
        }
    }
    Ok(())
}

fn report_age_seconds(started_at: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(started_at)
        .map(|started| {
            chrono::Utc::now()
                .signed_duration_since(started.with_timezone(&chrono::Utc))
                .num_seconds()
        })
        .unwrap_or_default()
}

fn parse_reconcile_outcome(value: Option<&str>) -> Result<(RunStatus, i32, String)> {
    match value.unwrap_or("interrupted") {
        "interrupted" => Ok((
            RunStatus::Interrupted,
            11,
            "run was interrupted before it finalized".to_string(),
        )),
        "cancelled" | "canceled" => Ok((
            RunStatus::Cancelled,
            130,
            "operation was cancelled before the run finalized".to_string(),
        )),
        "timed-out" | "timed_out" | "timeout" => Ok((
            RunStatus::TimedOut,
            124,
            "operation timed out before the run finalized".to_string(),
        )),
        "abandoned" => Ok((
            RunStatus::Abandoned,
            11,
            "run was abandoned without a live owner".to_string(),
        )),
        other => Err(ForgeError::invalid_argument(format!(
            "unknown reconcile outcome '{other}'; use interrupted, cancelled, timed-out or abandoned"
        ))),
    }
}

fn should_auto_reconcile(run: &RunContext) -> Result<Option<String>> {
    let age = report_age_seconds(&run.report.started_at);
    if let Some(pid) = run.metadata.creator_pid {
        if androidforge_process::process_exists(pid) {
            return Ok(None);
        }
        return Ok(Some(format!(
            "creator pid {pid} is no longer running (run age {age}s)"
        )));
    }
    if age > 2 * 60 * 60 {
        return Ok(Some(format!(
            "legacy running report is {age}s old and has no creator pid"
        )));
    }
    Ok(None)
}

async fn reconcile(ctx: &CliContext, args: &ReportReconcileArgs) -> Result<CommandSuccess> {
    if args.run.is_some() && args.latest_running {
        return Err(ForgeError::invalid_argument(
            "choose --run or --latest-running, not both",
        ));
    }
    let explicit = args.run.is_some() || args.latest_running;
    if args.outcome.is_some() && !explicit {
        return Err(
            ForgeError::invalid_argument("--outcome requires --run or --latest-running")
                .with_suggestion(
                    "Run without --outcome to reconcile only runs whose creator process is gone.",
                ),
        );
    }
    let (target_status, exit_code, default_reason) =
        parse_reconcile_outcome(args.outcome.as_deref())?;
    let summaries = list_runs(&ctx.paths.artifacts_dir)?;
    let targets: Vec<(String, Option<String>)> = if let Some(spec) = &args.run {
        vec![(spec.clone(), None)]
    } else if args.latest_running {
        summaries
            .iter()
            .filter(|run| run.status == RunStatus::Running)
            .max_by(|left, right| left.started_at.cmp(&right.started_at))
            .map(|run| vec![(run.run_id.clone(), None)])
            .unwrap_or_default()
    } else {
        summaries
            .iter()
            .filter(|run| run.status == RunStatus::Running)
            .map(|run| (run.run_id.clone(), None))
            .collect()
    };

    let mut changed = Vec::new();
    let mut skipped = Vec::new();
    for (spec, _) in targets {
        let dir = match resolve_report_dir(ctx, &spec) {
            Ok(dir) => dir,
            Err(error) if args.run.is_some() => return Err(error),
            Err(error) => {
                skipped.push(serde_json::json!({
                    "target": spec,
                    "reason": error.message().to_string(),
                }));
                continue;
            }
        };
        let mut run = match RunContext::open_dir(dir, ctx.redactor()) {
            Ok(run) => run,
            Err(error) => {
                skipped.push(serde_json::json!({
                    "target": spec,
                    "reason": error.message().to_string(),
                }));
                continue;
            }
        };
        if run.report.status != RunStatus::Running {
            skipped.push(serde_json::json!({
                "target": run.run_id,
                "reason": format!("run is already {:?}", run.report.status),
            }));
            continue;
        }
        let reason = if explicit {
            default_reason.clone()
        } else {
            match should_auto_reconcile(&run)? {
                Some(reason) => reason,
                None => {
                    skipped.push(serde_json::json!({
                        "target": run.run_id,
                        "reason": "creator process is still alive; not reconciling automatically",
                    }));
                    continue;
                }
            }
        };
        let previous = run.report.status;
        if !ctx.dry_run {
            run.add_note(format!(
                "reconciled as {}: {reason}",
                format!("{target_status:?}").to_lowercase()
            ));
            run.finalize_with_reason(target_status, exit_code, &reason)?;
        }
        changed.push(serde_json::json!({
            "run_id": run.run_id,
            "previous_status": previous,
            "status": target_status,
            "reason": reason,
            "dry_run": ctx.dry_run,
        }));
    }
    let result = serde_json::json!({
        "dry_run": ctx.dry_run,
        "reconciled": changed,
        "skipped": skipped,
        "scanned": summaries.len(),
    });
    let human = format!(
        "{} {} running run(s); {} skipped; {} scanned.\n",
        if ctx.dry_run {
            "Would reconcile"
        } else {
            "Reconciled"
        },
        changed.len(),
        skipped.len(),
        summaries.len()
    );
    Ok(success("report.reconcile", result, human))
}

async fn compare(ctx: &CliContext, args: &ReportCompareArgs) -> Result<CommandSuccess> {
    let budget = build_compare_budget(args)?;
    if args.fail_on_regression && budget.is_none() {
        return Err(ForgeError::invalid_argument(
            "--fail-on-regression requires a budget threshold; pass --budget or a --max-* flag",
        ));
    }
    let scope = resolve_report_scope(ctx, Some(&args.tags), args.since.as_deref())?;
    let current_dir = resolve_report_dir(ctx, &args.run)?;
    let current = RunContext::open_dir(current_dir, ctx.redactor())?;
    let baseline_id = if let Some(baseline) = &args.baseline {
        baseline.clone()
    } else {
        let root = &ctx.paths.artifacts_dir;
        let mut candidates = Vec::new();
        for summary in list_runs(root)? {
            if summary.run_id == current.run_id
                || summary.status == RunStatus::Running
                || summary.project != current.report.project
                || summary.command != current.report.command
            {
                continue;
            }
            let path = root.join(&summary.path);
            let Ok(report) = read_json::<RunReport>(&path.join("report.json")) else {
                continue;
            };
            if scope.matches(&report) {
                candidates.push(summary);
            }
        }
        candidates
            .iter()
            .find(|run| run.status == RunStatus::Passed)
            .or_else(|| candidates.first())
            .map(|run| run.run_id.clone())
            .ok_or_else(|| {
                ForgeError::new(
                    ErrorCode::InvalidArgument,
                    format!(
                        "no compatible baseline run found for {} ({}){}",
                        current.report.project,
                        current.report.command,
                        scope.describe()
                    ),
                )
                .with_suggestion(
                    "Pass --baseline <run>, relax the filters, or run the same scenario again.",
                )
            })?
    };
    let baseline_dir = resolve_report_dir(ctx, &baseline_id)?;
    let baseline = RunContext::open_dir(baseline_dir, ctx.redactor())?;
    let comparison = match &budget {
        Some(budget) => compare_with_budget(&current.report, &baseline.report, budget),
        None => compare_reports(&current.report, &baseline.report),
    };
    let mut human = format!(
        "Compared {} with baseline {}\nstatus: {} -> {}\nduration: {}ms -> {}ms ({:+}ms)\nnew failures: {}\nresolved failures: {}\n",
        comparison.current_run_id,
        comparison.baseline_run_id,
        format!("{:?}", comparison.baseline_status).to_lowercase(),
        format!("{:?}", comparison.current_status).to_lowercase(),
        comparison.baseline_duration_ms,
        comparison.current_duration_ms,
        comparison.duration_delta_ms,
        if comparison.new_failures.is_empty() {
            "none".to_string()
        } else {
            comparison.new_failures.join(", ")
        },
        if comparison.resolved_failures.is_empty() {
            "none".to_string()
        } else {
            comparison.resolved_failures.join(", ")
        }
    );
    if comparison.budget.is_some() {
        human.push_str(&format!(
            "budget: {} of {} check(s) passed\n",
            comparison
                .budget_checks
                .iter()
                .filter(|check| check.passed)
                .count(),
            comparison.budget_checks.len()
        ));
        for violation in &comparison.budget_violations {
            human.push_str(&format!(
                "  VIOLATION {}: {}\n",
                violation.metric, violation.reason
            ));
        }
    }
    if args.fail_on_regression && !comparison.budget_violations.is_empty() {
        return Err(ForgeError::new(
            ErrorCode::PerformanceRegression,
            format!(
                "{} performance budget violation(s) comparing {} with {}",
                comparison.budget_violations.len(),
                comparison.current_run_id,
                comparison.baseline_run_id
            ),
        )
        .with_context("comparison", serde_json::to_value(&comparison)?)
        .with_context(
            "budget_violations",
            serde_json::to_value(&comparison.budget_violations)?,
        )
        .with_context_string("run_id", &comparison.current_run_id)
        .with_context_string("baseline_run_id", &comparison.baseline_run_id)
        .with_suggestion(
            "Inspect error.context.budget_checks and the stored performance evidence.",
        ));
    }
    Ok(success(
        "report.compare",
        serde_json::to_value(&comparison)?,
        human,
    ))
}

async fn flaky(ctx: &CliContext, args: &ReportFlakyArgs) -> Result<CommandSuccess> {
    let summaries = list_runs(&ctx.paths.artifacts_dir)?;
    let mut reports = Vec::new();
    for summary in summaries.into_iter().take(args.limit.max(1)) {
        if let Ok(run) = RunContext::open(&ctx.paths.artifacts_dir, &summary.run_id, ctx.redactor())
        {
            reports.push(run.report);
        }
    }
    let report = analyze_flaky(&reports, args.min_runs);
    let human = format!(
        "Analyzed {} run(s); {} flaky step(s).\n{}",
        report.runs_analyzed,
        report
            .steps
            .iter()
            .filter(|step| step.classification == "flaky")
            .count(),
        report
            .steps
            .iter()
            .filter(|step| step.classification == "flaky")
            .map(|step| format!(
                "  {}: {} ({}/{} failed, {} executions)\n",
                step.step_name, step.command, step.failures, step.executions, step.executions
            ))
            .collect::<String>()
    );
    Ok(success(
        "report.flaky",
        serde_json::to_value(&report)?,
        human,
    ))
}

fn load_reports_for_memory(ctx: &CliContext, include_all: bool) -> (Vec<RunReport>, usize) {
    let mut reports: BTreeMap<String, RunReport> = BTreeMap::new();
    let mut unreadable = 0usize;
    let roots = if include_all {
        candidate_artifact_roots(ctx)
    } else {
        vec![ctx.paths.artifacts_dir.clone()]
    };
    for root in roots {
        let runs_dir = root.join("runs");
        let Ok(entries) = std::fs::read_dir(&runs_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path().join("report.json");
            if !path.is_file() {
                continue;
            }
            match androidforge_core::util::read_json::<RunReport>(&path) {
                Ok(report) => {
                    reports.entry(report.run_id.clone()).or_insert(report);
                }
                Err(_) => unreadable += 1,
            }
        }
    }
    (reports.into_values().collect(), unreadable)
}

fn parse_recurrence_status(value: Option<&str>) -> Result<Option<RecurrenceStatus>> {
    match value {
        None => Ok(None),
        Some("new") => Ok(Some(RecurrenceStatus::New)),
        Some("recurring") => Ok(Some(RecurrenceStatus::Recurring)),
        Some("regressed") => Ok(Some(RecurrenceStatus::Regressed)),
        Some("unknown") => Ok(Some(RecurrenceStatus::Unknown)),
        Some(other) => Err(ForgeError::invalid_argument(format!(
            "unknown recurrence status '{other}'; use new, recurring, regressed or unknown"
        ))),
    }
}

async fn report_history(ctx: &CliContext, args: &ReportHistoryArgs) -> Result<CommandSuccess> {
    if !args.fingerprint.starts_with("sha256:") {
        return Err(ForgeError::invalid_argument(
            "fingerprints use the form sha256:<hex>; run `report explain` to obtain one",
        ));
    }
    let (reports, unreadable) = load_reports_for_memory(ctx, false);
    let history = build_history(&reports, unreadable);
    let Some(entry) = find_fingerprint_in_project(
        &history,
        &args.fingerprint,
        &ctx.paths.project_root.to_string_lossy(),
    ) else {
        let candidates = history
            .fingerprints
            .iter()
            .take(5)
            .map(|entry| entry.fingerprint.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            format!(
                "fingerprint {} was not found in {} scanned local run(s)",
                args.fingerprint, history.scanned_runs
            ),
        )
        .with_context("available_fingerprints", serde_json::json!(candidates))
        .with_suggestion(
            "Run `androidforge report explain <run> --json` to read the stored fingerprint.",
        )
        .with_suggestion("Run `androidforge report recurrences --json` to list known failures."));
    };
    let mut entry = entry.clone();
    entry.occurrences.reverse();
    entry.occurrences.truncate(args.limit.max(1));
    let human = format!(
        "Fingerprint {}\nstatus: {}\ncategory: {}\noccurrences: {}\nfirst seen: {}\nlast seen: {}\ncurrently resolved: {}\n\n{}",
        entry.fingerprint,
        entry.status.as_str(),
        entry.category,
        entry.occurrence_count,
        entry.first_seen,
        entry.last_seen,
        entry.currently_resolved,
        entry
            .occurrences
            .iter()
            .map(|occurrence| format!(
                "  {}  {:?}  {}  {}\n",
                occurrence.started_at,
                occurrence.run_status,
                occurrence.run_id,
                occurrence.failed_step.as_deref().unwrap_or("-")
            ))
            .collect::<String>()
    );
    Ok(success(
        "report.history",
        serde_json::json!({
            "scanned_runs": history.scanned_runs,
            "unreadable_runs": history.unreadable_runs,
            "fingerprint": entry,
        }),
        human,
    ))
}

async fn report_recurrences(
    ctx: &CliContext,
    args: &ReportRecurrencesArgs,
) -> Result<CommandSuccess> {
    let status = parse_recurrence_status(args.status.as_deref())?;
    let since_rfc3339 = match &args.since {
        Some(value) => {
            let duration = parse_duration(value).map_err(ForgeError::invalid_argument)?;
            let since = chrono::Utc::now()
                - chrono::Duration::from_std(duration).map_err(|_| {
                    ForgeError::invalid_argument("--since is outside the supported range")
                })?;
            Some(since.to_rfc3339())
        }
        None => None,
    };
    let (reports, unreadable) = load_reports_for_memory(ctx, args.all_projects);
    let history = build_history(&reports, unreadable);
    let filtered = filter_history(
        &history,
        &RecurrenceFilter {
            project_root: if args.all_projects {
                args.project_root.clone()
            } else {
                Some(
                    args.project_root
                        .clone()
                        .unwrap_or_else(|| ctx.paths.project_root.to_string_lossy().into_owned()),
                )
            },
            package: args.package.clone(),
            fingerprint: None,
            device: args.device.clone(),
            app_version: args.app_version.clone(),
            since_rfc3339,
            status,
        },
    );
    let total = filtered.fingerprints.len();
    let counts =
        filtered
            .fingerprints
            .iter()
            .fold(BTreeMap::<&str, usize>::new(), |mut counts, entry| {
                *counts.entry(entry.status.as_str()).or_default() += 1;
                counts
            });
    let unresolved = filtered
        .fingerprints
        .iter()
        .filter(|entry| !entry.currently_resolved)
        .count();
    let mut entries = filtered.fingerprints;
    entries.truncate(args.limit.max(1));
    let human = format!(
        "Analyzed {} local run(s) ({} failed, {} unreadable).\n{} fingerprint(s): {} new, {} recurring, {} regressed, {} unknown; {} currently unresolved.\n{}",
        filtered.scanned_runs,
        filtered.failed_runs,
        filtered.unreadable_runs,
        total,
        counts.get("new").copied().unwrap_or(0),
        counts.get("recurring").copied().unwrap_or(0),
        counts.get("regressed").copied().unwrap_or(0),
        counts.get("unknown").copied().unwrap_or(0),
        unresolved,
        entries
            .iter()
            .map(|entry| format!(
                "  {}  {}  x{}  {}{}\n",
                entry.status.as_str(),
                entry.fingerprint,
                entry.occurrence_count,
                entry
                    .package
                    .as_deref()
                    .or(Some(entry.category.as_str()))
                    .unwrap_or("-"),
                if entry.currently_resolved {
                    "  (resolved)"
                } else {
                    ""
                }
            ))
            .collect::<String>()
    );
    Ok(success(
        "report.recurrences",
        serde_json::json!({
            "scanned_runs": filtered.scanned_runs,
            "failed_runs": filtered.failed_runs,
            "unreadable_runs": filtered.unreadable_runs,
            "count": total,
            "returned": entries.len(),
            "unresolved": unresolved,
            "counts": counts,
            "fingerprints": entries,
        }),
        human,
    ))
}

async fn prune(ctx: &CliContext, args: &crate::cli::ReportPruneArgs) -> Result<CommandSuccess> {
    if !ctx.yes && !ctx.dry_run {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            "pruning run artifacts is destructive; pass --yes",
        )
        .with_suggestion("Use --dry-run to preview which runs would be deleted."));
    }
    let runs = list_runs(&ctx.paths.artifacts_dir)?;
    let keep_last = args.keep_last.unwrap_or(0);
    let older_than = match &args.older_than {
        Some(value) => parse_duration(value).map_err(ForgeError::invalid_argument)?,
        None => Duration::from_secs(u64::from(ctx.config.file.project.retention_days) * 86_400),
    };
    let now = chrono::Utc::now();
    let mut removable = Vec::new();
    for (index, run) in runs.iter().enumerate() {
        if index < keep_last {
            continue;
        }
        if run.status == RunStatus::Running {
            continue;
        }
        let age = chrono::DateTime::parse_from_rfc3339(&run.started_at)
            .map(|started| now.signed_duration_since(started.with_timezone(&chrono::Utc)))
            .map(|duration| duration.to_std().unwrap_or_default())
            .unwrap_or_default();
        if age >= older_than {
            removable.push(run.clone());
        }
    }
    let mut deleted = Vec::new();
    for run in &removable {
        if !ctx.dry_run {
            let path = ctx.paths.artifacts_dir.join(&run.path);
            if path.is_dir() {
                std::fs::remove_dir_all(&path).map_err(|error| {
                    ForgeError::new(
                        ErrorCode::ArtifactWriteFailed,
                        format!("could not delete {}: {error}", path.display()),
                    )
                })?;
            }
        }
        deleted.push(run.run_id.clone());
    }
    let result = serde_json::json!({
        "dry_run": ctx.dry_run,
        "deleted_runs": deleted,
        "kept_runs": runs.len().saturating_sub(removable.len()),
        "older_than_seconds": older_than.as_secs(),
        "keep_last": keep_last,
    });
    Ok(success(
        "report.prune",
        result,
        format!(
            "{} {} run(s); {} kept.\n",
            if ctx.dry_run {
                "Would delete"
            } else {
                "Deleted"
            },
            deleted.len(),
            runs.len().saturating_sub(removable.len())
        ),
    ))
}

/// Resolve a run in the current artifacts directory, then fall back to
/// project-local `.artifacts/runs` directories. Conformance runs live under the
/// adapter's project root; agents often inspect them from the repository root
/// without repeating `--project`.
fn candidate_artifact_roots(ctx: &CliContext) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    roots.push(ctx.paths.artifacts_dir.clone());
    collect_artifact_roots(&ctx.paths.project_root, 0, &mut roots);
    collect_artifact_roots(&ctx.paths.cwd, 0, &mut roots);
    for start in [&ctx.paths.project_root, &ctx.paths.cwd] {
        let mut current = start.parent();
        for _ in 0..4 {
            let Some(parent) = current else {
                break;
            };
            collect_artifact_roots(parent, 0, &mut roots);
            current = parent.parent();
        }
    }
    roots.sort();
    roots.dedup();
    roots
}

fn same_project_scope(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        let normalized = value.replace('\\', "/");
        let trimmed = normalized.trim_end_matches('/');
        if cfg!(windows) {
            trimmed.to_ascii_lowercase()
        } else {
            trimmed.to_string()
        }
    };
    normalize(left) == normalize(right)
}

/// Filters applied when resolving `latest` / `latest-failure`, listing reports
/// and selecting a comparison baseline.
///
/// Every field is optional: an absent session id, serial, tag list or `since`
/// means the selection stays project-wide, which preserves the 1.0.0 default
/// behavior. `tags` uses AND semantics — a run must carry every requested tag.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct ReportScope {
    pub session_id: Option<String>,
    pub device_serial: Option<String>,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_rfc3339: Option<String>,
}

impl ReportScope {
    /// Build a scope from the globally selected session/serial plus explicit
    /// `--tag` and `--since` values supplied by the calling command.
    pub(crate) fn from_context(
        ctx: &CliContext,
        tags: Option<&[String]>,
        since: Option<&str>,
    ) -> Result<Self> {
        let device_serial = ctx
            .serial_arg
            .as_deref()
            .map(str::trim)
            .filter(|serial| !serial.is_empty())
            .map(str::to_string)
            .or_else(|| {
                std::env::var("ANDROIDFORGE_SERIAL")
                    .ok()
                    .map(|serial| serial.trim().to_string())
                    .filter(|serial| !serial.is_empty())
            });
        Ok(Self {
            session_id: ctx.session_id()?,
            device_serial,
            tags: tags
                .map(validate_report_tags)
                .transpose()?
                .unwrap_or_default(),
            since_rfc3339: parse_since_filter(since)?,
        })
    }

    fn matches(&self, report: &RunReport) -> bool {
        let started_after = match &self.since_rfc3339 {
            None => true,
            Some(since) => {
                let Some(started) = chrono::DateTime::parse_from_rfc3339(&report.started_at).ok()
                else {
                    return false;
                };
                let Some(since) = chrono::DateTime::parse_from_rfc3339(since).ok() else {
                    return false;
                };
                started >= since
            }
        };
        started_after
            && self
                .session_id
                .as_ref()
                .is_none_or(|session| report.session_id.as_deref() == Some(session.as_str()))
            && self
                .device_serial
                .as_ref()
                .is_none_or(|serial| report.device_serial.as_deref() == Some(serial.as_str()))
            && self
                .tags
                .iter()
                .all(|tag| report.tags.iter().any(|candidate| candidate == tag))
    }

    fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(session) = &self.session_id {
            parts.push(format!("session {session}"));
        }
        if let Some(serial) = &self.device_serial {
            parts.push(format!("device {serial}"));
        }
        if !self.tags.is_empty() {
            parts.push(format!("tags [{}]", self.tags.join(", ")));
        }
        if let Some(since) = &self.since_rfc3339 {
            parts.push(format!("since {since}"));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!(" (scope: {})", parts.join(", "))
        }
    }
}

/// Single entry point for report selection filters. All call sites (list,
/// explain, bundle and compare baseline selection) share this resolver.
pub(crate) fn resolve_report_scope(
    ctx: &CliContext,
    tags: Option<&[String]>,
    since: Option<&str>,
) -> Result<ReportScope> {
    ReportScope::from_context(ctx, tags, since)
}

/// Resolve `latest` and `latest-failure` consistently for reports and bundles.
/// Only runs whose recorded `project_root` matches the selected project are
/// eligible, so one app can never receive another app's evidence. A non-empty
/// [`ReportScope`] narrows the candidates further by session, device and tags.
pub(crate) fn resolve_latest_run_dir(
    ctx: &CliContext,
    failures_only: bool,
    scope: &ReportScope,
) -> Result<PathBuf> {
    let project_root = ctx.paths.project_root.to_string_lossy().into_owned();
    let mut runs: Vec<(androidforge_core::run::RunSummary, PathBuf)> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for root in candidate_artifact_roots(ctx) {
        let Ok(summaries) = list_runs(&root) else {
            continue;
        };
        for summary in summaries {
            if !summary.status.is_terminal() || !seen.insert(summary.run_id.clone()) {
                continue;
            }
            let path = root.join(&summary.path);
            let report_path = path.join("report.json");
            let Ok(report) = androidforge_core::util::read_json::<RunReport>(&report_path) else {
                continue;
            };
            if !same_project_scope(&report.project_root, &project_root) || !scope.matches(&report) {
                continue;
            }
            runs.push((summary, path));
        }
    }
    let detail = scope.describe();
    if runs.is_empty() {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            format!("no completed AndroidForge runs exist for project {project_root}{detail}"),
        )
        .with_suggestion("Run a command that creates a run report for this project first.")
        .with_suggestion("Pass --project <dir> to select another project."));
    }
    if let Some(selected) = select_latest_run(runs, failures_only) {
        return Ok(selected);
    }
    Err(ForgeError::new(
        ErrorCode::InvalidArgument,
        format!("no failed AndroidForge runs exist for project {project_root}{detail}"),
    )
    .with_suggestion("Use `latest` to inspect the newest completed run."))
}

fn select_latest_run(
    mut runs: Vec<(androidforge_core::run::RunSummary, PathBuf)>,
    failures_only: bool,
) -> Option<PathBuf> {
    runs.sort_by(|(left, _), (right, _)| {
        left.started_at
            .cmp(&right.started_at)
            .then_with(|| left.run_id.cmp(&right.run_id))
    });
    let selected = if failures_only {
        runs.iter()
            .rev()
            .find(|(summary, _)| summary.status != RunStatus::Passed)
    } else {
        runs.last()
    };
    selected.map(|(_, path)| path.clone())
}

fn resolve_report_dir(ctx: &CliContext, spec: &str) -> Result<PathBuf> {
    let original = match resolve_run_dir(&ctx.paths.artifacts_dir, spec) {
        Ok(dir) => return Ok(dir),
        Err(error) => error,
    };
    if Path::new(spec).is_dir() {
        return Err(original);
    }
    let roots = candidate_artifact_roots(ctx);
    let mut matches = Vec::new();
    for root in &roots {
        if let Ok(dir) = resolve_run_dir(root, spec)
            && !matches.contains(&dir)
        {
            matches.push(dir);
        }
    }
    match matches.len() {
        0 => Err(original.with_suggestion(
            "If the run belongs to a configured project, pass --project <dir> or run from that project.",
        )),
        1 => Ok(matches.remove(0)),
        _ if spec == "latest" => {
            matches.sort_by_key(|path| run_dir_name(path));
            Ok(matches.pop().expect("non-empty matches"))
        }
        _ => {
            let candidates = matches
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                format!("run '{spec}' exists in more than one project: {candidates}"),
            )
            .with_suggestion("Pass --project <dir> to choose the project."))
        }
    }
}

fn run_dir_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn collect_artifact_roots(root: &Path, depth: usize, roots: &mut Vec<PathBuf>) {
    if depth > 3 {
        return;
    }
    let artifacts = root.join(".artifacts");
    if artifacts.join("runs").is_dir() {
        roots.push(artifacts);
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.')
            || matches!(
                name.as_ref(),
                "target" | "node_modules" | "build" | "dist" | "vendor" | "crates" | "src"
            )
        {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_artifact_roots(&path, depth + 1, roots);
        }
    }
}

fn resolve_active_run(ctx: &CliContext) -> Result<(String, Option<String>, Option<String>)> {
    let session_id = ctx.session_id()?;
    if let Some(run) = &ctx.run_arg {
        return Ok((run.clone(), session_id.clone(), session_id));
    }
    if let Ok(run) = std::env::var("ANDROIDFORGE_RUN_ID")
        && !run.trim().is_empty()
    {
        return Ok((run, session_id.clone(), session_id));
    }
    let active = get_active_run(&ctx.paths, session_id.as_deref())?.ok_or_else(|| {
        ForgeError::new(ErrorCode::InvalidArgument, "no active run was found")
            .with_suggestion("Start one with `androidforge run start --name <name>`.")
            .with_suggestion("Or set ANDROIDFORGE_RUN_ID.")
    })?;
    Ok((active.run_id, session_id.clone(), session_id))
}

fn resolve_optional_run(ctx: &CliContext) -> Result<Option<String>> {
    if let Some(run) = &ctx.run_arg {
        return Ok(Some(run.clone()));
    }
    if let Ok(run) = std::env::var("ANDROIDFORGE_RUN_ID")
        && !run.trim().is_empty()
    {
        return Ok(Some(run));
    }
    let session_id = ctx.session_id()?;
    let Some(active) = get_active_run(&ctx.paths, session_id.as_deref())? else {
        return Ok(None);
    };
    // A crashed or externally finalized run can leave a pointer behind. Do not
    // make every later command fail against a finished run: clear the stale
    // pointer and let the command start an implicit run instead.
    let report_path = ctx.paths.run_dir(&active.run_id).join("report.json");
    if let Ok(report) = androidforge_core::util::read_json::<RunReport>(&report_path)
        && report.status != RunStatus::Running
    {
        clear_active_run(&ctx.paths, session_id.as_deref())?;
        return Ok(None);
    }
    Ok(Some(active.run_id))
}

fn report_human(report: &RunReport, dir: &std::path::Path) -> String {
    let mut human = format!(
        "Run {} ({:?})\ncommand: {}\nproject: {}\nstarted: {}\nduration: {}ms\nsession: {}\ndevice: {}\n\n",
        report.run_id,
        report.status,
        report.command,
        report.project,
        report.started_at,
        report.duration_ms,
        report.session_id.as_deref().unwrap_or("-"),
        report.device_serial.as_deref().unwrap_or("-"),
    );
    for step in &report.steps {
        human.push_str(&format!(
            "  [{:?}] {} ({})\n",
            step.status, step.name, step.duration_ms
        ));
        for artifact in &step.artifacts {
            human.push_str(&format!("      [{}] {}\n", artifact.kind, artifact.path));
        }
    }
    if let Some(failure) = &report.failure {
        human.push_str(&format!(
            "\nFailure [{}]: {}\n",
            failure.code.as_str(),
            failure.message
        ));
        for suggestion in &failure.suggestions {
            human.push_str(&format!("  - {suggestion}\n"));
        }
    }
    human.push_str(&format!(
        "\nReport: {}\n",
        dir.join("report.json").display()
    ));
    human
}

fn summary_json(report: &RunReport, dir: &std::path::Path) -> serde_json::Value {
    serde_json::json!({
        "run_id": report.run_id,
        "status": report.status,
        "command": report.command,
        "project": report.project,
        "started_at": report.started_at,
        "ended_at": report.ended_at,
        "duration_ms": report.duration_ms,
        "session_id": report.session_id,
        "device_serial": report.device_serial,
        "steps": report.steps.len(),
        "failed_steps": report.failed_steps(),
        "artifacts": report.artifacts.len(),
        "report_path": dir.join("report.json"),
        "manifest_path": dir.join("manifest.json"),
    })
}

async fn explain(ctx: &CliContext, args: &ReportExplainArgs) -> Result<CommandSuccess> {
    let scope = resolve_report_scope(ctx, Some(&args.tags), args.since.as_deref())?;
    let using_latest = matches!(args.run.as_str(), "latest" | "latest-failure");
    let dir = match args.run.as_str() {
        "latest" => resolve_latest_run_dir(ctx, false, &scope)?,
        "latest-failure" => resolve_latest_run_dir(ctx, true, &scope)?,
        _ => resolve_report_dir(ctx, &args.run)?,
    };
    let report: RunReport = androidforge_core::util::read_json(&dir.join("report.json"))?;
    let explanation = explain_report(&report);
    let recurrence = if report.status != RunStatus::Passed {
        let (reports, unreadable) = load_reports_for_memory(ctx, false);
        let history = build_history(&reports, unreadable);
        find_fingerprint_in_project(&history, &explanation.fingerprint, &report.project_root)
            .cloned()
    } else {
        None
    };
    let mut result = serde_json::to_value(&explanation)?;
    if args.run == "latest"
        && report.status == RunStatus::Passed
        && let Some(actions) = result
            .get_mut("next_actions")
            .and_then(serde_json::Value::as_array_mut)
    {
        actions.insert(
            0,
            serde_json::json!({
                "reason": "The newest completed run passed. Use latest-failure to inspect the newest failed run.",
                "priority": "high",
                "command": {
                    "argv": [
                        "androidforge",
                        "report",
                        "explain",
                        "latest-failure",
                        "--project",
                        ctx.paths.project_root.to_string_lossy(),
                        "--json"
                    ]
                },
                "destructive": false
            }),
        );
    }
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "run_dir".to_string(),
            serde_json::json!(dir.to_string_lossy()),
        );
        // Expose the filter that was actually applied. Explicit run ids bypass
        // latest-selection, so their reported scope is empty.
        let applied_scope = if using_latest {
            scope
        } else {
            ReportScope::default()
        };
        object.insert("scope".to_string(), serde_json::to_value(applied_scope)?);
        if let Some(recurrence) = &recurrence {
            object.insert("recurrence".to_string(), serde_json::to_value(recurrence)?);
        }
        if args.full {
            object.insert("report".to_string(), serde_json::to_value(&report)?);
            object.insert(
                "evidence_excerpts".to_string(),
                serde_json::json!(evidence_excerpts(&explanation, &dir)),
            );
        }
    }
    let markdown = explain_markdown(&explanation, &dir, args.full);
    let recurrence_line = recurrence
        .as_ref()
        .map(|recurrence| {
            format!(
                "recurrence: {} ({} occurrence(s), first {});\n",
                recurrence.status.as_str(),
                recurrence.occurrence_count,
                recurrence.first_seen
            )
        })
        .unwrap_or_default();
    let human = if args.markdown && !ctx.json {
        markdown.clone()
    } else {
        format!(
            "{} [{} / {} confidence]\nrun: {}\n{recurrence_line}next actions:\n{}\n",
            explanation.diagnosis.summary,
            explanation.diagnosis.category.as_str(),
            confidence_label(explanation.diagnosis.confidence),
            explanation.run_id,
            explanation
                .next_actions
                .iter()
                .map(|action| format!("  {}  ({})\n", action.command.argv.join(" "), action.reason))
                .collect::<String>()
        )
    };
    Ok(success("report.explain", result, human))
}

fn confidence_label(confidence: androidforge_core::diagnosis::Confidence) -> &'static str {
    use androidforge_core::diagnosis::Confidence;
    match confidence {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
        Confidence::InsufficientEvidence => "insufficient evidence",
    }
}

fn evidence_excerpts(explanation: &DiagnosticExplanation, dir: &Path) -> Vec<serde_json::Value> {
    let mut excerpts = Vec::new();
    for evidence in &explanation.evidence {
        let path = dir.join(&evidence.source);
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if metadata.len() > 2 * 1024 * 1024 {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(
            extension.as_str(),
            "txt" | "log" | "xml" | "json" | "md" | "csv"
        ) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut preview = text.chars().take(4000).collect::<String>();
        if text.chars().count() > 4000 {
            preview.push_str("\n[... excerpt truncated ...]");
        }
        excerpts.push(serde_json::json!({
            "path": evidence.source,
            "description": evidence.description,
            "preview": preview,
        }));
        if excerpts.len() >= 12 {
            break;
        }
    }
    excerpts
}

fn explain_markdown(explanation: &DiagnosticExplanation, dir: &Path, full: bool) -> String {
    let mut out = format!(
        "# Failure explanation: `{}`\n\n- Project: `{}`\n- Project root: `{}`\n- Scenario: `{}`\n- Status: **{}**\n- Category: **{}**\n- Confidence: **{}**\n- Fingerprint: `{}`\n\n## Summary\n\n{}\n",
        explanation.run_id,
        explanation.project,
        explanation.project_root,
        explanation.scenario,
        format!("{:?}", explanation.status).to_ascii_lowercase(),
        explanation.diagnosis.category.as_str(),
        confidence_label(explanation.diagnosis.confidence),
        explanation.fingerprint,
        explanation.diagnosis.summary
    );
    if let Some(code) = &explanation.diagnosis.code {
        out.push_str(&format!("\n- Error code: `{code}`"));
    }
    if let Some(operation) = &explanation.diagnosis.failed_operation {
        out.push_str(&format!("\n- Failed operation: `{operation}`"));
    }
    out.push('\n');
    if !explanation.alternatives.is_empty() {
        out.push_str("\n## Alternatives\n\n");
        for alternative in &explanation.alternatives {
            out.push_str(&format!(
                "- **{}** ({}) — {}\n",
                alternative.category.as_str(),
                confidence_label(alternative.confidence),
                alternative.summary
            ));
        }
    }
    if !explanation.uncertainty.is_empty() {
        out.push_str("\n## Uncertainty\n\n");
        for line in &explanation.uncertainty {
            out.push_str(&format!("- {line}\n"));
        }
    }
    out.push_str("\n## Evidence\n\n");
    if explanation.evidence.is_empty() {
        out.push_str("No artifact evidence was recorded.\n");
    } else {
        for evidence in &explanation.evidence {
            out.push_str(&format!(
                "- `{}` — {}\n",
                evidence.source, evidence.description
            ));
        }
    }
    out.push_str("\n## Next actions\n\n");
    if explanation.next_actions.is_empty() {
        out.push_str("No deterministic next action was available.\n");
    } else {
        for action in &explanation.next_actions {
            out.push_str(&format!(
                "### {}\n\n```powershell\n{}\n```\n",
                action.reason,
                action.command.argv.join(" ")
            ));
            if action.destructive {
                out.push_str("\n> **Destructive:** confirm this is intended before running it.\n");
            }
        }
    }
    out.push_str(&format!(
        "\n## Artifact directory\n\n```text\n{}\n```\n",
        dir.display()
    ));
    if full {
        out.push_str(
            "\n_Full explanation requested; use `--json --full` for expanded evidence._\n",
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse_all(report: &RunReport) {
        let explanation = explain_report(report);
        for action in &explanation.next_actions {
            crate::cli::Cli::try_parse_from(action.command.argv.clone()).unwrap_or_else(|error| {
                panic!(
                    "generated next action does not parse: {:?}\n{error}",
                    action.command.argv
                )
            });
        }
    }

    fn test_context(project: &std::path::Path) -> CliContext {
        let cli = crate::cli::Cli::try_parse_from([
            "androidforge",
            "--project",
            project.to_str().unwrap(),
            "report",
            "explain",
            "latest",
        ])
        .expect("parse test CLI");
        CliContext::new(&cli).expect("build test context")
    }

    fn write_scoped_report(
        project: &std::path::Path,
        run_id: &str,
        session: Option<&str>,
        serial: Option<&str>,
        tags: &[&str],
        status: RunStatus,
        started_at: &str,
    ) {
        let dir = project.join(".artifacts").join("runs").join(run_id);
        std::fs::create_dir_all(&dir).unwrap();
        let mut report = RunReport::new(run_id, "test.run", "Example", project.to_string_lossy());
        report.session_id = session.map(str::to_string);
        report.device_serial = serial.map(str::to_string);
        report.tags = tags.iter().map(|tag| tag.to_string()).collect();
        report.status = status;
        report.started_at = started_at.to_string();
        report.ended_at = Some(started_at.to_string());
        if status != RunStatus::Passed {
            report.failure = Some(androidforge_core::run::FailureInfo {
                code: ErrorCode::SelectorNotFound,
                message: "no UI element matched".to_string(),
                failed_operation: "ui.tap".to_string(),
                recoverable: true,
                suggestions: Vec::new(),
                context: BTreeMap::new(),
                artifacts: Vec::new(),
            });
        }
        std::fs::write(
            dir.join("report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }

    fn selected(path: PathBuf) -> String {
        path.file_name().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn latest_selection_honors_session_and_device_scope() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path();
        write_scoped_report(
            project,
            "ses-a-fail",
            Some("ses-a"),
            Some("dev-1"),
            &[],
            RunStatus::Failed,
            "2026-01-01T00:00:00Z",
        );
        write_scoped_report(
            project,
            "ses-a-pass",
            Some("ses-a"),
            Some("dev-1"),
            &[],
            RunStatus::Passed,
            "2026-01-02T00:00:00Z",
        );
        write_scoped_report(
            project,
            "ses-b-fail",
            Some("ses-b"),
            Some("dev-2"),
            &[],
            RunStatus::Failed,
            "2026-01-03T00:00:00Z",
        );
        write_scoped_report(
            project,
            "ses-b-pass",
            Some("ses-b"),
            Some("dev-2"),
            &[],
            RunStatus::Passed,
            "2026-01-04T00:00:00Z",
        );

        let mut ctx = test_context(project);
        let unfiltered = ReportScope::default();
        assert_eq!(
            selected(resolve_latest_run_dir(&ctx, false, &unfiltered).unwrap()),
            "ses-b-pass"
        );
        assert_eq!(
            selected(resolve_latest_run_dir(&ctx, true, &unfiltered).unwrap()),
            "ses-b-fail"
        );

        ctx.session_arg = Some("ses-a".to_string());
        let scope = resolve_report_scope(&ctx, None, None).unwrap();
        assert_eq!(scope.session_id.as_deref(), Some("ses-a"));
        assert_eq!(
            selected(resolve_latest_run_dir(&ctx, false, &scope).unwrap()),
            "ses-a-pass"
        );
        assert_eq!(
            selected(resolve_latest_run_dir(&ctx, true, &scope).unwrap()),
            "ses-a-fail"
        );

        ctx.session_arg = None;
        ctx.serial_arg = Some("dev-1".to_string());
        let scope = resolve_report_scope(&ctx, None, None).unwrap();
        assert_eq!(scope.device_serial.as_deref(), Some("dev-1"));
        assert_eq!(
            selected(resolve_latest_run_dir(&ctx, false, &scope).unwrap()),
            "ses-a-pass"
        );
        assert_eq!(
            selected(resolve_latest_run_dir(&ctx, true, &scope).unwrap()),
            "ses-a-fail"
        );

        ctx.session_arg = Some("ses-b".to_string());
        ctx.serial_arg = Some("dev-2".to_string());
        let scope = resolve_report_scope(&ctx, None, None).unwrap();
        assert_eq!(
            selected(resolve_latest_run_dir(&ctx, false, &scope).unwrap()),
            "ses-b-pass"
        );
        assert_eq!(
            selected(resolve_latest_run_dir(&ctx, true, &scope).unwrap()),
            "ses-b-fail"
        );

        // A combination that matches nothing must fail instead of falling back
        // to another session's or device's run.
        ctx.session_arg = Some("ses-a".to_string());
        ctx.serial_arg = Some("dev-2".to_string());
        let scope = resolve_report_scope(&ctx, None, None).unwrap();
        let error = resolve_latest_run_dir(&ctx, false, &scope).unwrap_err();
        assert!(
            error
                .message()
                .to_string()
                .contains("scope: session ses-a, device dev-2"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn latest_selection_honors_tag_scope() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path();
        write_scoped_report(
            project,
            "tagged-pass",
            Some("ses-a"),
            None,
            &["release", "smoke"],
            RunStatus::Passed,
            "2026-01-05T00:00:00Z",
        );
        write_scoped_report(
            project,
            "tagged-fail",
            Some("ses-a"),
            None,
            &["release"],
            RunStatus::Failed,
            "2026-01-06T00:00:00Z",
        );
        write_scoped_report(
            project,
            "untagged-pass",
            Some("ses-a"),
            None,
            &[],
            RunStatus::Passed,
            "2026-01-07T00:00:00Z",
        );

        let mut ctx = test_context(project);
        ctx.session_arg = Some("ses-a".to_string());

        let tags = vec!["smoke".to_string()];
        let scope = resolve_report_scope(&ctx, Some(&tags), None).unwrap();
        assert_eq!(scope.tags, tags);
        assert_eq!(
            selected(resolve_latest_run_dir(&ctx, false, &scope).unwrap()),
            "tagged-pass"
        );
        // The newest failure does not carry the smoke tag.
        assert!(resolve_latest_run_dir(&ctx, true, &scope).is_err());

        let tags = vec!["release".to_string()];
        let scope = resolve_report_scope(&ctx, Some(&tags), None).unwrap();
        assert_eq!(
            selected(resolve_latest_run_dir(&ctx, true, &scope).unwrap()),
            "tagged-fail"
        );

        // Multiple tags use AND semantics and must all be present.
        let tags = vec!["release".to_string(), "smoke".to_string()];
        let scope = resolve_report_scope(&ctx, Some(&tags), None).unwrap();
        assert_eq!(
            selected(resolve_latest_run_dir(&ctx, false, &scope).unwrap()),
            "tagged-pass"
        );
        let tags = vec!["smoke".to_string(), "missing".to_string()];
        let scope = resolve_report_scope(&ctx, Some(&tags), None).unwrap();
        assert!(resolve_latest_run_dir(&ctx, false, &scope).is_err());
    }

    #[test]
    fn diagnosis_next_actions_match_real_cli_syntax() {
        let cases = [
            (ErrorCode::SelectorNotFound, "no UI element matched"),
            (ErrorCode::SelectorAmbiguous, "multiple elements matched"),
            (ErrorCode::DeviceNotFound, "device missing"),
            (ErrorCode::ToolNotFound, "adb missing"),
            (ErrorCode::InstallFailed, "INSTALL_FAILED_VERSION_DOWNGRADE"),
            (
                ErrorCode::InstallSignatureMismatch,
                "signatures do not match",
            ),
            (ErrorCode::UiAssertionFailed, "assertion failed"),
            (ErrorCode::PermissionDenied, "permission denied"),
            (ErrorCode::CommandTimeout, "timed out"),
            (ErrorCode::CommandFailed, "ANR in com.example"),
        ];
        for (code, message) in cases {
            let mut report = RunReport::new("run-test", "conformance.run", "App", "C:\\App");
            report.status = RunStatus::Failed;
            report.build.apk_path = Some("C:\\builds\\app-debug.apk".to_string());
            report.failure = Some(androidforge_core::run::FailureInfo {
                code,
                message: message.to_string(),
                failed_operation: "ui.tap".to_string(),
                recoverable: true,
                suggestions: Vec::new(),
                context: [
                    ("package".to_string(), serde_json::json!("com.example.app")),
                    (
                        "selector".to_string(),
                        serde_json::json!({"text": "Save", "index": 1}),
                    ),
                    (
                        "permission".to_string(),
                        serde_json::json!("android.permission.CAMERA"),
                    ),
                ]
                .into_iter()
                .collect(),
                artifacts: Vec::new(),
            });
            parse_all(&report);
        }
    }
}
