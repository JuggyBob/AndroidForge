use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use androidforge_app::{AppFiles, AppManager, AppOps, InstallOptions, Permissions};
use androidforge_core::config::ForgeConfig;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::ArtifactRef;
use androidforge_core::paths::ForgePaths;
use androidforge_core::run::{RunContext, StepStatus};
use androidforge_core::session::Session;
use androidforge_core::util::ensure_dir;
use androidforge_diagnostics::capture_failure_bundle;
use androidforge_process::CommandSpec;
use androidforge_ui::{Selector, UiManager};
use serde::{Deserialize, Serialize};

const FIXTURE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureManifest {
    #[serde(default = "default_version")]
    pub version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_timeout_secs: Option<u64>,
    #[serde(default)]
    pub requires_confirmation: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<String, String>,
    #[serde(default)]
    pub steps: Vec<FixtureStep>,
    #[serde(skip)]
    pub source_path: Option<String>,
}

fn default_version() -> u32 {
    FIXTURE_VERSION
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FixturePhase {
    PreInstall,
    PostInstall,
    PreTest,
    PostTest,
    Cleanup,
}

impl FixturePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreInstall => "pre-install",
            Self::PostInstall => "post-install",
            Self::PreTest => "pre-test",
            Self::PostTest => "post-test",
            Self::Cleanup => "cleanup",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OnFailure {
    #[default]
    Stop,
    Continue,
    Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapturePolicy {
    None,
    #[default]
    OnFailure,
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureStep {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<FixturePhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub retries: u32,
    #[serde(default)]
    pub on_failure: OnFailure,
    #[serde(default)]
    pub capture: CapturePolicy,
    #[serde(default)]
    pub destructive: bool,
    pub action: FixtureAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FixtureAction {
    Adb {
        args: Vec<String>,
    },
    Shell {
        command: String,
    },
    Install {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        apk: Option<String>,
        #[serde(default)]
        fresh: bool,
    },
    Uninstall {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
        #[serde(default)]
        keep_data: bool,
    },
    ClearData {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
    },
    Launch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        activity: Option<String>,
    },
    ForceStop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
    },
    Grant {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
        permission: String,
    },
    Revoke {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
        permission: String,
    },
    AppOp {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
        operation: String,
        mode: String,
    },
    Push {
        local: String,
        remote: String,
    },
    Pull {
        remote: String,
        local: String,
    },
    AppFilePush {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
        local: String,
        remote: String,
        #[serde(default)]
        external: bool,
    },
    AppFilePull {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
        remote: String,
        local: String,
        #[serde(default)]
        external: bool,
    },
    Settings {
        namespace: String,
        key: String,
        value: String,
    },
    Wait {
        ms: u64,
    },
    Assert {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text_contains: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_desc: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_desc_contains: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        class: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        #[serde(default)]
        not_exists: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
    },
    Script {
        command: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
    },
    BackupAppData {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
        path: String,
    },
    RestoreAppData {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
        path: String,
    },
}

impl FixtureAction {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Adb { .. } => "adb",
            Self::Shell { .. } => "shell",
            Self::Install { .. } => "install",
            Self::Uninstall { .. } => "uninstall",
            Self::ClearData { .. } => "clear-data",
            Self::Launch { .. } => "launch",
            Self::ForceStop { .. } => "force-stop",
            Self::Grant { .. } => "grant",
            Self::Revoke { .. } => "revoke",
            Self::AppOp { .. } => "appop",
            Self::Push { .. } => "push",
            Self::Pull { .. } => "pull",
            Self::AppFilePush { .. } => "app-file-push",
            Self::AppFilePull { .. } => "app-file-pull",
            Self::Settings { .. } => "settings",
            Self::Wait { .. } => "wait",
            Self::Assert { .. } => "assert",
            Self::Script { .. } => "script",
            Self::BackupAppData { .. } => "backup-app-data",
            Self::RestoreAppData { .. } => "restore-app-data",
        }
    }

    pub fn is_destructive(&self) -> bool {
        match self {
            Self::Uninstall { .. } | Self::ClearData { .. } | Self::RestoreAppData { .. } => true,
            Self::Install { fresh, .. } => *fresh,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureSummary {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub path: String,
    pub valid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureValidation {
    pub valid: bool,
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureStatus {
    Planned,
    Success,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureStepOutcome {
    pub index: usize,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<FixturePhase>,
    pub action: String,
    pub status: StepStatus,
    pub attempts: u32,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureRunResult {
    pub name: String,
    pub path: String,
    pub status: FixtureStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<FixtureStepOutcome>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub duration_ms: u64,
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub struct FixtureRunOptions {
    pub name: String,
    pub phases: Vec<FixturePhase>,
    pub dry_run: bool,
    pub confirmed: bool,
    pub vars: BTreeMap<String, String>,
    pub apk: Option<PathBuf>,
    pub trace: bool,
}

impl FixtureRunOptions {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            phases: Vec::new(),
            dry_run: false,
            confirmed: false,
            vars: BTreeMap::new(),
            apk: None,
            trace: false,
        }
    }
}

pub fn load_fixture(config: &ForgeConfig, name: &str) -> Result<FixtureManifest> {
    let path = fixture_path(config, name)?;
    let text = std::fs::read_to_string(&path).map_err(|error| {
        ForgeError::new(
            ErrorCode::FixtureInvalid,
            format!("could not read fixture {}: {error}", path.display()),
        )
    })?;
    let mut manifest = parse_fixture_text(&path, &text).map_err(|error| {
        ForgeError::new(
            ErrorCode::FixtureInvalid,
            format!("could not parse fixture {}: {error}", path.display()),
        )
        .with_suggestion("Run `androidforge fixture validate <name>` after fixing the manifest.")
    })?;
    manifest.source_path = Some(path.to_string_lossy().into_owned());
    validate_manifest(config, &manifest)?;
    Ok(manifest)
}

fn parse_fixture_text(path: &Path, text: &str) -> std::result::Result<FixtureManifest, String> {
    let is_json = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
    if is_json {
        serde_json::from_str(text).map_err(|error| error.to_string())
    } else {
        toml::from_str(text).map_err(|error| error.to_string())
    }
}

fn fixture_manifest_path(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        for name in ["fixture.toml", "fixture.json"] {
            let candidate = path.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    } else if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("toml") || extension.eq_ignore_ascii_case("json")
        })
    {
        Some(path.to_path_buf())
    } else {
        None
    }
}

fn fixture_path(config: &ForgeConfig, name: &str) -> Result<PathBuf> {
    let direct = config.fixture_path(name);
    if direct.is_file() {
        return Ok(direct);
    }
    let fixtures = list_fixtures(config).unwrap_or_default();
    let available = fixtures
        .iter()
        .map(|fixture| fixture.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    Err(ForgeError::new(
        ErrorCode::FixtureInvalid,
        format!("fixture '{name}' was not found"),
    )
    .with_context_string(
        "fixtures_directory",
        config.fixtures_dir().to_string_lossy(),
    )
    .with_context_string("available", available)
    .with_suggestion("Run `androidforge fixture list` to inspect fixture packs.")
    .with_suggestion(
        "Create .androidforge/fixtures/<name>/fixture.toml, <name>.toml, <name>/fixture.json or <name>.json.",
    ))
}

pub fn list_fixtures(config: &ForgeConfig) -> Result<Vec<FixtureSummary>> {
    let dir = config.fixtures_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut summaries = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|error| {
        ForgeError::new(
            ErrorCode::FixtureInvalid,
            format!(
                "could not read fixture directory {}: {error}",
                dir.display()
            ),
        )
    })? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = match fixture_manifest_path(&entry.path()) {
            Some(path) => path,
            None => continue,
        };
        if !path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        match parse_fixture_text(&path, &text) {
            Ok(manifest) => {
                let valid = validate_manifest(config, &manifest).is_ok();
                let error = validate_manifest(config, &manifest)
                    .err()
                    .map(|error| error.message().to_string());
                summaries.push(FixtureSummary {
                    name: manifest.name,
                    description: manifest.description,
                    path: path.to_string_lossy().into_owned(),
                    valid,
                    error,
                });
            }
            Err(error) => {
                let name = path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                summaries.push(FixtureSummary {
                    name,
                    description: None,
                    path: path.to_string_lossy().into_owned(),
                    valid: false,
                    error: Some(error.to_string()),
                });
            }
        }
    }
    summaries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(summaries)
}

pub fn validate_fixture(config: &ForgeConfig, name: &str) -> Result<FixtureValidation> {
    let path = fixture_path(config, name)?;
    let text = std::fs::read_to_string(&path).map_err(|error| {
        ForgeError::new(
            ErrorCode::FixtureInvalid,
            format!("could not read fixture: {error}"),
        )
    })?;
    let manifest = parse_fixture_text(&path, &text).map_err(|error| {
        ForgeError::new(
            ErrorCode::FixtureInvalid,
            format!("could not parse fixture: {error}"),
        )
    })?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    validate_manifest_report(config, &manifest, &mut errors, &mut warnings);
    Ok(FixtureValidation {
        valid: errors.is_empty(),
        name: manifest.name,
        path: path.to_string_lossy().into_owned(),
        errors,
        warnings,
    })
}

fn validate_manifest(config: &ForgeConfig, manifest: &FixtureManifest) -> Result<()> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    validate_manifest_report(config, manifest, &mut errors, &mut warnings);
    if !errors.is_empty() {
        return Err(ForgeError::new(
            ErrorCode::FixtureInvalid,
            format!("fixture '{}' is invalid", manifest.name),
        )
        .with_context("errors", serde_json::to_value(&errors).unwrap_or_default())
        .with_context(
            "warnings",
            serde_json::to_value(&warnings).unwrap_or_default(),
        )
        .with_suggestion("Run `androidforge fixture validate <name>`."));
    }
    Ok(())
}

fn validate_manifest_report(
    config: &ForgeConfig,
    manifest: &FixtureManifest,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    if manifest.version != FIXTURE_VERSION {
        errors.push(format!(
            "unsupported fixture version {}; expected {FIXTURE_VERSION}",
            manifest.version
        ));
    }
    if manifest.name.trim().is_empty() {
        errors.push("fixture name cannot be empty".to_string());
    }
    if manifest.steps.is_empty() {
        warnings.push("fixture has no steps".to_string());
    }
    let mut names = BTreeSet::new();
    for (index, step) in manifest.steps.iter().enumerate() {
        let label = step.name.clone().unwrap_or_else(|| format!("step-{index}"));
        if !names.insert(label.clone()) {
            warnings.push(format!("duplicate step name '{label}'"));
        }
        if step.retries > 20 {
            errors.push(format!("{label}: retries must be 20 or less"));
        }
        if step.timeout_secs == Some(0) {
            errors.push(format!("{label}: timeout_secs must be greater than zero"));
        }
        match &step.action {
            FixtureAction::Script { command, .. } => {
                if command.is_empty() {
                    errors.push(format!("{label}: script command cannot be empty"));
                }
            }
            FixtureAction::Install { apk: None, .. } => {
                if config.debug_package().is_none() {
                    warnings.push(format!(
                        "{label}: install has no APK and the project has no package; provide apk or --apk"
                    ));
                }
            }
            FixtureAction::AppFilePush { remote, .. }
            | FixtureAction::AppFilePull { remote, .. } => {
                if remote.starts_with('/') || remote.contains("..") {
                    errors.push(format!(
                        "{label}: app-private paths must be relative and may not contain '..'"
                    ));
                }
            }
            FixtureAction::Wait { ms } => {
                if *ms > 60_000 {
                    warnings.push(format!(
                        "{label}: a fixed wait longer than 60s is usually a design smell"
                    ));
                }
            }
            FixtureAction::Assert {
                text,
                text_contains,
                content_desc,
                content_desc_contains,
                resource_id,
                class,
                ..
            } => {
                if text.is_none()
                    && text_contains.is_none()
                    && content_desc.is_none()
                    && content_desc_contains.is_none()
                    && resource_id.is_none()
                    && class.is_none()
                {
                    errors.push(format!(
                        "{label}: assert needs at least one selector condition"
                    ));
                }
            }
            FixtureAction::Uninstall { .. }
            | FixtureAction::ClearData { .. }
            | FixtureAction::RestoreAppData { .. }
                if !manifest.requires_confirmation =>
            {
                warnings.push(format!(
                    "{label}: destructive step is present; fixture run will require --yes"
                ));
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_fixture(
    config: &ForgeConfig,
    session: &Session,
    app: &AppManager,
    ui: &UiManager,
    paths: &ForgePaths,
    run: &mut RunContext,
    options: &FixtureRunOptions,
) -> Result<FixtureRunResult> {
    let manifest = load_fixture(config, &options.name)?;
    let started = Instant::now();
    let serial = session.serial()?.to_string();
    let selected = manifest
        .steps
        .iter()
        .enumerate()
        .filter(|(_, step)| {
            options.phases.is_empty()
                || step
                    .phase
                    .map(|phase| options.phases.contains(&phase))
                    .unwrap_or(true)
        })
        .collect::<Vec<_>>();

    let destructive = manifest.requires_confirmation
        || selected
            .iter()
            .any(|(_, step)| step.destructive || step.action.is_destructive());
    if destructive && !options.confirmed && !options.dry_run {
        return Err(ForgeError::new(
            ErrorCode::FixtureFailed,
            format!(
                "fixture '{}' contains destructive operations and was not confirmed",
                manifest.name
            ),
        )
        .with_suggestion("Re-run with --yes after reviewing the fixture steps.")
        .with_context(
            "destructive_steps",
            serde_json::to_value(
                selected
                    .iter()
                    .filter(|(_, step)| step.destructive || step.action.is_destructive())
                    .map(|(_, step)| step.name.clone().unwrap_or_default())
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default(),
        )
        .session(session.id.clone()));
    }

    let mut env = fixture_environment(config, session, run, options, "setup");
    env.extend(manifest.vars.clone());
    let mut outcomes = Vec::new();
    let mut artifacts = Vec::new();
    let mut warnings = Vec::new();
    let mut partial = false;

    for (index, step) in selected {
        let name = step
            .name
            .clone()
            .unwrap_or_else(|| format!("{} step {}", step.action.kind(), index + 1));
        let phase = step.phase;
        let command = describe_action(&step.action);
        let run_step = run.start_step(format!("fixture:{}", name), command.clone())?;
        if options.dry_run {
            run.finish_step_skipped(run_step, "dry run")?;
            outcomes.push(FixtureStepOutcome {
                index,
                name,
                phase,
                action: step.action.kind().to_string(),
                status: StepStatus::Skipped,
                attempts: 0,
                duration_ms: 0,
                error: None,
                artifacts: Vec::new(),
            });
            continue;
        }
        let step_started = Instant::now();
        let mut attempts = 0u32;
        let mut last_error = None;
        let mut step_artifacts = Vec::new();
        while attempts <= step.retries {
            attempts += 1;
            match execute_step(
                config, &serial, app, ui, paths, run, options, &env, step, index,
            )
            .await
            {
                Ok(mut execution) => {
                    step_artifacts.append(&mut execution.artifacts);
                    if step.capture == CapturePolicy::Always {
                        match androidforge_diagnostics::capture_screenshot(
                            &app.adb,
                            &serial,
                            run,
                            Some(&name),
                            Duration::from_secs(30),
                        )
                        .await
                        {
                            Ok(screenshot) => step_artifacts.push(screenshot.artifact),
                            Err(error) => warnings.push(format!(
                                "fixture post-step screenshot failed: {}",
                                error.message()
                            )),
                        }
                    }
                    run.finish_step_success(run_step, execution.details, step_artifacts.clone())?;
                    artifacts.extend(step_artifacts.clone());
                    outcomes.push(FixtureStepOutcome {
                        index,
                        name: name.clone(),
                        phase,
                        action: step.action.kind().to_string(),
                        status: StepStatus::Success,
                        attempts,
                        duration_ms: elapsed_ms(step_started),
                        error: None,
                        artifacts: step_artifacts.clone(),
                    });
                    last_error = None;
                    break;
                }
                Err(mut error) => {
                    if step.capture != CapturePolicy::None {
                        let bundle = capture_failure_bundle(
                            &app.adb,
                            Some(ui),
                            Some(app),
                            run,
                            &serial,
                            config.package(),
                            &format!("fixture {} step {}", manifest.name, name),
                            true,
                        )
                        .await;
                        step_artifacts.extend(bundle.artifacts.clone());
                        if !bundle.warnings.is_empty() {
                            warnings.extend(bundle.warnings);
                        }
                        error = error.with_artifacts(bundle.artifacts);
                    }
                    if attempts <= step.retries {
                        last_error = Some(error);
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        continue;
                    }
                    last_error = Some(error);
                    break;
                }
            }
        }

        if let Some(error) = last_error {
            run.finish_step_failure(run_step, &error, step_artifacts.clone())?;
            artifacts.extend(step_artifacts.clone());
            outcomes.push(FixtureStepOutcome {
                index,
                name: name.clone(),
                phase,
                action: step.action.kind().to_string(),
                status: StepStatus::Failed,
                attempts,
                duration_ms: elapsed_ms(step_started),
                error: Some(error.message().to_string()),
                artifacts: step_artifacts,
            });
            match step.on_failure {
                OnFailure::Continue => {
                    partial = true;
                    warnings.push(format!(
                        "step '{name}' failed; continuing because on_failure=continue"
                    ));
                }
                OnFailure::Cleanup => {
                    partial = true;
                    warnings.push(format!("step '{name}' failed; running cleanup phase"));
                    run_cleanup_steps(
                        config,
                        &serial,
                        app,
                        ui,
                        paths,
                        run,
                        options,
                        &env,
                        &mut warnings,
                    )
                    .await;
                }
                OnFailure::Stop => {
                    return Err(error
                        .with_context_string("fixture", manifest.name.clone())
                        .with_context_string("failed_step", name));
                }
            }
        }
    }

    let status = if outcomes
        .iter()
        .any(|step| step.status == StepStatus::Failed)
    {
        if partial {
            FixtureStatus::Partial
        } else {
            FixtureStatus::Failed
        }
    } else if options.dry_run {
        FixtureStatus::Planned
    } else {
        FixtureStatus::Success
    };
    Ok(FixtureRunResult {
        name: manifest.name,
        path: manifest.source_path.unwrap_or_default(),
        status,
        steps: outcomes,
        artifacts,
        warnings,
        duration_ms: elapsed_ms(started),
        dry_run: options.dry_run,
    })
}

#[allow(clippy::too_many_arguments)]
async fn execute_step(
    config: &ForgeConfig,
    serial: &str,
    app: &AppManager,
    ui: &UiManager,
    paths: &ForgePaths,
    run: &mut RunContext,
    options: &FixtureRunOptions,
    env: &BTreeMap<String, String>,
    step: &FixtureStep,
    index: usize,
) -> Result<StepExecution> {
    let timeout = Duration::from_secs(
        step.timeout_secs
            .unwrap_or(config.file.timeouts.fixture_secs),
    );
    let mut vars = env.clone();
    vars.extend(options.vars.clone());
    vars.insert(
        "ANDROIDFORGE_PHASE".to_string(),
        step.phase
            .map(|phase| phase.as_str().to_string())
            .unwrap_or_else(|| "test".to_string()),
    );
    vars.insert(
        "ANDROIDFORGE_ADB".to_string(),
        app.adb.adb.to_string_lossy().into_owned(),
    );
    let mut artifacts = Vec::new();
    let mut details = serde_json::json!({ "action": step.action.kind() });
    match &step.action {
        FixtureAction::Adb { args } => {
            let resolved = args
                .iter()
                .map(|arg| resolve_template(arg, &vars, &config.paths.project_root))
                .collect::<Result<Vec<_>>>()?;
            let output = app
                .adb
                .run_checked(
                    Some(serial),
                    &resolved,
                    Some(timeout),
                    Some(8 * 1024 * 1024),
                )
                .await?;
            let artifact = run.write_text_artifact(
                &format!("logs/fixture-{:03}-adb.txt", index + 1),
                "command_output",
                Some("Fixture adb output"),
                &format!("{}\n{}", output.stdout_text(), output.stderr_text()),
            )?;
            artifacts.push(artifact);
        }
        FixtureAction::Shell { command } => {
            let command = resolve_template(command, &vars, &config.paths.project_root)?;
            let output = app
                .adb
                .run_checked(
                    Some(serial),
                    &["shell".to_string(), command],
                    Some(timeout),
                    Some(8 * 1024 * 1024),
                )
                .await?;
            let artifact = run.write_text_artifact(
                &format!("logs/fixture-{:03}-shell.txt", index + 1),
                "command_output",
                Some("Fixture shell output"),
                &format!("{}\n{}", output.stdout_text(), output.stderr_text()),
            )?;
            artifacts.push(artifact);
        }
        FixtureAction::Install { apk, fresh } => {
            let apk = apk
                .as_ref()
                .map(|path| resolve_template(path, &vars, &config.paths.project_root))
                .transpose()?
                .map(PathBuf::from)
                .or_else(|| options.apk.clone())
                .or_else(|| env.get("ANDROIDFORGE_APK").map(PathBuf::from))
                .ok_or_else(|| {
                    ForgeError::new(
                        ErrorCode::FixtureInvalid,
                        "install step needs an apk path or a fixture --apk argument",
                    )
                })?;
            let mut install = InstallOptions::new(apk.clone());
            install.fresh = *fresh;
            install.confirmed = options.confirmed;
            if let Some(package) = config.package() {
                install.allowed_packages.push(package.to_string());
            }
            if let Some(package) = config.debug_package()
                && !install
                    .allowed_packages
                    .iter()
                    .any(|value| value == package)
            {
                install.allowed_packages.push(package.to_string());
            }
            install.expected_cert_sha256 = config.file.project.expected_cert_sha256.clone();
            install.verify_installed = config.file.project.verify_installed_apk;
            install.timeout = timeout;
            let result = app.install(serial, &install).await?;
            run.set_build(result.build.clone());
            let identity = serde_json::to_string_pretty(&result.build).unwrap_or_default();
            artifacts.push(run.write_text_artifact(
                &format!("build/fixture-install-{:03}.json", index + 1),
                "build_identity",
                Some("Build identity after fixture install"),
                &identity,
            )?);
            details = serde_json::json!({
                "action": "install",
                "package": result.build.package,
                "version_name": result.build.version_name,
                "version_code": result.build.version_code,
                "sha256": result.build.apk_sha256,
            });
        }
        FixtureAction::Uninstall { package, keep_data } => {
            let package = package_or_config(package, config)?;
            ensure_confirmed(options, "uninstall")?;
            app.uninstall(serial, &package, *keep_data).await?;
        }
        FixtureAction::ClearData { package } => {
            let package = package_or_config(package, config)?;
            ensure_confirmed(options, "clear-data")?;
            app.clear_data(serial, &package).await?;
        }
        FixtureAction::Launch { package, activity } => {
            let package = package_or_config(package, config)?;
            let activity = activity
                .as_ref()
                .map(|value| resolve_template(value, &vars, &config.paths.project_root))
                .transpose()?;
            let launch = app
                .launch(serial, &package, activity.as_deref(), true)
                .await?;
            details = serde_json::to_value(&launch).unwrap_or_default();
        }
        FixtureAction::ForceStop { package } => {
            let package = package_or_config(package, config)?;
            app.force_stop(serial, &package).await?;
        }
        FixtureAction::Grant {
            package,
            permission,
        } => {
            let package = package_or_config(package, config)?;
            let permission = resolve_template(permission, &vars, &config.paths.project_root)?;
            let state = Permissions::new(app.adb.clone())
                .grant(serial, &package, &permission)
                .await?;
            details = serde_json::to_value(&state).unwrap_or_default();
        }
        FixtureAction::Revoke {
            package,
            permission,
        } => {
            let package = package_or_config(package, config)?;
            let permission = resolve_template(permission, &vars, &config.paths.project_root)?;
            let state = Permissions::new(app.adb.clone())
                .revoke(serial, &package, &permission)
                .await?;
            details = serde_json::to_value(&state).unwrap_or_default();
        }
        FixtureAction::AppOp {
            package,
            operation,
            mode,
        } => {
            let package = package_or_config(package, config)?;
            let operation = resolve_template(operation, &vars, &config.paths.project_root)?;
            let mode = resolve_template(mode, &vars, &config.paths.project_root)?;
            let state = AppOps::new(app.adb.clone())
                .set(serial, &package, &operation, &mode)
                .await?;
            details = serde_json::to_value(&state).unwrap_or_default();
        }
        FixtureAction::Push { local, remote } => {
            let local = resolve_local(local, &vars, &config.paths.project_root)?;
            let remote = resolve_template(remote, &vars, &config.paths.project_root)?;
            app.adb.push(serial, &local, &remote).await?;
        }
        FixtureAction::Pull { remote, local } => {
            let remote = resolve_template(remote, &vars, &config.paths.project_root)?;
            let local = resolve_local(local, &vars, &config.paths.project_root)?;
            if let Some(parent) = local.parent() {
                ensure_dir(parent)?;
            }
            app.adb.pull(serial, &remote, &local).await?;
        }
        FixtureAction::AppFilePush {
            package,
            local,
            remote,
            external,
        } => {
            let package = package_or_config(package, config)?;
            let local = resolve_local(local, &vars, &config.paths.project_root)?;
            let remote = resolve_template(remote, &vars, &config.paths.project_root)?;
            let transfer = AppFiles::new(app.adb.clone())
                .push(serial, &package, &local, &remote, *external)
                .await?;
            details = serde_json::to_value(&transfer).unwrap_or_default();
            artifacts.push(run.write_text_artifact(
                &format!("logs/fixture-{:03}-app-file-push.json", index + 1),
                "file_transfer",
                Some("App file transfer result"),
                &serde_json::to_string_pretty(&transfer).unwrap_or_default(),
            )?);
        }
        FixtureAction::AppFilePull {
            package,
            remote,
            local,
            external,
        } => {
            let package = package_or_config(package, config)?;
            let remote = resolve_template(remote, &vars, &config.paths.project_root)?;
            let local = resolve_local(local, &vars, &config.paths.project_root)?;
            let transfer = AppFiles::new(app.adb.clone())
                .pull(serial, &package, &remote, &local, *external)
                .await?;
            details = serde_json::to_value(&transfer).unwrap_or_default();
        }
        FixtureAction::Settings {
            namespace,
            key,
            value,
        } => {
            let namespace = resolve_template(namespace, &vars, &config.paths.project_root)?;
            let key = resolve_template(key, &vars, &config.paths.project_root)?;
            let value = resolve_template(value, &vars, &config.paths.project_root)?;
            app.adb
                .settings_put(serial, &namespace, &key, &value)
                .await?;
            let actual = app.adb.settings_get(serial, &namespace, &key).await?;
            if actual.trim() != value {
                return Err(ForgeError::new(
                    ErrorCode::FixtureFailed,
                    format!("settings {namespace}:{key} did not accept the requested value"),
                )
                .with_context_string("expected", value)
                .with_context_string("actual", actual));
            }
        }
        FixtureAction::Wait { ms } => {
            if *ms > 120_000 {
                return Err(ForgeError::invalid_argument(
                    "fixture wait cannot exceed 120 seconds",
                ));
            }
            tokio::time::sleep(Duration::from_millis(*ms)).await;
        }
        FixtureAction::Assert {
            text,
            text_contains,
            content_desc,
            content_desc_contains,
            resource_id,
            class,
            index,
            not_exists,
            timeout_secs,
        } => {
            let resolved_text = resolve_opt(text, &vars, &config.paths.project_root)?;
            let resolved_text_contains =
                resolve_opt(text_contains, &vars, &config.paths.project_root)?;
            let resolved_content_desc =
                resolve_opt(content_desc, &vars, &config.paths.project_root)?;
            let resolved_content_desc_contains =
                resolve_opt(content_desc_contains, &vars, &config.paths.project_root)?;
            let resolved_resource_id = resolve_opt(resource_id, &vars, &config.paths.project_root)?;
            let resolved_class = resolve_opt(class, &vars, &config.paths.project_root)?;
            let selector = Selector {
                text: resolved_text,
                text_contains: resolved_text_contains,
                content_desc: resolved_content_desc,
                content_desc_contains: resolved_content_desc_contains,
                resource_id: resolved_resource_id,
                class: resolved_class,
                index: *index,
                ..Selector::default()
            };
            let result = ui
                .wait(
                    serial,
                    &selector,
                    Duration::from_secs(
                        timeout_secs.unwrap_or(config.file.ui.default_timeout_secs),
                    ),
                    *not_exists,
                )
                .await?;
            details = serde_json::to_value(&result).unwrap_or_default();
        }
        FixtureAction::Script {
            command,
            env: step_env,
        } => {
            if command.is_empty() {
                return Err(ForgeError::invalid_argument(
                    "fixture script command is empty",
                ));
            }
            let mut resolved = Vec::with_capacity(command.len());
            for arg in command {
                resolved.push(resolve_template(arg, &vars, &config.paths.project_root)?);
            }
            let mut script_env = env.clone();
            for (key, value) in step_env {
                script_env.insert(
                    key.clone(),
                    resolve_template(value, &vars, &config.paths.project_root)?,
                );
            }
            let program = resolved.remove(0);
            let mut spec = CommandSpec::new(program)
                .args(resolved)
                .cwd(config.paths.project_root.clone())
                .timeout(timeout)
                .trace(options.trace)
                .hide_window(true);
            for (key, value) in &script_env {
                spec = spec.env(key, value);
            }
            let output = androidforge_process::Runner::new(options.trace)
                .run(spec)
                .await?;
            let stdout = output.stdout_text();
            let stderr = output.stderr_text();
            artifacts.push(run.write_text_artifact(
                &format!("logs/fixture-{:03}-script.stdout.txt", index + 1),
                "command_output",
                Some("Fixture script stdout"),
                &stdout,
            )?);
            if !stderr.is_empty() {
                artifacts.push(run.write_text_artifact(
                    &format!("logs/fixture-{:03}-script.stderr.txt", index + 1),
                    "command_output",
                    Some("Fixture script stderr"),
                    &stderr,
                )?);
            }
            if !output.success {
                return Err(ForgeError::new(
                    ErrorCode::FixtureFailed,
                    format!("fixture script exited with {:?}", output.exit_code),
                )
                .with_context_string("stdout", stdout.chars().take(4000).collect::<String>())
                .with_context_string("stderr", stderr.chars().take(4000).collect::<String>())
                .with_artifacts(artifacts));
            }
        }
        FixtureAction::BackupAppData { package, path } => {
            let package = package_or_config(package, config)?;
            let path = resolve_local(path, &vars, &config.paths.project_root)?;
            let transfer = AppFiles::new(app.adb.clone())
                .backup_data(serial, &package, &path)
                .await?;
            details = serde_json::to_value(&transfer).unwrap_or_default();
        }
        FixtureAction::RestoreAppData { package, path } => {
            ensure_confirmed(options, "restore-app-data")?;
            let package = package_or_config(package, config)?;
            let path = resolve_local(path, &vars, &config.paths.project_root)?;
            let transfer = AppFiles::new(app.adb.clone())
                .restore_data(serial, &package, &path)
                .await?;
            details = serde_json::to_value(&transfer).unwrap_or_default();
        }
    }
    let _ = paths;
    Ok(StepExecution { artifacts, details })
}

#[allow(clippy::too_many_arguments)]
async fn run_cleanup_steps(
    config: &ForgeConfig,
    serial: &str,
    app: &AppManager,
    ui: &UiManager,
    paths: &ForgePaths,
    run: &mut RunContext,
    options: &FixtureRunOptions,
    env: &BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) {
    let Ok(manifest) = load_fixture(config, &options.name) else {
        return;
    };
    for (index, step) in manifest
        .steps
        .iter()
        .enumerate()
        .filter(|(_, step)| step.phase == Some(FixturePhase::Cleanup))
    {
        let name = step
            .name
            .clone()
            .unwrap_or_else(|| format!("cleanup step {}", index + 1));
        if let Err(error) = execute_step(
            config, serial, app, ui, paths, run, options, env, step, index,
        )
        .await
        {
            warnings.push(format!("cleanup step '{name}' failed: {}", error.message()));
        }
    }
}

fn fixture_environment(
    config: &ForgeConfig,
    session: &Session,
    run: &RunContext,
    options: &FixtureRunOptions,
    phase: &str,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("ANDROIDFORGE_SESSION".to_string(), session.id.clone());
    env.insert(
        "ANDROIDFORGE_SERIAL".to_string(),
        session.serial.clone().unwrap_or_default(),
    );
    env.insert(
        "ANDROIDFORGE_PACKAGE".to_string(),
        config.package().unwrap_or_default().to_string(),
    );
    env.insert(
        "ANDROIDFORGE_APK".to_string(),
        options
            .apk
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| std::env::var("ANDROIDFORGE_APK").unwrap_or_default()),
    );
    env.insert(
        "ANDROIDFORGE_ARTIFACTS".to_string(),
        run.dir.to_string_lossy().into_owned(),
    );
    env.insert("ANDROIDFORGE_PHASE".to_string(), phase.to_string());
    env.insert(
        "ANDROIDFORGE_ADB".to_string(),
        config
            .file
            .sdk
            .adb
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    env.insert(
        "ANDROIDFORGE_ROOT".to_string(),
        config.paths.project_root.to_string_lossy().into_owned(),
    );
    env.insert("ANDROIDFORGE_RUN_ID".to_string(), run.run_id.clone());
    env.insert("ANDROIDFORGE_PROJECT".to_string(), config.project_name());
    if let Ok(exe) = std::env::current_exe() {
        env.insert(
            "ANDROIDFORGE_EXE".to_string(),
            exe.to_string_lossy().into_owned(),
        );
    }
    env.extend(options.vars.clone());
    env
}

fn resolve_template(
    input: &str,
    vars: &BTreeMap<String, String>,
    project_root: &Path,
) -> Result<String> {
    let mut value = input.to_string();
    let project_root = project_root.to_string_lossy();
    let env_regex = regex::Regex::new(r"\$\{ENV:([A-Za-z_][A-Za-z0-9_]*)\}").expect("env regex");
    // Variables may themselves contain ${PROJECT_ROOT} or ${ENV:NAME}, so expand
    // to a fixpoint instead of relying on declaration order.
    for _ in 0..8 {
        let before = value.clone();
        value = value.replace("${PROJECT_ROOT}", &project_root);
        value = expand_env_templates(&value, &env_regex)?;
        value = expand_vars_once(&value, vars, input)?;
        if value == before {
            break;
        }
    }
    if value.contains("${") {
        return Err(ForgeError::new(
            ErrorCode::FixtureInvalid,
            format!("fixture value contains an unresolved template: '{input}'"),
        )
        .with_suggestion(
            "Define the variable in fixture vars, --var, or the AndroidForge environment contract.",
        )
        .with_suggestion("Use ${ENV:NAME} only for explicitly supplied secrets."));
    }
    Ok(value)
}

fn expand_env_templates(value: &str, env_regex: &regex::Regex) -> Result<String> {
    let mut output = String::new();
    let mut last = 0;
    for captures in env_regex.captures_iter(value) {
        let whole = captures.get(0).unwrap();
        output.push_str(&value[last..whole.start()]);
        let name = captures.get(1).unwrap().as_str();
        let secret = std::env::var(name).map_err(|_| {
            ForgeError::new(
                ErrorCode::FixtureInvalid,
                format!("fixture references missing environment variable {name}"),
            )
            .with_suggestion(
                "Set the variable before running the fixture; it is never written to artifacts.",
            )
        })?;
        output.push_str(&secret);
        last = whole.end();
    }
    output.push_str(&value[last..]);
    Ok(output)
}

fn expand_vars_once(
    value: &str,
    vars: &BTreeMap<String, String>,
    original: &str,
) -> Result<String> {
    let mut result = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        result.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            return Err(ForgeError::new(
                ErrorCode::FixtureInvalid,
                format!("unclosed template in fixture value '{original}'"),
            ));
        };
        let name = &rest[start + 2..start + end];
        if let Some(replacement) = vars.get(name) {
            result.push_str(replacement);
        } else {
            result.push_str(&rest[start..start + end + 1]);
        }
        rest = &rest[start + end + 1..];
    }
    result.push_str(rest);
    Ok(result)
}

fn resolve_opt(
    input: &Option<String>,
    vars: &BTreeMap<String, String>,
    project_root: &Path,
) -> Result<Option<String>> {
    input
        .as_ref()
        .map(|value| resolve_template(value, vars, project_root))
        .transpose()
}

fn resolve_local(
    input: &str,
    vars: &BTreeMap<String, String>,
    project_root: &Path,
) -> Result<PathBuf> {
    let value = resolve_template(input, vars, project_root)?;
    let path = PathBuf::from(value);
    Ok(if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    })
}

fn package_or_config(package: &Option<String>, config: &ForgeConfig) -> Result<String> {
    package
        .clone()
        .or_else(|| config.debug_package().map(str::to_string))
        .ok_or_else(|| {
            ForgeError::new(
                ErrorCode::FixtureInvalid,
                "fixture step needs a package and the project adapter has none",
            )
            .with_suggestion("Set [project].package or provide package in the fixture step.")
        })
}

fn ensure_confirmed(options: &FixtureRunOptions, operation: &str) -> Result<()> {
    if options.confirmed {
        Ok(())
    } else {
        Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            format!("fixture operation {operation} is destructive and was not confirmed"),
        )
        .with_suggestion("Re-run with --yes after reviewing the fixture.")
        .with_suggestion("Back up app data first where the application is debuggable."))
    }
}

fn describe_action(action: &FixtureAction) -> String {
    match action {
        FixtureAction::Adb { args } => format!("adb {}", args.join(" ")),
        FixtureAction::Shell { command } => {
            format!("shell {}", command.chars().take(300).collect::<String>())
        }
        FixtureAction::Install { apk, fresh } => format!(
            "install {}{}",
            apk.as_deref().unwrap_or("${ANDROIDFORGE_APK}"),
            if *fresh { " --fresh" } else { "" }
        ),
        FixtureAction::Uninstall { package, .. } => format!(
            "uninstall {}",
            package.as_deref().unwrap_or("<project package>")
        ),
        FixtureAction::ClearData { package } => format!(
            "clear-data {}",
            package.as_deref().unwrap_or("<project package>")
        ),
        FixtureAction::Launch { package, .. } => format!(
            "launch {}",
            package.as_deref().unwrap_or("<project package>")
        ),
        FixtureAction::ForceStop { package } => format!(
            "force-stop {}",
            package.as_deref().unwrap_or("<project package>")
        ),
        FixtureAction::Grant { permission, .. } => format!("grant {permission}"),
        FixtureAction::Revoke { permission, .. } => format!("revoke {permission}"),
        FixtureAction::AppOp {
            operation, mode, ..
        } => format!("appop {operation}={mode}"),
        FixtureAction::Push { local, remote } | FixtureAction::Pull { remote, local } => {
            format!("push {local} -> {remote}")
        }
        FixtureAction::AppFilePush { remote, .. } => format!("app-file-push {remote}"),
        FixtureAction::AppFilePull { remote, .. } => format!("app-file-pull {remote}"),
        FixtureAction::Settings { namespace, key, .. } => format!("settings {namespace}:{key}"),
        FixtureAction::Wait { ms } => format!("wait {ms}ms"),
        FixtureAction::Assert { text, .. } => {
            format!("assert {}", text.as_deref().unwrap_or("<selector>"))
        }
        FixtureAction::Script { command, .. } => format!("script {}", command.join(" ")),
        FixtureAction::BackupAppData { path, .. } => format!("backup-app-data {path}"),
        FixtureAction::RestoreAppData { path, .. } => format!("restore-app-data {path}"),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[derive(Debug)]
struct StepExecution {
    artifacts: Vec<ArtifactRef>,
    details: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_and_toml_fixture_text() {
        let toml_manifest =
            parse_fixture_text(Path::new("fixture.toml"), "version = 1\nname = \"toml\"\n")
                .unwrap();
        assert_eq!(toml_manifest.name, "toml");
        let json_manifest =
            parse_fixture_text(Path::new("fixture.json"), r#"{"version":1,"name":"json"}"#)
                .unwrap();
        assert_eq!(json_manifest.name, "json");
        assert!(parse_fixture_text(Path::new("fixture.json"), "not json").is_err());
    }

    #[test]
    fn resolves_project_and_variable_templates() {
        let mut vars = BTreeMap::new();
        vars.insert("NAME".to_string(), "value".to_string());
        let result =
            resolve_template("${PROJECT_ROOT}/x/${NAME}", &vars, Path::new("C:/p")).unwrap();
        assert!(result.ends_with("x/value"));
    }

    #[test]
    fn resolves_project_root_inside_a_fixture_variable() {
        let mut vars = BTreeMap::new();
        vars.insert(
            "WATCH_STATE".to_string(),
            "${PROJECT_ROOT}/fixtures/watch-state.json".to_string(),
        );
        let result = resolve_template("${WATCH_STATE}", &vars, Path::new("C:/p")).unwrap();
        assert_eq!(result, "C:/p/fixtures/watch-state.json");
    }

    #[test]
    fn rejects_unresolved_templates() {
        let result = resolve_template("${MISSING}", &BTreeMap::new(), Path::new("C:/p"));
        assert!(result.is_err());
    }

    #[test]
    fn destructive_actions_are_marked() {
        assert!(FixtureAction::ClearData { package: None }.is_destructive());
        assert!(
            FixtureAction::Install {
                apk: None,
                fresh: true
            }
            .is_destructive()
        );
    }
}
