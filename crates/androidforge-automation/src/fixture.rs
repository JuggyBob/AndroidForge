//! Fixture packs: imperative steps plus optional declarative app state.
//!
//! ## Declarative state (debug-only)
//!
//! `state` is applied and reset by the `apply_state` / `reset_state` actions.
//! Every path is relative to the app's private data directory and is bounded
//! (no absolute paths, `..`, control characters or backslashes). State is only
//! available for debuggable applications, through the same `run-as` primitives
//! used by `app data seed`, `app data export` and `app data import`.
//!
//! - `prefs` is a UTF-8 key/value file: one `key=value` per line, `#` comments,
//!   keys limited to `[A-Za-z0-9_.-]+`, values without newlines or control
//!   characters, emitted in sorted key order. It is a fixture-owned file, not
//!   an Android `SharedPreferences` XML file.
//! - `history` is opaque: a user-supplied argv command (never a shell string)
//!   receives a bounded count and writes a payload file. AndroidForge does not
//!   parse DataStore protobuf; the script is the writer contract.
//! - `installed_version` either asserts the package-manager version or stamps
//!   a fixture-owned version file. Both modes refuse non-debuggable apps with
//!   `E_APP_NOT_DEBUGGABLE`.
//!
//! Mutation requires explicit consent at the execution boundary via
//! `run_fixture_with_data_policy(..., allow_data_mutation)`; core never infers
//! consent from the device kind.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
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

/// Upper bound for `state.history.count` so a declared fixture cannot request
/// an unbounded amount of generated state.
pub const MAX_FIXTURE_HISTORY_COUNT: u32 = 1000;
/// Maximum number of path components in a declared app-private state path.
pub const MAX_FIXTURE_STATE_PATH_COMPONENTS: usize = 8;
/// Maximum length of a declared app-private state path.
pub const MAX_FIXTURE_STATE_PATH_LEN: usize = 240;
/// Default app-private path for the generated prefs key/value file.
pub const DEFAULT_FIXTURE_PREFS_PATH: &str = "files/androidforge/fixture-prefs.txt";
/// Default app-private path for the generated installed-version stamp.
pub const DEFAULT_FIXTURE_VERSION_PATH: &str = "files/androidforge/installed-version.txt";

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<FixtureState>,
    #[serde(default)]
    pub steps: Vec<FixtureStep>,
    #[serde(skip)]
    pub source_path: Option<String>,
}

fn default_version() -> u32 {
    FIXTURE_VERSION
}

fn default_fixture_prefs_path() -> String {
    DEFAULT_FIXTURE_PREFS_PATH.to_string()
}

fn default_fixture_version_path() -> String {
    DEFAULT_FIXTURE_VERSION_PATH.to_string()
}

/// Declarative, debug-only app state. Applied by `apply_state` and removed by
/// `reset_state`; every part is optional and independently validated.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefs: Option<FixturePrefsState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<FixtureHistoryState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<FixtureInstalledVersionState>,
}

/// Simple UTF-8 key/value file written through the debug-only `run-as` path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixturePrefsState {
    #[serde(default = "default_fixture_prefs_path")]
    pub path: String,
    pub values: BTreeMap<String, String>,
}

/// Bounded, opaque history seeding through a user-supplied argv command.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureHistoryState {
    #[serde(default)]
    pub count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureInstalledVersionMode {
    Assert,
    Set,
}

impl FixtureInstalledVersionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assert => "assert",
            Self::Set => "set",
        }
    }
}

