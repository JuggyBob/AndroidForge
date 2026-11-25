use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::paths::ForgePaths;
use androidforge_core::run::{ActiveRun, RunReport, RunStatus, RunSummary, list_runs};
use androidforge_core::session::{SessionState, list_sessions};
use androidforge_core::util::{parse_duration, read_json};
use chrono::{DateTime, Utc};
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

pub async fn maintenance(ctx: &CliContext, command: &MaintenanceCommand) -> Result<CommandSuccess> {
    match command {
        MaintenanceCommand::Summary => summary(ctx),
        MaintenanceCommand::Clean(args) => clean(ctx, args),
    }
}

fn summary(ctx: &CliContext) -> Result<CommandSuccess> {
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

/// Upper bound on the run directories removed by one confirmed execution.
/// Candidates are ordered oldest first, so a later call continues where the
/// previous one stopped instead of deleting an arbitrary slice.
pub const MAX_RETENTION_DELETIONS: usize = 100;

/// Typed, CLI-independent request for tag-aware run artifact retention.
///
/// `keep_tags` uses OR semantics: a run carrying *any* listed tag is
/// protected. Retention is conservative, so one protected label is enough to
/// preserve a run. `dry_run` defaults to `true`, and files are never removed
/// unless `yes` is explicitly set as well. `keep_last` preserves the newest
/// runs exactly as `maintenance clean --keep-last` already does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionRequest {
    pub keep_tags: Vec<String>,
    pub older_than: Duration,
    pub keep_last: usize,
    pub dry_run: bool,
    pub yes: bool,
}

impl RetentionRequest {
    pub fn new(
        keep_tags: impl IntoIterator<Item = impl Into<String>>,
        older_than: Duration,
    ) -> Self {
        Self {
            keep_tags: keep_tags.into_iter().map(Into::into).collect(),
            older_than,
            keep_last: 10,
            dry_run: true,
            yes: false,
        }
    }
}

impl Default for RetentionRequest {
    fn default() -> Self {
        Self::new(Vec::<String>::new(), Duration::from_secs(30 * 86_400))
    }
}

/// One artifact inside a run directory that would be removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetentionArtifact {
    /// Path relative to the run directory, with `/` separators.
    pub path: String,
    pub bytes: u64,
}

/// One run directory in the deletion plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetentionDeletion {
    pub run_id: String,
    /// Absolute path that would be removed.
    pub path: String,
    /// Artifact-store-relative path (`runs/<run_id>`).
    pub relative_path: String,
    /// Recursive size of the run directory at planning time.
    pub bytes: u64,
    /// Why this run is eligible for deletion.
    pub reason: String,
    /// Bounded listing of the artifacts inside the directory.
    pub artifacts: Vec<RetentionArtifact>,
}

/// A run the planner deliberately left alone, with the exact reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetentionKeep {
    pub run_id: String,
    pub reason: String,
}

/// Deterministic, serializable result of a retention plan or execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetentionPlan {
    pub schema_version: u32,
    pub dry_run: bool,
    pub older_than_seconds: u64,
    pub keep_last: usize,
    pub keep_tags: Vec<String>,
    /// Run directories to delete, oldest first, capped at
    /// [`MAX_RETENTION_DELETIONS`].
    pub deletions: Vec<RetentionDeletion>,
    pub kept_runs: Vec<RetentionKeep>,
    pub reclaimed_bytes: u64,
    /// Eligible runs not included because the per-call bound was reached.
    pub deferred: usize,
    /// Deletions exist but neither `yes` nor a dry run was requested.
    pub requires_confirmation: bool,
    /// True only after at least one directory was actually removed.
    pub executed: bool,
}

