use std::path::Path;
use std::time::Duration;

use androidforge_core::output::CommandSuccess;
use androidforge_core::run::{RunStatus, list_runs};
use androidforge_core::session::{SessionState, list_sessions};
use androidforge_core::util::parse_duration;
use serde::Serialize;

use crate::cli::{MaintenanceCleanArgs, MaintenanceCommand};
use crate::commands::common::success;
use crate::context::CliContext;

#[derive(Debug, Clone, Serialize)]
struct StorageSummary {
    schema_version: u32,
    state_dir: String,
    artifacts_dir: String,
    state_bytes: u64,
    artifacts_bytes: u64,
    total_bytes: u64,
    session_count: usize,
    active_session_count: usize,
    terminal_session_count: usize,
    stale_session_count: usize,
    session_bytes: u64,
    run_count: usize,
    running_run_count: usize,
    terminal_run_count: usize,
    run_bytes: u64,
    oldest_run_started_at: Option<String>,
    largest_runs: Vec<RunStorage>,
    terminal_sessions: Vec<SessionStorage>,
}

#[derive(Debug, Clone, Serialize)]
struct RunStorage {
    run_id: String,
    status: RunStatus,
    started_at: String,
    bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
struct SessionStorage {
    id: String,
    state: SessionState,
    serial: Option<String>,
    created_at: String,
    bytes: u64,
}

pub async fn maintenance(
    ctx: &CliContext,
    command: &MaintenanceCommand,
) -> androidforge_core::Result<CommandSuccess> {
    match command {
        MaintenanceCommand::Summary => summary(ctx),
        MaintenanceCommand::Clean(args) => clean(ctx, args),
    }
}

fn summary(ctx: &CliContext) -> androidforge_core::Result<CommandSuccess> {
    let state_dir = ctx.paths.state_dir.clone();
    let artifacts_dir = ctx.paths.artifacts_dir.clone();
    let sessions = list_sessions(&ctx.paths).unwrap_or_default();
    let runs = list_runs(&artifacts_dir).unwrap_or_default();
    let mut terminal_sessions = Vec::new();
    for session in &sessions {
        let dir = ctx.paths.sessions_dir().join(&session.id);
        let bytes = dir_size(&dir);
        if matches!(
            session.state,
            SessionState::Released | SessionState::Failed | SessionState::Abandoned
        ) {
            terminal_sessions.push(SessionStorage {
                id: session.id.clone(),
                state: session.state,
                serial: session.serial.clone(),
                created_at: session.created_at.clone(),
                bytes,
            });
        }
    }
    let mut run_storage = Vec::new();
    for run in &runs {
        let dir = artifacts_dir.join(&run.path);
        run_storage.push(RunStorage {
            run_id: run.run_id.clone(),
            status: run.status,
            started_at: run.started_at.clone(),
            bytes: dir_size(&dir),
        });
    }
    run_storage.sort_by_key(|left| std::cmp::Reverse(left.bytes));
    let state_bytes = dir_size(&state_dir);
    let artifacts_bytes = dir_size(&artifacts_dir);
    let active_sessions = sessions
        .iter()
        .filter(|session| {
            !matches!(
                session.state,
                SessionState::Released | SessionState::Failed | SessionState::Abandoned
            )
        })
        .count();
    let running_runs = runs
        .iter()
        .filter(|run| run.status == RunStatus::Running)
        .count();
    let summary = StorageSummary {
        schema_version: 1,
        state_dir: state_dir.to_string_lossy().into_owned(),
        artifacts_dir: artifacts_dir.to_string_lossy().into_owned(),
        state_bytes,
        artifacts_bytes,
        total_bytes: state_bytes.saturating_add(artifacts_bytes),
        session_count: sessions.len(),
        active_session_count: active_sessions,
        terminal_session_count: terminal_sessions.len(),
        stale_session_count: sessions.iter().filter(|session| session.stale).count(),
        session_bytes: terminal_sessions.iter().map(|session| session.bytes).sum(),
        run_count: runs.len(),
        running_run_count: running_runs,
        terminal_run_count: runs.len().saturating_sub(running_runs),
        run_bytes: run_storage.iter().map(|run| run.bytes).sum(),
        oldest_run_started_at: runs.last().map(|run| run.started_at.clone()),
        largest_runs: run_storage.into_iter().take(10).collect(),
        terminal_sessions,
    };
    let human = format!(
        "AndroidForge storage: {}\n  state: {} ({})\n  artifacts: {} ({})\n  sessions: {} ({} active, {} terminal, {} stale)\n  runs: {} ({} running, {} terminal)\n",
        format_bytes(summary.total_bytes),
        summary.state_dir,
        format_bytes(summary.state_bytes),
        summary.artifacts_dir,
        format_bytes(summary.artifacts_bytes),
        summary.session_count,
        summary.active_session_count,
        summary.terminal_session_count,
        summary.stale_session_count,
        summary.run_count,
        summary.running_run_count,
        summary.terminal_run_count,
    );
    Ok(success(
        "maintenance.summary",
        serde_json::to_value(&summary)?,
        human,
    ))
}

fn clean(
    ctx: &CliContext,
    args: &MaintenanceCleanArgs,
) -> androidforge_core::Result<CommandSuccess> {
    let older_than = args
        .older_than
        .as_deref()
        .map(parse_duration)
        .transpose()
        .map_err(androidforge_core::error::ForgeError::invalid_argument)?
        .unwrap_or(Duration::from_secs(30 * 86_400));
    let keep_last = args.keep_last.unwrap_or(10);
    if !ctx.yes && !ctx.dry_run {
        return Err(androidforge_core::error::ForgeError::invalid_argument(
            "maintenance clean is destructive; pass --yes",
        )
        .with_suggestion("Run with --dry-run first to preview exactly what would be removed."));
    }

    let session_report = if args.sessions {
        Some(androidforge_core::session::prune_sessions(
            &ctx.paths,
            older_than,
            keep_last,
            ctx.dry_run,
        )?)
    } else {
        None
    };
    let run_report = if args.runs {
        Some(prune_runs(ctx, older_than, keep_last, ctx.dry_run)?)
    } else {
        None
    };
    let result = serde_json::json!({
        "dry_run": ctx.dry_run,
        "older_than_seconds": older_than.as_secs(),
        "keep_last": keep_last,
        "sessions": session_report,
        "runs": run_report,
    });
    let human = format!(
        "{} maintenance: {} session(s), {} run(s) removed; {} kept.\n",
        if ctx.dry_run {
            "Would clean"
        } else {
            "Cleaned"
        },
        session_report
            .as_ref()
            .map(|report| report.pruned.len())
            .unwrap_or(0),
        run_report
            .as_ref()
            .map(|report| report.deleted_runs.len())
            .unwrap_or(0),
        session_report
            .as_ref()
            .map(|report| report.kept)
            .unwrap_or_default(),
    );
    Ok(success("maintenance.clean", result, human))
}

#[derive(Debug, Clone, Serialize)]
struct PrunedRun {
    run_id: String,
    bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
struct RunPruneReport {
    dry_run: bool,
    deleted_runs: Vec<String>,
    kept_runs: usize,
    removed_bytes: u64,
}

fn prune_runs(
    ctx: &CliContext,
    older_than: Duration,
    keep_last: usize,
    dry_run: bool,
) -> androidforge_core::Result<RunPruneReport> {
    let runs = list_runs(&ctx.paths.artifacts_dir)?;
    let now = chrono::Utc::now();
    let mut candidates: Vec<PrunedRun> = Vec::new();
    for (index, run) in runs.iter().enumerate() {
        if index < keep_last || run.status == RunStatus::Running {
            continue;
        }
        let age = chrono::DateTime::parse_from_rfc3339(&run.started_at)
            .map(|started| {
                now.signed_duration_since(started.with_timezone(&chrono::Utc))
                    .to_std()
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        if age >= older_than {
            candidates.push(PrunedRun {
                run_id: run.run_id.clone(),
                bytes: dir_size(&ctx.paths.artifacts_dir.join(&run.path)),
            });
        }
    }
    let mut deleted = Vec::new();
    let mut removed_bytes = 0;
    for run in &candidates {
        if !dry_run {
            let path = ctx.paths.artifacts_dir.join("runs").join(&run.run_id);
            if path.is_dir() {
                std::fs::remove_dir_all(&path).map_err(|error| {
                    androidforge_core::error::ForgeError::new(
                        androidforge_core::error::ErrorCode::ArtifactWriteFailed,
                        format!("could not delete {}: {error}", path.display()),
                    )
                })?;
            }
        }
        removed_bytes += run.bytes;
        deleted.push(run.run_id.clone());
    }
    Ok(RunPruneReport {
        dry_run,
        kept_runs: runs.len().saturating_sub(candidates.len()),
        deleted_runs: deleted,
        removed_bytes,
    })
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            total += dir_size(&path);
        } else if let Ok(metadata) = entry.metadata() {
            total += metadata.len();
        }
    }
    total
}

fn format_bytes(bytes: u64) -> String {
    androidforge_core::util::format_bytes(bytes)
}