/// Assert a version against the package manager, or stamp it into app-private
/// state. Both modes require a debuggable application.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureInstalledVersionState {
    pub mode: FixtureInstalledVersionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<i64>,
    #[serde(default = "default_fixture_version_path")]
    pub path: String,
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
    /// Apply the manifest's declarative `state` (debug-only, requires explicit
    /// data-mutation consent at the execution boundary).
    ApplyState {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
    },
    /// Remove the files created by the manifest's declarative `state`
    /// (debug-only, requires explicit data-mutation consent).
    ResetState {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
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
            Self::ApplyState { .. } => "apply_state",
            Self::ResetState { .. } => "reset_state",
        }
    }

    pub fn is_destructive(&self) -> bool {
        match self {
            Self::Uninstall { .. }
            | Self::ClearData { .. }
            | Self::RestoreAppData { .. }
            | Self::ResetState { .. } => true,
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
    if let Some(state) = &manifest.state {
        validate_fixture_state_report(state, "state", errors);
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
            FixtureAction::ApplyState { .. } | FixtureAction::ResetState { .. } => {
                if manifest.state.is_none() {
                    errors.push(format!(
                        "{label}: {} needs a declarative state block",
                        step.action.kind()
                    ));
                }
                if config.debug_package().is_none() {
                    warnings.push(format!(
                        "{label}: {} has no package; provide package in the step or set [project].package",
                        step.action.kind()
                    ));
                }
                if step.action.is_destructive() && !manifest.requires_confirmation {
                    warnings.push(format!(
                        "{label}: destructive step is present; fixture run will require --yes"
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

/// Validate declarative state without touching a device.
pub fn validate_fixture_state_report(state: &FixtureState, prefix: &str, errors: &mut Vec<String>) {
    if let Some(prefs) = &state.prefs {
        if prefs.values.is_empty() {
            errors.push(format!("{prefix}.prefs.values cannot be empty"));
        }
        if let Err(error) = validate_state_path(&prefs.path) {
            errors.push(format!("{prefix}.prefs.path: {error}"));
        }
        for (key, value) in &prefs.values {
            if !valid_pref_key(key) {
                errors.push(format!(
                    "{prefix}.prefs key '{key}' must match [A-Za-z0-9_.-]+"
                ));
            }
            if value.chars().any(char::is_control) || value.contains('\n') || value.contains('\r') {
                errors.push(format!(
                    "{prefix}.prefs value for '{key}' contains a control character"
                ));
            }
        }
    }
    if let Some(history) = &state.history {
        if history.count > MAX_FIXTURE_HISTORY_COUNT {
            errors.push(format!(
                "{prefix}.history.count must be {MAX_FIXTURE_HISTORY_COUNT} or less"
            ));
        }
        validate_command_argv(
            &history.command,
            &format!("{prefix}.history.command"),
            errors,
        );
        if let Some(file) = &history.file
            && let Err(error) = validate_state_path(file)
        {
            errors.push(format!("{prefix}.history.file: {error}"));
        }
        for (key, value) in &history.env {
            if !valid_env_name(key) {
                errors.push(format!("{prefix}.history.env has an invalid name '{key}'"));
            }
            if value.chars().any(char::is_control) {
                errors.push(format!(
                    "{prefix}.history.env value for '{key}' contains a control character"
                ));
            }
        }
    }
    if let Some(version) = &state.installed_version {
        if version.version_name.is_none() && version.version_code.is_none() {
            errors.push(format!(
                "{prefix}.installed_version needs version_name or version_code"
            ));
        }
        if let Some(name) = &version.version_name
            && (name.trim().is_empty() || name.chars().any(char::is_control))
        {
            errors.push(format!(
                "{prefix}.installed_version.version_name is invalid"
            ));
        }
        if version.mode == FixtureInstalledVersionMode::Set
            && let Err(error) = validate_state_path(&version.path)
        {
            errors.push(format!("{prefix}.installed_version.path: {error}"));
        }
    }
}

/// App-private state paths are relative, shallow, bounded and shell-safe.
pub fn validate_state_path(path: &str) -> std::result::Result<String, String> {
    if path.is_empty() {
        return Err("path cannot be empty".to_string());
    }
    if path.len() > MAX_FIXTURE_STATE_PATH_LEN {
        return Err(format!("path exceeds {MAX_FIXTURE_STATE_PATH_LEN} bytes"));
    }
    if path.chars().any(char::is_control) || path.contains('\\') {
        return Err("path contains a control character or backslash".to_string());
    }
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return Err("path must be relative to the app data directory".to_string());
    }
    let mut clean = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("path may not contain '..' or an absolute prefix".to_string());
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err("path cannot be empty".to_string());
    }
    if clean.components().count() > MAX_FIXTURE_STATE_PATH_COMPONENTS {
        return Err(format!(
            "path has more than {MAX_FIXTURE_STATE_PATH_COMPONENTS} components"
        ));
    }
    // App-private paths always use forward slashes, even on Windows hosts.
    Ok(clean.to_string_lossy().replace('\\', "/"))
}

/// A script command is an argv array; shell strings are never accepted.
pub fn validate_command_argv(command: &[String], prefix: &str, errors: &mut Vec<String>) {
    if command.is_empty() {
        errors.push(format!("{prefix} cannot be empty"));
        return;
    }
    for (index, arg) in command.iter().enumerate() {
        if arg.is_empty() {
            errors.push(format!("{prefix}[{index}] cannot be empty"));
        }
        if arg.chars().any(char::is_control) {
            errors.push(format!("{prefix}[{index}] contains a control character"));
        }
    }
}

fn valid_pref_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Render the documented prefs key/value format deterministically.
pub fn serialize_fixture_prefs(values: &BTreeMap<String, String>) -> String {
    let mut text = String::from("# AndroidForge fixture prefs (debug-only, key=value)\n");
    for (key, value) in values {
        text.push_str(key);
        text.push('=');
        text.push_str(value);
        text.push('\n');
    }
    text
}

/// A resolved, validated declarative state. No device calls are made while
/// planning, so this shape is directly testable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureStatePlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefs: Option<PlannedFixturePrefs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<PlannedFixtureHistory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<PlannedFixtureInstalledVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedFixturePrefs {
    pub path: String,
    pub contents: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedFixtureHistory {
    pub count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedFixtureInstalledVersion {
    pub mode: FixtureInstalledVersionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

fn resolve_state_path(
    value: &str,
    vars: &BTreeMap<String, String>,
    project_root: &Path,
) -> Result<String> {
    let resolved = resolve_template(value, vars, project_root)?;
    validate_state_path(&resolved).map_err(ForgeError::invalid_argument)
}

/// Resolve templates and validate bounds before any device mutation.
pub fn plan_fixture_state(
    state: &FixtureState,
    vars: &BTreeMap<String, String>,
    project_root: &Path,
) -> Result<FixtureStatePlan> {
    let mut errors = Vec::new();
    validate_fixture_state_report(state, "state", &mut errors);
    if !errors.is_empty() {
        return Err(
            ForgeError::new(ErrorCode::FixtureInvalid, "fixture state is invalid")
                .with_context("errors", serde_json::to_value(&errors).unwrap_or_default()),
        );
    }
    let prefs = state
        .prefs
        .as_ref()
        .map(|prefs| -> Result<PlannedFixturePrefs> {
            Ok(PlannedFixturePrefs {
                path: resolve_state_path(&prefs.path, vars, project_root)?,
                contents: serialize_fixture_prefs(&prefs.values),
            })
        })
        .transpose()?;
    let history = state
        .history
        .as_ref()
        .map(|history| -> Result<PlannedFixtureHistory> {
            let command = history
                .command
                .iter()
                .map(|arg| resolve_template(arg, vars, project_root))
                .collect::<Result<Vec<_>>>()?;
            let mut argv_errors = Vec::new();
            validate_command_argv(&command, "state.history.command", &mut argv_errors);
            if !argv_errors.is_empty() {
                return Err(ForgeError::invalid_argument(argv_errors.join("; ")));
            }
            let env = history
                .env
                .iter()
                .map(|(key, value)| Ok((key.clone(), resolve_template(value, vars, project_root)?)))
                .collect::<Result<BTreeMap<_, _>>>()?;
            Ok(PlannedFixtureHistory {
                count: history.count,
                file: history
                    .file
                    .as_ref()
                    .map(|file| resolve_state_path(file, vars, project_root))
                    .transpose()?,
                command,
                env,
            })
        })
        .transpose()?;
    let installed_version = state
        .installed_version
        .as_ref()
        .map(|version| -> Result<PlannedFixtureInstalledVersion> {
            let path = match version.mode {
                FixtureInstalledVersionMode::Set => {
                    Some(resolve_state_path(&version.path, vars, project_root)?)
                }
                FixtureInstalledVersionMode::Assert => None,
            };
            Ok(PlannedFixtureInstalledVersion {
                mode: version.mode,
                version_name: version
                    .version_name
                    .as_ref()
                    .map(|name| resolve_template(name, vars, project_root))
                    .transpose()?,
                version_code: version.version_code,
                path,
            })
        })
        .transpose()?;
    Ok(FixtureStatePlan {
        prefs,
        history,
        installed_version,
    })
}

/// Exact `adb shell` argv used to remove files created by `apply_state`.
/// Paths are deduplicated and re-validated even though planning already did so.
pub fn fixture_state_reset_commands(plan: &FixtureStatePlan, package: &str) -> Vec<Vec<String>> {
    let mut paths = BTreeSet::new();
    if let Some(prefs) = &plan.prefs {
        paths.insert(prefs.path.clone());
    }
    if let Some(history) = &plan.history
        && let Some(file) = &history.file
    {
        paths.insert(file.clone());
    }
    if let Some(version) = &plan.installed_version
        && version.mode == FixtureInstalledVersionMode::Set
        && let Some(path) = &version.path
    {
        paths.insert(path.clone());
    }
    paths
        .into_iter()
        .filter(|path| validate_state_path(path).is_ok())
        .map(|path| {
            vec![
                "run-as".to_string(),
                package.to_string(),
                "rm".to_string(),
                "-f".to_string(),
                path,
            ]
        })
        .collect()
}

/// Consent is never inferred by core. The CLI must pass `true` only for an
/// emulator or after its physical-data risk check.
pub fn require_fixture_data_mutation(allow_data_mutation: bool) -> Result<()> {
    if allow_data_mutation {
        Ok(())
    } else {
        Err(ForgeError::new(
            ErrorCode::PhysicalDeviceMutationBlocked,
            "fixture declarative state mutates app-private data without explicit consent",
        )
        .with_suggestion(
            "The CLI must opt in with allow_data_mutation=true after its physical-data risk check.",
        )
        .with_suggestion(
            "On physical devices use --allow-physical-data or --allow-physical-mutation.",
        ))
    }
}

pub fn require_fixture_debuggable(debuggable: bool, package: &str, serial: &str) -> Result<()> {
    if debuggable {
        Ok(())
    } else {
        Err(ForgeError::new(
            ErrorCode::AppNotDebuggable,
            format!("fixture state requires a debuggable application; {package} is not debuggable"),
        )
        .with_suggestion("Install a debuggable build before applying fixture state.")
        .with_suggestion("AndroidForge never roots a device to reach app-private data.")
        .device(serial.to_string()))
    }
}

/// Everything a device-backed state operation needs, supplied by the caller.
#[derive(Debug, Clone, Copy)]
pub struct FixtureStateExecution<'a> {
    pub app: &'a AppManager,
    pub serial: &'a str,
    pub package: &'a str,
    pub cwd: &'a Path,
    pub timeout: Duration,
    pub trace: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureStateOutcome {
    pub action: String,
    pub debuggable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

async fn state_debuggable(execution: &FixtureStateExecution<'_>) -> Result<bool> {
    let files = AppFiles::new(execution.app.adb.clone());
    let debuggable = files
        .is_debuggable(execution.serial, execution.package)
        .await?;
    require_fixture_debuggable(debuggable, execution.package, execution.serial)?;
    Ok(debuggable)
}

/// Apply declarative state. Requires explicit data-mutation consent and a
/// debuggable application; never parses DataStore protobuf itself.
pub async fn apply_fixture_state(
    execution: &FixtureStateExecution<'_>,
    plan: &FixtureStatePlan,
    allow_data_mutation: bool,
    run: &mut RunContext,
    index: usize,
) -> Result<FixtureStateOutcome> {
    require_fixture_data_mutation(allow_data_mutation)?;
    let debuggable = state_debuggable(execution).await?;
    let files = AppFiles::new(execution.app.adb.clone());
    let mut paths = Vec::new();
    let mut notes = Vec::new();

    if let Some(prefs) = &plan.prefs {
        let local = run.write_bytes(
            &format!("fixtures/state/{:03}-prefs.txt", index + 1),
            prefs.contents.as_bytes(),
        )?;
        let transfer = files
            .push(
                execution.serial,
                execution.package,
                &local,
                &prefs.path,
                false,
            )
            .await?;
        notes.push(format!(
            "prefs hash verified: {}",
            transfer.remote_hash_verified
        ));
        paths.push(prefs.path.clone());
    }

    if let Some(history) = &plan.history {
        let relative = format!("fixtures/state/{:03}-history.bin", index + 1);
        let output = run.dir.join(&relative);
        if let Some(parent) = output.parent() {
            ensure_dir(parent)?;
        }
        let mut argv = history.command.clone();
        let program = argv.remove(0);
        let mut spec = CommandSpec::new(program)
            .args(argv)
            .cwd(execution.cwd.to_path_buf())
            .timeout(execution.timeout)
            .trace(execution.trace)
            .hide_window(true)
            .env("ANDROIDFORGE_FIXTURE_STATE_MODE", "apply")
            .env(
                "ANDROIDFORGE_FIXTURE_HISTORY_COUNT",
                history.count.to_string(),
            )
            .env(
                "ANDROIDFORGE_FIXTURE_HISTORY_OUTPUT",
                output.to_string_lossy(),
            )
            .env("ANDROIDFORGE_SERIAL", execution.serial)
            .env("ANDROIDFORGE_PACKAGE", execution.package)
            .env("ANDROIDFORGE_ADB", execution.app.adb.adb.to_string_lossy());
        if let Some(file) = &history.file {
            spec = spec.env("ANDROIDFORGE_FIXTURE_HISTORY_FILE", file);
        }
        for (key, value) in &history.env {
            spec = spec.env(key, value);
        }
        let result = androidforge_process::Runner::new(execution.trace)
            .run(spec)
            .await;
        let stdout = result
            .as_ref()
            .map(|output| output.stdout_text())
            .unwrap_or_default();
        let stderr = result
            .as_ref()
            .map(|output| output.stderr_text())
            .unwrap_or_default();
        run.write_text_artifact(
            &format!("logs/fixture-{:03}-history.stdout.txt", index + 1),
            "command_output",
            Some("Fixture history writer stdout"),
            &stdout,
        )?;
        if !stderr.is_empty() {
            run.write_text_artifact(
                &format!("logs/fixture-{:03}-history.stderr.txt", index + 1),
                "command_output",
                Some("Fixture history writer stderr"),
                &stderr,
            )?;
        }
        let result = result.map_err(|error| {
            ForgeError::new(ErrorCode::FixtureFailed, error.to_string())
                .with_context_string("stdout", stdout.chars().take(4000).collect::<String>())
                .with_context_string("stderr", stderr.chars().take(4000).collect::<String>())
        })?;
        if !result.success {
            return Err(ForgeError::new(
                ErrorCode::FixtureFailed,
                format!("fixture history writer exited with {:?}", result.exit_code),
            )
            .with_context_string("stdout", stdout.chars().take(4000).collect::<String>())
            .with_context_string("stderr", stderr.chars().take(4000).collect::<String>()));
        }
        notes.push(format!("history writer seeded count {}", history.count));
        if let Some(file) = &history.file {
            if !output.is_file() {
                return Err(ForgeError::new(
                    ErrorCode::FileNotFound,
                    "history writer did not produce its output file",
                )
                .with_context_string("expected", output.to_string_lossy().into_owned()));
            }
            files
                .push(execution.serial, execution.package, &output, file, false)
                .await?;
            paths.push(file.clone());
        }
    }

    if let Some(version) = &plan.installed_version {
        match version.mode {
            FixtureInstalledVersionMode::Assert => {
                let states = execution
                    .app
                    .status(execution.serial, execution.package)
                    .await?;
                require_fixture_debuggable(states.debuggable, execution.package, execution.serial)?;
                if !states.installed {
                    return Err(ForgeError::new(
                        ErrorCode::PackageNotFound,
                        format!("package {} is not installed", execution.package),
                    )
                    .device(execution.serial.to_string()));
                }
                if let Some(expected) = &version.version_name
                    && states.version_name.as_deref() != Some(expected.as_str())
                {
                    return Err(ForgeError::new(
                        ErrorCode::FixtureFailed,
                        "installed version_name does not match the fixture",
                    )
                    .with_context_string("expected", expected.clone())
                    .with_context_string(
                        "actual",
                        states
                            .version_name
                            .clone()
                            .unwrap_or_else(|| "null".to_string()),
                    ));
                }
                if let Some(expected) = version.version_code
                    && states.version_code != Some(expected)
                {
                    return Err(ForgeError::new(
                        ErrorCode::FixtureFailed,
                        "installed version_code does not match the fixture",
                    )
                    .with_context_string("expected", expected.to_string())
                    .with_context_string(
                        "actual",
                        states
                            .version_code
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "null".to_string()),
                    ));
                }
                notes.push(format!(
                    "asserted name={:?} code={:?}",
                    states.version_name, states.version_code
                ));
            }
            FixtureInstalledVersionMode::Set => {
                let path = version
                    .path
                    .as_deref()
                    .unwrap_or(DEFAULT_FIXTURE_VERSION_PATH);
                let mut stamp = String::new();
                if let Some(name) = &version.version_name {
                    stamp.push_str("version_name=");
                    stamp.push_str(name);
                    stamp.push('\n');
                }
                if let Some(code) = version.version_code {
                    stamp.push_str("version_code=");
                    stamp.push_str(&code.to_string());
                    stamp.push('\n');
                }
                let local = run.write_bytes(
                    &format!("fixtures/state/{:03}-version.txt", index + 1),
                    stamp.as_bytes(),
                )?;
                files
                    .push(execution.serial, execution.package, &local, path, false)
                    .await?;
                paths.push(path.to_string());
                notes.push("installed-version stamp written through run-as".to_string());
            }
        }
    }

    Ok(FixtureStateOutcome {
        action: "apply_state".to_string(),
        debuggable,
        paths,
        notes,
    })
}

/// Remove only the app-private files declared by the fixture state plan.
pub async fn reset_fixture_state(
    execution: &FixtureStateExecution<'_>,
    plan: &FixtureStatePlan,
    allow_data_mutation: bool,
    run: &mut RunContext,
    index: usize,
) -> Result<FixtureStateOutcome> {
    require_fixture_data_mutation(allow_data_mutation)?;
    let debuggable = state_debuggable(execution).await?;
    let mut removed = Vec::new();
    for args in fixture_state_reset_commands(plan, execution.package) {
        let path = args.last().cloned().unwrap_or_default();
        execution
            .app
            .adb
            .shell_args(
                execution.serial,
                &args,
                Some(execution.timeout),
                Some(1024 * 1024),
            )
            .await?;
        removed.push(path);
    }
    run.write_text_artifact(
        &format!("logs/fixture-{:03}-state-reset.json", index + 1),
        "fixture_state_reset",
        Some("App-private fixture state files removed"),
        &serde_json::to_string_pretty(&removed).unwrap_or_default(),
    )?;
    Ok(FixtureStateOutcome {
        action: "reset_state".to_string(),
        debuggable,
        paths: removed.clone(),
        notes: vec![format!("removed {} state file(s)", removed.len())],
    })
}

/// Backward-compatible entry point. Declarative state is refused because core
/// does not infer consent; CLI wiring should call
/// `run_fixture_with_data_policy` after its physical-data check.
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
    run_fixture_with_data_policy(config, session, app, ui, paths, run, options, false).await
}

/// Run a fixture with an explicit data-mutation capability decision. Callers
/// must compute `allow_data_mutation` from the device kind and their scoped
/// physical-risk flags; core never assumes consent.
#[allow(clippy::too_many_arguments)]
pub async fn run_fixture_with_data_policy(
    config: &ForgeConfig,
    session: &Session,
    app: &AppManager,
    ui: &UiManager,
    paths: &ForgePaths,
    run: &mut RunContext,
    options: &FixtureRunOptions,
    allow_data_mutation: bool,
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
                config,
                &serial,
                app,
                ui,
                paths,
                run,
                options,
                &env,
                manifest.state.as_ref(),
                allow_data_mutation,
                step,
                index,
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
                        allow_data_mutation,
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
    state: Option<&FixtureState>,
    allow_data_mutation: bool,
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
        FixtureAction::ApplyState { package } => {
            let package = package_or_config(package, config)?;
            let declared = state.ok_or_else(|| {
                ForgeError::new(
                    ErrorCode::FixtureInvalid,
                    "apply_state needs a declarative state block",
                )
            })?;
            let plan = plan_fixture_state(declared, &vars, &config.paths.project_root)?;
            let execution = FixtureStateExecution {
                app,
                serial,
                package: &package,
                cwd: &config.paths.project_root,
                timeout,
                trace: options.trace,
            };
            let outcome =
                apply_fixture_state(&execution, &plan, allow_data_mutation, run, index).await?;
            details = serde_json::to_value(&outcome).unwrap_or_default();
        }
        FixtureAction::ResetState { package } => {
            ensure_confirmed(options, "reset-state")?;
            let package = package_or_config(package, config)?;
            let declared = state.ok_or_else(|| {
                ForgeError::new(
                    ErrorCode::FixtureInvalid,
                    "reset_state needs a declarative state block",
                )
            })?;
            let plan = plan_fixture_state(declared, &vars, &config.paths.project_root)?;
            let execution = FixtureStateExecution {
                app,
                serial,
                package: &package,
                cwd: &config.paths.project_root,
                timeout,
                trace: options.trace,
            };
            let outcome =
                reset_fixture_state(&execution, &plan, allow_data_mutation, run, index).await?;
            details = serde_json::to_value(&outcome).unwrap_or_default();
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
    allow_data_mutation: bool,
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
            config,
            serial,
            app,
            ui,
            paths,
            run,
            options,
            env,
            manifest.state.as_ref(),
            allow_data_mutation,
            step,
            index,
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
        FixtureAction::ApplyState { package } => format!(
            "apply_state {}",
            package.as_deref().unwrap_or("<project package>")
        ),
        FixtureAction::ResetState { package } => format!(
            "reset_state {}",
            package.as_deref().unwrap_or("<project package>")
        ),
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
        assert!(FixtureAction::ResetState { package: None }.is_destructive());
        assert!(!FixtureAction::ApplyState { package: None }.is_destructive());
        assert_eq!(
            FixtureAction::ApplyState { package: None }.kind(),
            "apply_state"
        );
        assert_eq!(
            FixtureAction::ResetState { package: None }.kind(),
            "reset_state"
        );
    }

    fn state_manifest_json() -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "name": "state-demo",
            "state": {
                "prefs": {
                    "path": "files/androidforge/fixture-prefs.txt",
                    "values": {"onboarding_done": "true", "last_tab": "home"}
                },
                "history": {
                    "count": 12,
                    "file": "files/androidforge/history.bin",
                    "command": ["${PROJECT_ROOT}/fixtures/write-history", "--count", "12"],
                    "env": {"WRITER_MODE": "fixture"}
                },
                "installed_version": {
                    "mode": "assert",
                    "version_name": "1.2.3",
                    "version_code": 12
                }
            },
            "steps": [
                {"name": "apply", "action": {"type": "apply_state"}},
                {
                    "name": "reset",
                    "phase": "cleanup",
                    "action": {"type": "reset_state", "package": "com.example.app"}
                }
            ]
        })
    }

    #[test]
    fn parses_declarative_state_from_json_and_toml() {
        let json = state_manifest_json().to_string();
        let manifest = parse_fixture_text(Path::new("fixture.json"), &json).unwrap();
        let state = manifest.state.as_ref().expect("state");
        assert_eq!(state.prefs.as_ref().unwrap().values.len(), 2);
        assert_eq!(state.history.as_ref().unwrap().count, 12);
        assert_eq!(
            state.installed_version.as_ref().unwrap().mode,
            FixtureInstalledVersionMode::Assert
        );
        assert_eq!(manifest.steps[0].action.kind(), "apply_state");
        assert_eq!(manifest.steps[1].action.kind(), "reset_state");

        let toml = r#"
version = 1
name = "state-toml"
[state.prefs]
path = "files/androidforge/prefs.txt"
[state.prefs.values]
onboarding_done = "true"
[state.history]
count = 3
command = ["fixtures/write-history", "--count", "3"]
[state.installed_version]
mode = "set"
version_code = 7
[[steps]]
name = "apply"
[steps.action]
type = "apply_state"
"#;
        let manifest = parse_fixture_text(Path::new("fixture.toml"), toml).unwrap();
        let state = manifest.state.as_ref().expect("state");
        assert_eq!(state.history.as_ref().unwrap().command.len(), 3);
        assert_eq!(
            state.installed_version.as_ref().unwrap().mode,
            FixtureInstalledVersionMode::Set
        );
        assert_eq!(manifest.steps[0].action.kind(), "apply_state");
    }

    #[test]
    fn rejects_unknown_fields_at_every_level() {
        let mut manifest = state_manifest_json();
        manifest["unexpected"] = serde_json::json!(true);
        assert!(parse_fixture_text(Path::new("fixture.json"), &manifest.to_string()).is_err());

        let mut manifest = state_manifest_json();
        manifest["state"]["prefs"]["unexpected"] = serde_json::json!(true);
        assert!(parse_fixture_text(Path::new("fixture.json"), &manifest.to_string()).is_err());

        let mut manifest = state_manifest_json();
        manifest["state"]["history"]["unexpected"] = serde_json::json!(true);
        assert!(parse_fixture_text(Path::new("fixture.json"), &manifest.to_string()).is_err());

        let mut manifest = state_manifest_json();
        manifest["state"]["installed_version"]["unexpected"] = serde_json::json!(true);
        assert!(parse_fixture_text(Path::new("fixture.json"), &manifest.to_string()).is_err());

        let mut manifest = state_manifest_json();
        manifest["steps"][0]["action"]["unexpected"] = serde_json::json!(true);
        assert!(parse_fixture_text(Path::new("fixture.json"), &manifest.to_string()).is_err());
    }

    #[test]
    fn fixture_schema_accepts_state_and_rejects_unknown_fields() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixture.schema.json");
        let text = std::fs::read_to_string(&path).expect("read fixture schema");
        let schema: serde_json::Value = serde_json::from_str(&text).expect("parse fixture schema");
        jsonschema::meta::validate(&schema).expect("fixture schema is not valid draft 2020-12");
        let validator = jsonschema::validator_for(&schema).expect("compile fixture schema");
        let valid = state_manifest_json();
        assert!(validator.is_valid(&valid), "schema rejected {valid:#}");

        let mut unknown = state_manifest_json();
        unknown["state"]["history"]["unexpected"] = serde_json::json!(1);
        assert!(!validator.is_valid(&unknown));

        let mut bad_mode = state_manifest_json();
        bad_mode["state"]["installed_version"]["mode"] = serde_json::json!("bogus");
        assert!(!validator.is_valid(&bad_mode));

        let mut shell_string = state_manifest_json();
        shell_string["state"]["history"]["command"] = serde_json::json!("python writer.py");
        assert!(!validator.is_valid(&shell_string));

        let mut unbounded = state_manifest_json();
        unbounded["state"]["history"]["count"] = serde_json::json!(1001);
        assert!(!validator.is_valid(&unbounded));

        let mut empty_prefs = state_manifest_json();
        empty_prefs["state"]["prefs"]["values"] = serde_json::json!({});
        assert!(!validator.is_valid(&empty_prefs));

        let mut no_version = state_manifest_json();
        let installed = no_version["state"]["installed_version"]
            .as_object_mut()
            .unwrap();
        installed.remove("version_name");
        installed.remove("version_code");
        assert!(!validator.is_valid(&no_version));
    }

    #[test]
    fn prefs_format_is_sorted_and_documented() {
        let mut values = BTreeMap::new();
        values.insert("z_last".to_string(), "true".to_string());
        values.insert("a_first".to_string(), "one=two".to_string());
        assert_eq!(
            serialize_fixture_prefs(&values),
            "# AndroidForge fixture prefs (debug-only, key=value)\na_first=one=two\nz_last=true\n"
        );

        let mut errors = Vec::new();
        let state = FixtureState {
            prefs: Some(FixturePrefsState {
                path: DEFAULT_FIXTURE_PREFS_PATH.to_string(),
                values: BTreeMap::from([("bad key".to_string(), "value".to_string())]),
            }),
            ..FixtureState::default()
        };
        validate_fixture_state_report(&state, "state", &mut errors);
        assert!(errors.iter().any(|error| error.contains("must match")));
    }

    #[test]
    fn state_paths_are_bounded_and_relative() {
        assert_eq!(
            validate_state_path("files/./state.json").unwrap(),
            "files/state.json"
        );
        for bad in [
            "",
            "/data/data/com.example",
            "files/../../state.json",
            "files\\state.json",
            "files/state\n.json",
            &"a/".repeat(100),
        ] {
            assert!(
                validate_state_path(bad).is_err(),
                "accepted bad path: {bad:?}"
            );
        }
    }

    #[test]
    fn history_command_is_argv_not_a_shell_string() {
        let shell: std::result::Result<FixtureHistoryState, _> =
            serde_json::from_str(r#"{"count":1,"command":"python writer.py"}"#);
        assert!(shell.is_err(), "a shell string must not parse as a command");

        let state = FixtureState {
            history: Some(FixtureHistoryState {
                count: 2,
                file: Some("files/history.bin".to_string()),
                command: vec![
                    "${PROJECT_ROOT}/fixtures/write-history".to_string(),
                    "--count".to_string(),
                    "2".to_string(),
                ],
                env: BTreeMap::new(),
            }),
            ..FixtureState::default()
        };
        let plan = plan_fixture_state(&state, &BTreeMap::new(), Path::new("C:/proj")).unwrap();
        assert_eq!(
            plan.history.as_ref().unwrap().command,
            vec![
                "C:/proj/fixtures/write-history".to_string(),
                "--count".to_string(),
                "2".to_string(),
            ]
        );

        let mut errors = Vec::new();
        validate_command_argv(&[], "command", &mut errors);
        validate_command_argv(&["ok".to_string(), "".to_string()], "command", &mut errors);
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn plans_state_without_a_device() {
        let manifest: FixtureManifest = serde_json::from_value(state_manifest_json()).unwrap();
        let plan = plan_fixture_state(
            manifest.state.as_ref().unwrap(),
            &BTreeMap::new(),
            Path::new("C:/proj"),
        )
        .unwrap();
        assert_eq!(
            plan.prefs.as_ref().unwrap().path,
            "files/androidforge/fixture-prefs.txt"
        );
        assert_eq!(plan.history.as_ref().unwrap().count, 12);
        assert_eq!(
            plan.installed_version.as_ref().unwrap().mode,
            FixtureInstalledVersionMode::Assert
        );
        assert!(plan.installed_version.as_ref().unwrap().path.is_none());
    }

    #[test]
    fn validation_refuses_invalid_bounds_and_missing_version_fields() {
        let state = FixtureState {
            history: Some(FixtureHistoryState {
                count: MAX_FIXTURE_HISTORY_COUNT + 1,
                file: Some("/abs".to_string()),
                command: vec!["writer".to_string()],
                env: BTreeMap::new(),
            }),
            installed_version: Some(FixtureInstalledVersionState {
                mode: FixtureInstalledVersionMode::Set,
                version_name: None,
                version_code: None,
                path: "files/../../version".to_string(),
            }),
            ..FixtureState::default()
        };
        let mut errors = Vec::new();
        validate_fixture_state_report(&state, "state", &mut errors);
        assert!(errors.iter().any(|e| e.contains("count must be")));
        assert!(errors.iter().any(|e| e.contains("history.file")));
        assert!(
            errors
                .iter()
                .any(|e| e.contains("version_name or version_code"))
        );
        assert!(errors.iter().any(|e| e.contains("installed_version.path")));
        assert!(plan_fixture_state(&state, &BTreeMap::new(), Path::new("C:/proj")).is_err());

        assert!(
            serde_json::from_str::<FixtureInstalledVersionState>(
                r#"{"mode":"unknown","version_code":1}"#
            )
            .is_err()
        );
    }

    #[test]
    fn data_mutation_consent_is_explicit() {
        assert!(require_fixture_data_mutation(true).is_ok());
        let error = require_fixture_data_mutation(false).unwrap_err();
        assert_eq!(error.code(), ErrorCode::PhysicalDeviceMutationBlocked);
    }

    #[test]
    fn debug_only_refusal_is_explicit() {
        assert!(require_fixture_debuggable(true, "com.example", "emulator-5554").is_ok());
        let error = require_fixture_debuggable(false, "com.example", "emulator-5554").unwrap_err();
        assert_eq!(error.code(), ErrorCode::AppNotDebuggable);
        assert!(error.message().contains("not debuggable"));
    }

    #[test]
    fn reset_removes_only_declared_paths_deduplicated() {
        let state = FixtureState {
            prefs: Some(FixturePrefsState {
                path: "files/shared.txt".to_string(),
                values: BTreeMap::from([("k".to_string(), "v".to_string())]),
            }),
            history: Some(FixtureHistoryState {
                count: 1,
                file: Some("files/history.bin".to_string()),
                command: vec!["writer".to_string()],
                env: BTreeMap::new(),
            }),
            installed_version: Some(FixtureInstalledVersionState {
                mode: FixtureInstalledVersionMode::Set,
                version_name: Some("1.0".to_string()),
                version_code: None,
                path: "files/shared.txt".to_string(),
            }),
        };
        let plan = plan_fixture_state(&state, &BTreeMap::new(), Path::new("C:/proj")).unwrap();
        let commands = fixture_state_reset_commands(&plan, "com.example");
        assert_eq!(
            commands,
            vec![
                vec![
                    "run-as".to_string(),
                    "com.example".to_string(),
                    "rm".to_string(),
                    "-f".to_string(),
                    "files/history.bin".to_string(),
                ],
                vec![
                    "run-as".to_string(),
                    "com.example".to_string(),
                    "rm".to_string(),
                    "-f".to_string(),
                    "files/shared.txt".to_string(),
                ],
            ]
        );
        for command in &commands {
            assert!(
                !command.iter().any(|arg| arg.contains(' ')),
                "shell string in argv"
            );
        }
    }
}