/// Build a deletion plan without touching any files.
///
/// Protection order is intentional and never deletes on uncertainty:
/// keep-latest window, unsafe path, unreadable report, running status, active
/// run pointer, protected keep tag, unparseable timestamp, then the age
/// threshold. Everything that is skipped is recorded in `kept_runs` with a
/// reason so a dry run is auditable.
pub fn plan_retention(paths: &ForgePaths, request: &RetentionRequest) -> Result<RetentionPlan> {
    let runs = list_runs(&paths.artifacts_dir)?;
    let active = active_run_ids(paths);
    let now = Utc::now();
    let mut candidates: Vec<(String, RunSummary, PathBuf)> = Vec::new();
    let mut kept = Vec::new();

    for (index, run) in runs.iter().enumerate() {
        if index < request.keep_last {
            kept.push(RetentionKeep {
                run_id: run.run_id.clone(),
                reason: format!("within keep_last {}", request.keep_last),
            });
            continue;
        }
        let Some(dir) = safe_run_dir(&paths.artifacts_dir, run) else {
            kept.push(RetentionKeep {
                run_id: run.run_id.clone(),
                reason: "unsafe run path".to_string(),
            });
            continue;
        };
        let Ok(report) = read_json::<RunReport>(&dir.join("report.json")) else {
            kept.push(RetentionKeep {
                run_id: run.run_id.clone(),
                reason: "unreadable report".to_string(),
            });
            continue;
        };
        if report.status == RunStatus::Running {
            kept.push(RetentionKeep {
                run_id: run.run_id.clone(),
                reason: "running run".to_string(),
            });
            continue;
        }
        if active.contains(&run.run_id) {
            kept.push(RetentionKeep {
                run_id: run.run_id.clone(),
                reason: "active run pointer".to_string(),
            });
            continue;
        }
        if let Some(tag) = matching_keep_tag(&report.tags, &request.keep_tags) {
            kept.push(RetentionKeep {
                run_id: run.run_id.clone(),
                reason: format!("protected tag: {tag}"),
            });
            continue;
        }
        let Some(age) = run_age(&run.started_at, &now) else {
            kept.push(RetentionKeep {
                run_id: run.run_id.clone(),
                reason: "unparseable started_at".to_string(),
            });
            continue;
        };
        if age < request.older_than {
            kept.push(RetentionKeep {
                run_id: run.run_id.clone(),
                reason: "newer than retention threshold".to_string(),
            });
            continue;
        }
        candidates.push((run.started_at.clone(), run.clone(), dir));
    }

    candidates.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.run_id.cmp(&right.1.run_id))
    });
    let deferred = candidates.len().saturating_sub(MAX_RETENTION_DELETIONS);
    candidates.truncate(MAX_RETENTION_DELETIONS);

    let mut deletions = Vec::new();
    let mut reclaimed_bytes = 0u64;
    for (_, run, dir) in candidates {
        let bytes = dir_size(&dir);
        reclaimed_bytes = reclaimed_bytes.saturating_add(bytes);
        deletions.push(RetentionDeletion {
            run_id: run.run_id.clone(),
            path: dir.to_string_lossy().into_owned(),
            relative_path: run.path.clone(),
            bytes,
            reason: format!(
                "age >= {}s and no protected keep tag",
                request.older_than.as_secs()
            ),
            artifacts: list_run_artifacts(&dir),
        });
    }

    let requires_confirmation = !request.dry_run && !request.yes && !deletions.is_empty();
    Ok(RetentionPlan {
        schema_version: 1,
        dry_run: request.dry_run,
        older_than_seconds: request.older_than.as_secs(),
        keep_last: request.keep_last,
        keep_tags: request.keep_tags.clone(),
        deletions,
        kept_runs: kept,
        reclaimed_bytes,
        deferred,
        requires_confirmation,
        executed: false,
    })
}

