use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::artifact;
use crate::error::{ErrorBody, ErrorCode, ForgeError, Result};
use crate::model::{ArtifactRef, BuildIdentity, Device};
use crate::paths::ForgePaths;
use crate::redact::Redactor;
use crate::util::{
    ensure_dir, new_run_token, now_rfc3339, read_json, relative_to, slugify, write_atomic,
    write_json_atomic,
};

pub const REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Passed,
    Failed,
    Partial,
    Cancelled,
    Interrupted,
    TimedOut,
    Abandoned,
}

impl RunStatus {
    pub fn is_success(self) -> bool {
        matches!(self, Self::Passed)
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Running,
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureInfo {
    pub code: ErrorCode,
    pub message: String,
    pub failed_operation: String,
    pub recoverable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
}

impl FailureInfo {
    pub fn from_error(error: &ForgeError, operation: &str) -> Self {
        let body = error.body();
        Self {
            code: body.code,
            message: body.message.clone(),
            failed_operation: operation.to_string(),
            recoverable: body.recoverable,
            suggestions: body.suggestions.clone(),
            context: body.context.clone(),
            artifacts: body.artifacts.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStep {
    pub id: String,
    pub name: String,
    pub command: String,
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<FailureInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub details: Value,
    #[serde(default)]
    pub retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunInvocation {
    pub argv: Vec<String>,
    pub cwd: String,
    pub project_root: String,
    pub captured_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub schema_version: u32,
    pub run_id: String,
    pub command: String,
    pub project: String,
    pub project_root: String,
    /// Exact CLI argv that started the run. Older reports do not contain it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation: Option<RunInvocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<Device>,
    #[serde(default)]
    pub build: BuildIdentity,
    pub status: RunStatus,
    pub exit_code: i32,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    pub duration_ms: u64,
    #[serde(default)]
    pub steps: Vec<RunStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<FailureInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recovery_suggestions: Vec<String>,
}

impl RunReport {
    pub fn new(
        run_id: impl Into<String>,
        command: impl Into<String>,
        project: impl Into<String>,
        project_root: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: REPORT_SCHEMA_VERSION,
            run_id: run_id.into(),
            command: command.into(),
            project: project.into(),
            project_root: project_root.into(),
            invocation: None,
            session_id: None,
            device_serial: None,
            device: None,
            build: BuildIdentity::default(),
            status: RunStatus::Running,
            exit_code: 0,
            started_at: now_rfc3339(),
            ended_at: None,
            duration_ms: 0,
            steps: Vec::new(),
            failure: None,
            artifacts: Vec::new(),
            recovery_suggestions: Vec::new(),
        }
    }

    pub fn failed_steps(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| step.status == StepStatus::Failed)
            .count()
    }

    pub fn is_pass(&self) -> bool {
        self.status == RunStatus::Passed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub name: String,
    pub version: String,
    pub host_os: String,
    pub host_arch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMetadata {
    pub schema_version: u32,
    pub run_id: String,
    pub tool: ToolMetadata,
    pub command: String,
    pub project: String,
    pub project_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_config: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<Device>,
    #[serde(default)]
    pub build: BuildIdentity,
    /// PID of the AndroidForge process that created the run. Optional because
    /// reports written before reconciliation support did not record it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_host: Option<String>,
    #[serde(default)]
    pub tool_versions: BTreeMap<String, String>,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub artifacts_root: PathBuf,
    pub project_root: PathBuf,
    pub project_name: String,
    pub project_config: Option<PathBuf>,
    pub name: Option<String>,
    pub command: String,
    pub session_id: Option<String>,
    pub device: Option<Device>,
    pub build: BuildIdentity,
    pub tool_versions: BTreeMap<String, String>,
}

impl RunOptions {
    pub fn new(
        artifacts_root: impl Into<PathBuf>,
        project_root: impl Into<PathBuf>,
        project_name: impl Into<String>,
        command: impl Into<String>,
    ) -> Self {
        Self {
            artifacts_root: artifacts_root.into(),
            project_root: project_root.into(),
            project_name: project_name.into(),
            project_config: None,
            name: None,
            command: command.into(),
            session_id: None,
            device: None,
            build: BuildIdentity::default(),
            tool_versions: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
pub struct RunContext {
    pub run_id: String,
    pub dir: PathBuf,
    pub report: RunReport,
    pub metadata: RunMetadata,
    redactor: Redactor,
}

impl RunContext {
    pub fn create(options: RunOptions, redactor: Redactor) -> Result<Self> {
        let name = options
            .name
            .clone()
            .unwrap_or_else(|| options.command.clone());
        let run_id = format!("{}_{}", new_run_token(), slugify(&name, 40));
        let dir = options.artifacts_root.join("runs").join(&run_id);
        ensure_dir(&dir)?;

        let mut report = RunReport::new(
            &run_id,
            &options.command,
            &options.project_name,
            options.project_root.to_string_lossy(),
        );
        report.session_id = options.session_id.clone();
        report.device_serial = options.device.as_ref().map(|device| device.serial.clone());
        report.device = options.device.clone();
        report.build = options.build.clone();
        report.invocation = Some(RunInvocation {
            argv: std::env::args_os()
                .map(|value| value.to_string_lossy().into_owned())
                .collect(),
            cwd: std::env::current_dir()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            project_root: options.project_root.to_string_lossy().into_owned(),
            captured_at: now_rfc3339(),
        });

        let metadata = RunMetadata {
            schema_version: REPORT_SCHEMA_VERSION,
            run_id: run_id.clone(),
            tool: ToolMetadata {
                name: "androidforge".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                host_os: std::env::consts::OS.to_string(),
                host_arch: std::env::consts::ARCH.to_string(),
            },
            command: options.command,
            project: options.project_name,
            project_root: options.project_root.to_string_lossy().into_owned(),
            project_config: options
                .project_config
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            session_id: options.session_id,
            device: options.device,
            build: options.build,
            creator_pid: Some(std::process::id()),
            creator_host: std::env::var("COMPUTERNAME").ok(),
            tool_versions: options.tool_versions,
            started_at: report.started_at.clone(),
            notes: Vec::new(),
        };

        let mut context = Self {
            run_id,
            dir,
            report,
            metadata,
            redactor,
        };
        context.save()?;
        Ok(context)
    }

    pub fn open(artifacts_root: &Path, run_id: &str, redactor: Redactor) -> Result<Self> {
        let dir = artifacts_root.join("runs").join(run_id);
        Self::open_dir(dir, redactor)
    }

    pub fn open_dir(dir: PathBuf, redactor: Redactor) -> Result<Self> {
        let report_path = dir.join("report.json");
        if !report_path.is_file() {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                format!("run was not found at {}", dir.display()),
            )
            .with_suggestion(
                "Use `androidforge report path <run>` or `androidforge report show latest`.",
            ));
        }
        let report: RunReport = read_json(&report_path)?;
        if report.schema_version != REPORT_SCHEMA_VERSION {
            return Err(ForgeError::new(
                ErrorCode::Internal,
                format!(
                    "run {} uses report schema {}; this build supports {}",
                    report.run_id, report.schema_version, REPORT_SCHEMA_VERSION
                ),
            ));
        }
        let metadata_path = dir.join("metadata.json");
        let metadata: RunMetadata = if metadata_path.is_file() {
            read_json(&metadata_path)?
        } else {
            RunMetadata {
                schema_version: REPORT_SCHEMA_VERSION,
                run_id: report.run_id.clone(),
                tool: ToolMetadata {
                    name: "androidforge".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    host_os: std::env::consts::OS.to_string(),
                    host_arch: std::env::consts::ARCH.to_string(),
                },
                command: report.command.clone(),
                project: report.project.clone(),
                project_root: report.project_root.clone(),
                project_config: None,
                session_id: report.session_id.clone(),
                device: report.device.clone(),
                build: report.build.clone(),
                creator_pid: None,
                creator_host: None,
                tool_versions: BTreeMap::new(),
                started_at: report.started_at.clone(),
                notes: Vec::new(),
            }
        };
        Ok(Self {
            run_id: report.run_id.clone(),
            dir,
            report,
            metadata,
            redactor,
        })
    }

    pub fn report_path(&self) -> PathBuf {
        self.dir.join("report.json")
    }

    pub fn metadata_path(&self) -> PathBuf {
        self.dir.join("metadata.json")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.dir.join("manifest.json")
    }

    pub fn set_device(&mut self, device: Device) {
        self.report.device_serial = Some(device.serial.clone());
        self.report.device = Some(device.clone());
        self.metadata.device = Some(device);
    }

    pub fn set_build(&mut self, build: BuildIdentity) {
        self.report.build = build.clone();
        self.metadata.build = build;
    }

    pub fn add_note(&mut self, note: impl Into<String>) {
        let note = note.into();
        if !self.metadata.notes.contains(&note) {
            self.metadata.notes.push(note);
        }
    }

    pub fn start_step(
        &mut self,
        name: impl Into<String>,
        command: impl Into<String>,
    ) -> Result<usize> {
        let index = self.report.steps.len();
        let step = RunStep {
            id: format!("step-{:03}", index + 1),
            name: name.into(),
            command: command.into(),
            status: StepStatus::Running,
            exit_code: None,
            started_at: now_rfc3339(),
            ended_at: None,
            duration_ms: 0,
            failure: None,
            artifacts: Vec::new(),
            details: Value::Object(Default::default()),
            retries: 0,
        };
        self.report.steps.push(step);
        self.save()?;
        Ok(index)
    }

    pub fn finish_step_success(
        &mut self,
        index: usize,
        details: Value,
        artifacts: Vec<ArtifactRef>,
    ) -> Result<()> {
        if let Some(step) = self.report.steps.get_mut(index) {
            step.status = StepStatus::Success;
            step.exit_code = Some(0);
            step.ended_at = Some(now_rfc3339());
            step.duration_ms = duration_from(&step.started_at);
            step.details = details;
            step.failure = None;
            for artifact in artifacts {
                merge_artifact(&mut step.artifacts, artifact.clone());
                merge_artifact(&mut self.report.artifacts, artifact);
            }
        }
        self.save()
    }

    pub fn finish_step_failure(
        &mut self,
        index: usize,
        error: &ForgeError,
        extra_artifacts: Vec<ArtifactRef>,
    ) -> Result<()> {
        if let Some(step) = self.report.steps.get_mut(index) {
            step.status = StepStatus::Failed;
            step.exit_code = Some(error.exit_code());
            step.ended_at = Some(now_rfc3339());
            step.duration_ms = duration_from(&step.started_at);
            for artifact in &extra_artifacts {
                merge_artifact(&mut step.artifacts, artifact.clone());
            }
            let mut failure = FailureInfo::from_error(error, &step.name);
            for artifact in &step.artifacts {
                merge_artifact(&mut failure.artifacts, artifact.clone());
            }
            step.failure = Some(failure);
        }
        for artifact in extra_artifacts {
            merge_artifact(&mut self.report.artifacts, artifact);
        }
        let failure = FailureInfo::from_error(error, "run");
        if self.report.failure.is_none() {
            self.report.failure = Some(failure);
        }
        for suggestion in &error.body().suggestions {
            if !self.report.recovery_suggestions.contains(suggestion) {
                self.report.recovery_suggestions.push(suggestion.clone());
            }
        }
        self.save()
    }

    pub fn finish_step_skipped(&mut self, index: usize, reason: impl Into<String>) -> Result<()> {
        if let Some(step) = self.report.steps.get_mut(index) {
            step.status = StepStatus::Skipped;
            step.ended_at = Some(now_rfc3339());
            step.duration_ms = duration_from(&step.started_at);
            step.details = serde_json::json!({ "reason": reason.into() });
        }
        self.save()
    }

    pub fn add_artifact(&mut self, artifact_ref: ArtifactRef) -> Result<ArtifactRef> {
        merge_artifact(&mut self.report.artifacts, artifact_ref.clone());
        self.save()?;
        Ok(artifact_ref)
    }

    pub fn register_artifact(
        &mut self,
        path: &Path,
        kind: &str,
        description: Option<&str>,
    ) -> Result<ArtifactRef> {
        let artifact_ref = artifact::register(&self.dir, path, kind, description)?;
        self.add_artifact(artifact_ref)
    }

    pub fn write_binary_artifact(
        &mut self,
        relative: &str,
        kind: &str,
        description: Option<&str>,
        bytes: &[u8],
    ) -> Result<ArtifactRef> {
        let artifact_ref = artifact::write(&self.dir, relative, kind, description, bytes)?;
        self.add_artifact(artifact_ref)
    }

    pub fn write_text_artifact(
        &mut self,
        relative: &str,
        kind: &str,
        description: Option<&str>,
        text: &str,
    ) -> Result<ArtifactRef> {
        let redacted = self.redactor.redact(text);
        self.write_binary_artifact(relative, kind, description, redacted.as_bytes())
    }

    pub fn write_bytes(&self, relative: &str, bytes: &[u8]) -> Result<PathBuf> {
        let path = artifact::safe_join(&self.dir, relative)?;
        write_atomic(&path, bytes)?;
        Ok(path)
    }

    pub fn save(&mut self) -> Result<()> {
        self.report.duration_ms = duration_from(&self.report.started_at);
        ensure_dir(&self.dir)?;
        write_redacted_json(&self.report_path(), &self.report, &self.redactor)?;
        write_redacted_json(&self.metadata_path(), &self.metadata, &self.redactor)?;
        let manifest = artifact::build_manifest(&self.dir, &self.run_id)?;
        write_json_atomic(&self.manifest_path(), &manifest)?;
        Ok(())
    }

    pub fn finalize(&mut self, status: RunStatus, exit_code: i32) -> Result<()> {
        self.finalize_with_reason(
            status,
            exit_code,
            "the step did not finish before the run was finalized",
        )
    }

    pub fn finalize_with_reason(
        &mut self,
        status: RunStatus,
        exit_code: i32,
        reason: &str,
    ) -> Result<()> {
        for step in &mut self.report.steps {
            if step.status == StepStatus::Running {
                step.status = StepStatus::Failed;
                step.ended_at = Some(now_rfc3339());
                step.duration_ms = duration_from(&step.started_at);
                step.failure = Some(FailureInfo {
                    code: ErrorCode::Interrupted,
                    message: reason.to_string(),
                    failed_operation: step.name.clone(),
                    recoverable: true,
                    suggestions: vec![
                        "Inspect the run's logs and retry the failed operation.".to_string(),
                    ],
                    context: BTreeMap::new(),
                    artifacts: step.artifacts.clone(),
                });
            }
        }
        self.report.status = status;
        self.report.exit_code = exit_code;
        self.report.ended_at = Some(now_rfc3339());
        self.report.duration_ms = duration_from(&self.report.started_at);
        if self.report.failure.is_none()
            && let Some(step) = self.report.steps.iter().find(|step| step.failure.is_some())
        {
            self.report.failure = step.failure.clone();
        }
        self.save()
    }

    pub fn finalize_pass(&mut self) -> Result<()> {
        self.finalize(RunStatus::Passed, 0)
    }

    pub fn finalize_failure(&mut self, error: &ForgeError) -> Result<()> {
        if self.report.failure.is_none() {
            self.report.failure = Some(FailureInfo::from_error(error, "run"));
        }
        for suggestion in &error.body().suggestions {
            if !self.report.recovery_suggestions.contains(suggestion) {
                self.report.recovery_suggestions.push(suggestion.clone());
            }
        }
        self.finalize(RunStatus::Failed, error.exit_code())
    }

    pub fn finalize_partial(&mut self) -> Result<()> {
        self.finalize(RunStatus::Partial, ErrorCode::PartialFailure.exit_code())
    }

    pub fn redactor(&self) -> &Redactor {
        &self.redactor
    }

    pub fn set_redactor(&mut self, redactor: Redactor) {
        self.redactor = redactor;
    }
}

fn duration_from(started_at: &str) -> u64 {
    let Ok(started) = DateTime::parse_from_rfc3339(started_at) else {
        return 0;
    };
    let elapsed = Utc::now().signed_duration_since(started.with_timezone(&Utc));
    elapsed.num_milliseconds().max(0).min(i64::from(u32::MAX)) as u64
}

fn merge_artifact(list: &mut Vec<ArtifactRef>, artifact: ArtifactRef) {
    if !list
        .iter()
        .any(|existing| existing.kind == artifact.kind && existing.path == artifact.path)
    {
        list.push(artifact);
    }
}

fn write_redacted_json<T: Serialize>(path: &Path, value: &T, redactor: &Redactor) -> Result<()> {
    let mut json = serde_json::to_value(value)?;
    redactor.redact_value(&mut json);
    let mut bytes = serde_json::to_vec_pretty(&json)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveRun {
    pub run_id: String,
    pub session_id: Option<String>,
    pub project_root: String,
    pub created_at: String,
}

pub fn active_run_key(session_id: Option<&str>, project_root: &Path) -> String {
    let base = session_id
        .map(|id| {
            id.chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                        ch
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        })
        .filter(|key| !key.is_empty())
        .unwrap_or_else(|| "default".to_string());
    if session_id.is_some() {
        // A session is the workspace identity: `run start` at the repository
        // root and a later `--project examples/App` command must join the same
        // active run, not silently create a second one.
        return base;
    }
    // Without a session the pointer is global, not inside the project's
    // .artifacts directory. Include a stable project hash so two projects
    // cannot overwrite each other's pointer.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in project_root.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{base}-{hash:016x}")
}

pub fn active_run_path(paths: &ForgePaths, key: &str) -> PathBuf {
    paths.active_runs_dir().join(format!("{key}.json"))
}

pub fn set_active_run(
    paths: &ForgePaths,
    session_id: Option<&str>,
    run_id: &str,
    project_root: &Path,
) -> Result<ActiveRun> {
    let active = ActiveRun {
        run_id: run_id.to_string(),
        session_id: session_id.map(str::to_string),
        project_root: project_root.to_string_lossy().into_owned(),
        created_at: now_rfc3339(),
    };
    let key = active_run_key(session_id, project_root);
    write_json_atomic(&active_run_path(paths, &key), &active)?;
    Ok(active)
}

pub fn get_active_run(paths: &ForgePaths, session_id: Option<&str>) -> Result<Option<ActiveRun>> {
    let key = active_run_key(session_id, &paths.project_root);
    let path = active_run_path(paths, &key);
    if !path.is_file() {
        return Ok(None);
    }
    let active: ActiveRun = read_json(&path)?;
    if active.project_root != paths.project_root.to_string_lossy()
        || !paths.run_dir(&active.run_id).join("report.json").is_file()
    {
        let _ = fs::remove_file(&path);
        return Ok(None);
    }
    Ok(Some(active))
}

pub fn clear_active_run(paths: &ForgePaths, session_id: Option<&str>) -> Result<()> {
    let key = active_run_key(session_id, &paths.project_root);
    let path = active_run_path(paths, &key);
    if path.is_file() {
        fs::remove_file(&path).map_err(|error| {
            ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!("could not clear active run {}: {error}", path.display()),
            )
        })?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: String,
    pub command: String,
    pub project: String,
    pub status: RunStatus,
    pub exit_code: i32,
    pub started_at: String,
    pub duration_ms: u64,
    pub steps: usize,
    pub failed_steps: usize,
    pub path: String,
}

pub fn list_runs(artifacts_root: &Path) -> Result<Vec<RunSummary>> {
    let runs_dir = artifacts_root.join("runs");
    if !runs_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut runs = Vec::new();
    for entry in fs::read_dir(&runs_dir).map_err(|error| {
        ForgeError::new(
            ErrorCode::Internal,
            format!("could not read {}: {error}", runs_dir.display()),
        )
    })? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let report_path = path.join("report.json");
        if !report_path.is_file() {
            continue;
        }
        let report: RunReport = match read_json(&report_path) {
            Ok(report) => report,
            Err(_) => continue,
        };
        let failed_steps = report.failed_steps();
        let steps = report.steps.len();
        runs.push(RunSummary {
            run_id: report.run_id,
            command: report.command,
            project: report.project,
            status: report.status,
            exit_code: report.exit_code,
            started_at: report.started_at,
            duration_ms: report.duration_ms,
            steps,
            failed_steps,
            path: relative_to(&path, artifacts_root),
        });
    }
    runs.sort_by(|left, right| right.started_at.cmp(&left.started_at));
    Ok(runs)
}

pub fn resolve_run_dir(artifacts_root: &Path, spec: &str) -> Result<PathBuf> {
    if spec == "latest" {
        let runs = list_runs(artifacts_root)?;
        let latest = runs.into_iter().next().ok_or_else(|| {
            ForgeError::new(ErrorCode::InvalidArgument, "no AndroidForge runs exist yet")
                .with_suggestion("Start a run with `androidforge run start --name <name>`.")
        })?;
        return Ok(artifacts_root.join(&latest.path));
    }
    let as_path = Path::new(spec);
    if as_path.is_dir() && as_path.join("report.json").is_file() {
        return Ok(as_path.to_path_buf());
    }
    let candidate = artifacts_root.join("runs").join(spec);
    if candidate.join("report.json").is_file() {
        return Ok(candidate);
    }
    Err(ForgeError::new(
        ErrorCode::InvalidArgument,
        format!("run '{spec}' was not found"),
    )
    .with_suggestion("Use `androidforge report show latest` or `androidforge report path <run>`."))
}

pub fn unique_artifact_paths(artifacts: impl IntoIterator<Item = ArtifactRef>) -> Vec<ArtifactRef> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for artifact in artifacts {
        if seen.insert((artifact.kind.clone(), artifact.path.clone())) {
            out.push(artifact);
        }
    }
    out
}

pub fn error_body_to_failure(error: &ErrorBody, operation: &str) -> FailureInfo {
    FailureInfo {
        code: error.code,
        message: error.message.clone(),
        failed_operation: operation.to_string(),
        recoverable: error.recoverable,
        suggestions: error.suggestions.clone(),
        context: error.context.clone(),
        artifacts: error.artifacts.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_finalizes_report_with_steps() {
        let temp = tempfile::tempdir().unwrap();
        let mut options = RunOptions::new(
            temp.path().join(".artifacts"),
            temp.path(),
            "Example",
            "ui.assert",
        );
        options.name = Some("Smoke test".to_string());
        let mut context = RunContext::create(options, Redactor::default()).unwrap();
        let step = context
            .start_step("assert welcome", "ui.assert --text Welcome")
            .unwrap();
        context
            .finish_step_success(step, serde_json::json!({ "matched": true }), Vec::new())
            .unwrap();
        context.finalize_pass().unwrap();
        assert!(context.report_path().is_file());
        let report: RunReport = read_json(&context.report_path()).unwrap();
        assert_eq!(report.status, RunStatus::Passed);
        assert_eq!(report.steps.len(), 1);
        assert!(context.manifest_path().is_file());
    }

    #[test]
    fn active_run_round_trip_and_stale_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let mut paths = ForgePaths::discover(Some(project.clone()), None).unwrap();
        paths.state_dir = temp.path().join("state");
        let run_dir = paths.run_dir("run-1");
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::write(run_dir.join("report.json"), "{}").unwrap();
        let active = set_active_run(&paths, Some("ses-123"), "run-1", &paths.project_root).unwrap();
        assert_eq!(active.run_id, "run-1");
        assert!(get_active_run(&paths, Some("ses-123")).unwrap().is_some());
        clear_active_run(&paths, Some("ses-123")).unwrap();
        assert!(get_active_run(&paths, Some("ses-123")).unwrap().is_none());
    }
}