/// Apply a previously built plan. Deletes only when `yes` is set and `dry_run`
/// is not, revalidating each entry so a run that became active or protected
/// between planning and execution is never removed. A missing directory is
/// treated as already deleted, which makes repeated calls idempotent.
pub fn execute_retention(
    paths: &ForgePaths,
    request: &RetentionRequest,
    plan: RetentionPlan,
) -> Result<RetentionPlan> {
    if plan.deletions.is_empty() || request.dry_run || !request.yes {
        return Ok(plan);
    }
    let active = active_run_ids(paths);
    let protected_top = list_runs(&paths.artifacts_dir)?
        .into_iter()
        .take(request.keep_last)
        .map(|run| run.run_id)
        .collect::<BTreeSet<_>>();
    let now = Utc::now();
    let mut kept = plan.kept_runs;
    let mut executed = Vec::new();
    let mut reclaimed_bytes = 0u64;

    for deletion in plan.deletions {
        let Some(dir) = safe_run_dir_from_id(&paths.artifacts_dir, &deletion.run_id) else {
            kept.push(RetentionKeep {
                run_id: deletion.run_id,
                reason: "skipped at execution: unsafe run path".to_string(),
            });
            continue;
        };
        let Ok(report) = read_json::<RunReport>(&dir.join("report.json")) else {
            kept.push(RetentionKeep {
                run_id: deletion.run_id,
                reason: "skipped at execution: unreadable report".to_string(),
            });
            continue;
        };
        let skip = if report.status == RunStatus::Running {
            Some("running run")
        } else if active.contains(&deletion.run_id) {
            Some("active run pointer")
        } else if protected_top.contains(&deletion.run_id) {
            Some("within keep_last")
        } else if matching_keep_tag(&report.tags, &request.keep_tags).is_some() {
            Some("protected tag")
        } else if run_age(&report.started_at, &now).is_none_or(|age| age < request.older_than) {
            Some("no longer past the retention threshold")
        } else {
            None
        };
        if let Some(reason) = skip {
            kept.push(RetentionKeep {
                run_id: deletion.run_id,
                reason: format!("skipped at execution: {reason}"),
            });
            continue;
        }
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {
                reclaimed_bytes = reclaimed_bytes.saturating_add(deletion.bytes);
                executed.push(deletion);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ForgeError::new(
                    ErrorCode::ArtifactWriteFailed,
                    format!("could not delete {}: {error}", dir.display()),
                ));
            }
        }
    }

    Ok(RetentionPlan {
        schema_version: plan.schema_version,
        dry_run: plan.dry_run,
        older_than_seconds: plan.older_than_seconds,
        keep_last: plan.keep_last,
        keep_tags: plan.keep_tags,
        deletions: executed,
        kept_runs: kept,
        reclaimed_bytes,
        deferred: plan.deferred,
        requires_confirmation: false,
        executed: true,
    })
}

/// Plan and, if confirmed, execute retention in one call.
pub fn apply_retention(paths: &ForgePaths, request: &RetentionRequest) -> Result<RetentionPlan> {
    execute_retention(paths, request, plan_retention(paths, request)?)
}

fn matching_keep_tag<'a>(tags: &[String], keep_tags: &'a [String]) -> Option<&'a String> {
    keep_tags
        .iter()
        .find(|keep| tags.iter().any(|tag| tag == *keep))
}

fn active_run_ids(paths: &ForgePaths) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(paths.active_runs_dir()) else {
        return ids;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Ok(active) = read_json::<ActiveRun>(&path) {
            ids.insert(active.run_id);
        }
    }
    ids
}

fn run_age(started_at: &str, now: &DateTime<Utc>) -> Option<Duration> {
    let started = DateTime::parse_from_rfc3339(started_at).ok()?;
    now.signed_duration_since(started.with_timezone(&Utc))
        .to_std()
        .ok()
}

/// Resolve a run directory, refusing absolute paths, parent traversal and
/// symlinks that escape the artifact store's `runs` directory.
fn safe_run_dir_from_id(artifacts_root: &Path, run_id: &str) -> Option<PathBuf> {
    let id = Path::new(run_id);
    let mut components = id.components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return None;
    }
    let runs_dir = artifacts_root.join("runs");
    let dir = runs_dir.join(id);
    let root = std::fs::canonicalize(&runs_dir).ok()?;
    let resolved = std::fs::canonicalize(&dir).ok()?;
    if resolved == root || !resolved.starts_with(&root) {
        return None;
    }
    Some(dir)
}

/// A run's recorded path must be the canonical `runs/<run_id>` shape and agree
/// with its report id before it may be considered for deletion.
fn safe_run_dir(artifacts_root: &Path, run: &RunSummary) -> Option<PathBuf> {
    let relative = Path::new(&run.path);
    let mut components = relative.components();
    if !matches!(components.next(), Some(Component::Normal(name)) if name == "runs") {
        return None;
    }
    let tail: Vec<_> = components.collect();
    if tail.len() != 1
        || !tail
            .iter()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return None;
    }
    if tail[0].as_os_str() != Path::new(&run.run_id).as_os_str() {
        return None;
    }
    safe_run_dir_from_id(artifacts_root, &run.run_id)
}

const MAX_LISTED_ARTIFACTS: usize = 64;

fn list_run_artifacts(dir: &Path) -> Vec<RetentionArtifact> {
    let mut artifacts = Vec::new();
    collect_artifacts(dir, dir, &mut artifacts);
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    artifacts
}

fn collect_artifacts(base: &Path, dir: &Path, out: &mut Vec<RetentionArtifact>) {
    if out.len() >= MAX_LISTED_ARTIFACTS {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_LISTED_ARTIFACTS {
            break;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_artifacts(base, &path, out);
        } else if let Ok(metadata) = entry.metadata() {
            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(RetentionArtifact {
                path: relative,
                bytes: metadata.len(),
            });
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct RunPruneReport {
    dry_run: bool,
    deleted_runs: Vec<String>,
    kept_runs: usize,
    removed_bytes: u64,
}

fn clean(ctx: &CliContext, args: &MaintenanceCleanArgs) -> Result<CommandSuccess> {
    let older_than = args
        .older_than
        .as_deref()
        .map(parse_duration)
        .transpose()
        .map_err(ForgeError::invalid_argument)?
        .unwrap_or(Duration::from_secs(30 * 86_400));
    let keep_last = args.keep_last.unwrap_or(10);
    // `--keep-tag` is a conservative preview trigger: asking to protect tagged
    // runs without an explicit confirmation must not delete anything, even if
    // the caller forgot `--dry-run`.
    let dry_run = ctx.dry_run || (!ctx.yes && !args.keep_tag.is_empty());
    if !ctx.yes && !dry_run {
        return Err(
            ForgeError::invalid_argument("maintenance clean is destructive; pass --yes")
                .with_suggestion(
                    "Run with --dry-run first to preview exactly what would be removed.",
                ),
        );
    }

    let session_report = if args.sessions {
        Some(androidforge_core::session::prune_sessions(
            &ctx.paths, older_than, keep_last, dry_run,
        )?)
    } else {
        None
    };
    let run_report = if args.runs {
        let request = RetentionRequest {
            keep_tags: args.keep_tag.clone(),
            older_than,
            keep_last,
            dry_run,
            yes: ctx.yes,
        };
        let plan = apply_retention(&ctx.paths, &request)?;
        let legacy = RunPruneReport {
            dry_run: plan.dry_run,
            deleted_runs: plan
                .deletions
                .iter()
                .map(|deletion| deletion.run_id.clone())
                .collect(),
            kept_runs: plan.kept_runs.len(),
            removed_bytes: plan.reclaimed_bytes,
        };
        Some((legacy, plan))
    } else {
        None
    };
    let result = serde_json::json!({
        "dry_run": dry_run,
        "older_than_seconds": older_than.as_secs(),
        "keep_last": keep_last,
        "sessions": session_report,
        "runs": run_report.as_ref().map(|(report, _)| report),
        // Additive, versioned retention plan: the legacy `runs` summary remains
        // unchanged for existing consumers while callers can audit exactly
        // which runs were protected, deferred, or removed.
        "retention": run_report.as_ref().map(|(_, plan)| plan),
    });
    let human = format!(
        "{} maintenance: {} session(s), {} run(s) removed; {} kept.\n",
        if dry_run { "Would clean" } else { "Cleaned" },
        session_report
            .as_ref()
            .map(|report| report.pruned.len())
            .unwrap_or(0),
        run_report
            .as_ref()
            .map(|(report, _)| report.deleted_runs.len())
            .unwrap_or(0),
        session_report
            .as_ref()
            .map(|report| report.kept)
            .unwrap_or_default(),
    );
    Ok(success("maintenance.clean", result, human))
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

#[cfg(test)]
mod tests {
    use super::*;
    use androidforge_core::run::set_active_run;

    struct Fixture {
        temp: tempfile::TempDir,
        paths: ForgePaths,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let project = temp.path().join("project");
            std::fs::create_dir_all(&project).unwrap();
            let mut paths = ForgePaths::discover(Some(project.clone()), None).unwrap();
            paths.state_dir = temp.path().join("state");
            paths.artifacts_dir = temp.path().join("artifacts");
            std::fs::create_dir_all(paths.artifacts_dir.join("runs")).unwrap();
            std::fs::create_dir_all(paths.active_runs_dir()).unwrap();
            Self { temp, paths }
        }

        fn write_run(
            &self,
            run_id: &str,
            status: RunStatus,
            started_at: &str,
            tags: &[&str],
        ) -> PathBuf {
            let dir = self.paths.run_dir(run_id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("payload.bin"), vec![b'x'; 16]).unwrap();
            let report = serde_json::json!({
                "schema_version": 1,
                "run_id": run_id,
                "command": "test.command",
                "project": "Example",
                "project_root": self.paths.project_root.to_string_lossy(),
                "status": status.as_str(),
                "exit_code": if status == RunStatus::Running { 0 } else { 1 },
                "started_at": started_at,
                "duration_ms": 0,
                "tags": tags,
            });
            std::fs::write(
                dir.join("report.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            dir
        }

        fn request(&self, keep_tags: &[&str], older_than: Duration) -> RetentionRequest {
            let mut request = RetentionRequest::new(keep_tags.iter().copied(), older_than);
            request.keep_last = 0;
            request
        }
    }

    fn old_timestamp() -> String {
        (Utc::now() - chrono::Duration::hours(2)).to_rfc3339()
    }

    fn deleted_ids(plan: &RetentionPlan) -> Vec<&str> {
        plan.deletions
            .iter()
            .map(|deletion| deletion.run_id.as_str())
            .collect()
    }

    #[test]
    fn retention_plan_protects_runs_with_a_keep_tag() {
        let fixture = Fixture::new();
        let aged = old_timestamp();
        fixture.write_run(
            "run-tagged",
            RunStatus::Passed,
            &aged,
            &["release", "smoke"],
        );
        fixture.write_run("run-other-tag", RunStatus::Failed, &aged, &["smoke"]);
        fixture.write_run("run-untagged", RunStatus::Passed, &aged, &[]);

        let request = fixture.request(&["release"], Duration::from_secs(60));
        let plan = plan_retention(&fixture.paths, &request).unwrap();

        assert_eq!(
            deleted_ids(&plan),
            vec!["run-other-tag", "run-untagged"],
            "keep tags protect only runs that carry them"
        );
        assert!(plan.kept_runs.iter().any(|keep| {
            keep.run_id == "run-tagged" && keep.reason == "protected tag: release"
        }));
        assert!(fixture.paths.run_dir("run-tagged").is_dir());
        for deletion in &plan.deletions {
            assert_eq!(
                deletion.bytes,
                dir_size(&fixture.paths.run_dir(&deletion.run_id)),
                "planned bytes must match the run directory"
            );
            assert_eq!(deletion.relative_path, format!("runs/{}", deletion.run_id));
        }
    }

    #[test]
    fn retention_respects_the_age_boundary() {
        let fixture = Fixture::new();
        let boundary = (Utc::now() - chrono::Duration::seconds(10)).to_rfc3339();
        let recent = (Utc::now() - chrono::Duration::seconds(9)).to_rfc3339();
        fixture.write_run("run-boundary", RunStatus::Passed, &boundary, &[]);
        fixture.write_run("run-recent", RunStatus::Passed, &recent, &[]);

        let request = fixture.request(&[], Duration::from_secs(10));
        let plan = plan_retention(&fixture.paths, &request).unwrap();

        assert_eq!(deleted_ids(&plan), vec!["run-boundary"]);
        assert!(plan.kept_runs.iter().any(|keep| {
            keep.run_id == "run-recent" && keep.reason == "newer than retention threshold"
        }));
    }

    #[test]
    fn retention_defaults_to_dry_run_and_requires_explicit_yes() {
        assert!(RetentionRequest::default().dry_run);
        assert!(!RetentionRequest::default().yes);

        let fixture = Fixture::new();
        let aged = old_timestamp();
        let dir = fixture.write_run("run-old", RunStatus::Passed, &aged, &[]);

        let mut request = fixture.request(&[], Duration::from_secs(60));
        let dry = apply_retention(&fixture.paths, &request).unwrap();
        assert_eq!(deleted_ids(&dry), vec!["run-old"]);
        assert!(dir.is_dir(), "dry run must never delete");
        assert!(!dry.executed);
        assert!(!dry.requires_confirmation, "dry run is already explicit");

        request.dry_run = false;
        let unconfirmed = apply_retention(&fixture.paths, &request).unwrap();
        assert_eq!(deleted_ids(&unconfirmed), vec!["run-old"]);
        assert!(dir.is_dir(), "missing --yes must never delete");
        assert!(unconfirmed.requires_confirmation);
        assert!(!unconfirmed.executed);

        request.yes = true;
        let confirmed = apply_retention(&fixture.paths, &request).unwrap();
        assert_eq!(deleted_ids(&confirmed), vec!["run-old"]);
        assert!(!dir.exists());
        assert!(confirmed.executed);
        assert!(confirmed.reclaimed_bytes > 0);
    }

    #[test]
    fn retention_never_deletes_running_or_active_runs() {
        let fixture = Fixture::new();
        let aged = old_timestamp();
        let running = fixture.write_run("run-running", RunStatus::Running, &aged, &[]);
        let active = fixture.write_run("run-active", RunStatus::Passed, &aged, &[]);
        let terminal = fixture.write_run("run-terminal", RunStatus::Failed, &aged, &[]);
        set_active_run(
            &fixture.paths,
            Some("ses-test"),
            "run-active",
            &fixture.paths.project_root,
        )
        .unwrap();

        let mut request = fixture.request(&[], Duration::from_secs(60));
        let plan = plan_retention(&fixture.paths, &request).unwrap();
        assert_eq!(deleted_ids(&plan), vec!["run-terminal"]);
        assert!(
            plan.kept_runs
                .iter()
                .any(|keep| keep.run_id == "run-running" && keep.reason == "running run")
        );
        assert!(
            plan.kept_runs
                .iter()
                .any(|keep| keep.run_id == "run-active" && keep.reason == "active run pointer")
        );

        request.yes = true;
        request.dry_run = false;
        apply_retention(&fixture.paths, &request).unwrap();
        assert!(running.is_dir());
        assert!(active.is_dir());
        assert!(!terminal.exists());
    }

    #[test]
    fn retention_is_bounded_and_idempotent() {
        let fixture = Fixture::new();
        let aged = old_timestamp();
        let total = MAX_RETENTION_DELETIONS + 2;
        for index in 0..total {
            fixture.write_run(&format!("run-{index:03}"), RunStatus::Passed, &aged, &[]);
        }

        let mut request = fixture.request(&[], Duration::from_secs(60));
        request.dry_run = false;
        request.yes = true;
        let first = apply_retention(&fixture.paths, &request).unwrap();
        assert_eq!(first.deletions.len(), MAX_RETENTION_DELETIONS);
        assert_eq!(first.deferred, 2);
        assert_eq!(
            first.deletions.first().unwrap().run_id,
            "run-000",
            "oldest runs go first"
        );

        let second = apply_retention(&fixture.paths, &request).unwrap();
        assert_eq!(second.deletions.len(), 2);
        assert_eq!(second.deferred, 0);

        let third = apply_retention(&fixture.paths, &request).unwrap();
        assert!(
            third.deletions.is_empty(),
            "repeat execution removes nothing"
        );
    }

    #[test]
    fn retention_refuses_unsafe_paths() {
        let fixture = Fixture::new();
        let aged = old_timestamp();
        fixture.write_run("run-safe", RunStatus::Passed, &aged, &[]);

        let summary = |run_id: &str, path: &str| RunSummary {
            run_id: run_id.to_string(),
            command: "test.command".to_string(),
            project: "Example".to_string(),
            status: RunStatus::Passed,
            exit_code: 0,
            started_at: aged.clone(),
            duration_ms: 0,
            steps: 0,
            failed_steps: 0,
            path: path.to_string(),
        };

        for (run_id, path) in [
            ("outside", "../outside"),
            ("absolute", "/tmp/outside"),
            ("traversal", "runs/../../outside"),
            ("nested", "runs/nested/run"),
            ("mismatch", "runs/other-id"),
        ] {
            assert!(
                safe_run_dir(&fixture.paths.artifacts_dir, &summary(run_id, path)).is_none(),
                "unsafe path must be refused: {path}"
            );
        }
        assert!(
            safe_run_dir_from_id(&fixture.paths.artifacts_dir, "../escape").is_none(),
            "run ids must be a single normal path component"
        );

        let request = fixture.request(&[], Duration::from_secs(60));
        let plan = plan_retention(&fixture.paths, &request).unwrap();
        assert_eq!(deleted_ids(&plan), vec!["run-safe"]);
    }

    #[test]
    fn retention_plan_is_serializable_with_paths_bytes_and_reasons() {
        let fixture = Fixture::new();
        fixture.write_run("run-old", RunStatus::Passed, &old_timestamp(), &[]);
        let request = fixture.request(&[], Duration::from_secs(60));
        let plan = plan_retention(&fixture.paths, &request).unwrap();
        let value = serde_json::to_value(&plan).unwrap();
        let deletion = &value["deletions"][0];
        assert!(deletion["path"].as_str().unwrap().ends_with("run-old"));
        assert_eq!(deletion["relative_path"].as_str(), Some("runs/run-old"));
        assert!(deletion["bytes"].as_u64().unwrap() > 0);
        assert!(deletion["reason"].as_str().unwrap().contains("age >="));
        assert!(
            deletion["artifacts"][0]["path"]
                .as_str()
                .unwrap()
                .ends_with("payload.bin")
        );
    }

    #[test]
    fn retention_keeps_the_newest_runs_from_the_existing_clean_policy() {
        let fixture = Fixture::new();
        let aged = old_timestamp();
        for index in 0..3 {
            fixture.write_run(&format!("run-{index}"), RunStatus::Passed, &aged, &[]);
        }
        let mut request = fixture.request(&[], Duration::from_secs(60));
        request.keep_last = 2;
        let plan = plan_retention(&fixture.paths, &request).unwrap();
        assert_eq!(plan.deletions.len(), 1);
        assert_eq!(plan.kept_runs.len(), 2);
        assert!(
            plan.kept_runs
                .iter()
                .all(|keep| keep.reason.starts_with("within keep_last"))
        );
        assert!(fixture.temp.path().join("artifacts").is_dir());
    }
}
