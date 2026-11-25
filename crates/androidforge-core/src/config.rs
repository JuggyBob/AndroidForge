use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, ForgeError, Result};
use crate::paths::ForgePaths;
use crate::util::{normalize_fingerprint, version_at_least};

pub const CURRENT_TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ADAPTER_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookConfig {
    #[serde(rename = "type", alias = "kind")]
    pub kind: HookKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extras: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default = "default_true")]
    pub idempotent: bool,
    /// Run a broadcast hook as the app UID through `run-as` (debuggable builds).
    /// This lets an explicit shell broadcast reach a non-exported receiver.
    #[serde(default)]
    pub run_as: bool,
    /// Optional postcondition. A hook that succeeds at the ADB layer but does
    /// not change app state is not a successful app hook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<HookVerification>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HookVerification {
    /// Remote shell command, for example `run-as <pkg> cat files/status.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Local command. Takes precedence over `shell` when both are set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_not_contains: Option<String>,
    /// Require this exact text or content description to be visible after the
    /// hook. This proves the UI route, not just the adb exit code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_text: Option<String>,
    /// Require the foreground activity to equal this value or end with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_activity: Option<String>,
    /// Require the foreground package to equal this value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_package: Option<String>,
    /// Require the verification output to differ from the pre-hook baseline.
    pub must_change: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookKind {
    Broadcast,
    DeepLink,
    Shell,
    Instrumentation,
    Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectSection {
    pub name: Option<String>,
    pub package: Option<String>,
    pub debug_package: Option<String>,
    #[serde(deserialize_with = "string_or_vec")]
    pub expected_cert_sha256: Vec<String>,
    pub default_avd: Option<String>,
    pub default_device: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub artifact_dir: String,
    pub retention_days: u32,
    pub verify_installed_apk: bool,
    pub auto_prune: bool,
}

impl Default for ProjectSection {
    fn default() -> Self {
        Self {
            name: None,
            package: None,
            debug_package: None,
            expected_cert_sha256: Vec::new(),
            default_avd: None,
            default_device: None,
            locale: None,
            timezone: None,
            artifact_dir: ".artifacts".to_string(),
            retention_days: 14,
            verify_installed_apk: true,
            auto_prune: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FixturesSection {
    pub directory: String,
    pub default: Option<String>,
}

impl Default for FixturesSection {
    fn default() -> Self {
        Self {
            directory: ".androidforge/fixtures".to_string(),
            default: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConformanceSection {
    pub flow: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiSection {
    pub backend: String,
    pub default_timeout_secs: u64,
    pub poll_interval_ms: u64,
    pub settle_interval_ms: u64,
    pub text_verify: bool,
    pub tap_clickable_ancestor: bool,
}

impl Default for UiSection {
    fn default() -> Self {
        Self {
            backend: "uiautomator".to_string(),
            default_timeout_secs: 15,
            poll_interval_ms: 300,
            settle_interval_ms: 400,
            text_verify: true,
            tap_clickable_ancestor: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MaestroSection {
    pub enabled: bool,
    pub binary: String,
    pub timeout_secs: u64,
}

impl Default for MaestroSection {
    fn default() -> Self {
        Self {
            enabled: true,
            binary: "maestro".to_string(),
            timeout_secs: 900,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiagnosticsSection {
    pub logcat_tail_lines: usize,
    pub max_log_bytes: usize,
    pub capture_on_failure: bool,
    pub redact: bool,
    pub include_dumps: bool,
    pub record_max_seconds: u64,
    pub logcat_max_seconds: u64,
}

impl Default for DiagnosticsSection {
    fn default() -> Self {
        Self {
            logcat_tail_lines: 2000,
            max_log_bytes: 5 * 1024 * 1024,
            capture_on_failure: true,
            redact: true,
            include_dumps: true,
            record_max_seconds: 180,
            logcat_max_seconds: 900,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentProfile {
    #[default]
    Full,
    Compact,
    Brief,
}

impl AgentProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Compact => "compact",
            Self::Brief => "brief",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSection {
    /// Default output profile for agent-oriented commands.
    pub profile: AgentProfile,
    /// Maximum list items in compact/brief output.
    pub max_items: usize,
    /// Maximum characters for text fields in compact/brief output.
    pub max_text_chars: usize,
}

impl Default for AgentSection {
    fn default() -> Self {
        Self {
            profile: AgentProfile::Full,
            max_items: 40,
            max_text_chars: 240,
        }
    }
}

pub const DEFAULT_OBSERVATION_RETENTION: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageSection {
    /// Number of observation records retained per session. Older records are
    /// evicted first; the record just written is never evicted.
    pub observation_retention: usize,
}

impl Default for StorageSection {
    fn default() -> Self {
        Self {
            observation_retention: DEFAULT_OBSERVATION_RETENTION,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TimeoutsSection {
    pub command_secs: u64,
    pub adb_secs: u64,
    pub install_secs: u64,
    pub boot_secs: u64,
    pub ui_secs: u64,
    pub fixture_secs: u64,
    pub emulator_stop_secs: u64,
    pub app_launch_secs: u64,
    pub screenshot_secs: u64,
}

impl Default for TimeoutsSection {
    fn default() -> Self {
        Self {
            command_secs: 30,
            adb_secs: 30,
            install_secs: 300,
            boot_secs: 300,
            ui_secs: 15,
            fixture_secs: 300,
            emulator_stop_secs: 30,
            app_launch_secs: 60,
            screenshot_secs: 30,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SdkSection {
    pub root: Option<PathBuf>,
    pub adb: Option<PathBuf>,
    pub emulator: Option<PathBuf>,
    pub build_tools: Option<String>,
    pub aapt2: Option<PathBuf>,
    pub apksigner: Option<PathBuf>,
    pub zipalign: Option<PathBuf>,
    pub apkanalyzer: Option<PathBuf>,
    pub java: Option<PathBuf>,
    pub maestro: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HealthSection {
    /// Warn when the battery is below this percentage and not charging.
    pub battery_warn_percent: u32,
    /// Warn when /data has less free space than this.
    pub storage_warn_bytes: u64,
    /// Warn when Android's thermal status code reaches this level.
    pub thermal_warn_code: i32,
    /// Collect health before each matrix target unless explicitly disabled.
    pub matrix_preflight: bool,
}

impl Default for HealthSection {
    fn default() -> Self {
        Self {
            battery_warn_percent: 15,
            storage_warn_bytes: 512 * 1024 * 1024,
            thermal_warn_code: 3,
            matrix_preflight: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RedactionSection {
    pub enabled: bool,
    pub patterns: Vec<String>,
}

impl Default for RedactionSection {
    fn default() -> Self {
        Self {
            enabled: true,
            patterns: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BuildSection {
    /// Variant used when `apk resolve` is called without --variant.
    pub default_variant: Option<String>,
    /// Module used when more than one module output matches.
    pub default_module: Option<String>,
    /// Map of variant name to project-relative explicit paths or glob patterns.
    pub variants: BTreeMap<String, Vec<String>>,
    /// Search conventional Gradle output locations when a variant has no explicit patterns.
    pub conventional_fallback: bool,
    /// Maximum number of files inspected for one resolution request.
    pub max_candidates: usize,
    /// Map of variant name to an explicit argv array. No shell string is
    /// interpreted and no build command is inferred. Missing entries are valid.
    pub commands: BTreeMap<String, Vec<String>>,
    /// Project-relative working directory for build commands. Must not contain
    /// `..`, an absolute path or a drive prefix.
    pub cwd: Option<String>,
    /// Build-specific timeout. Takes precedence over `[timeouts].build_secs`.
    pub timeout_secs: Option<u64>,
    /// Environment variable names passed through to build commands. Names
    /// only: no `=` and no NUL.
    pub env_passthrough: Vec<String>,
}

impl Default for BuildSection {
    fn default() -> Self {
        Self {
            default_variant: None,
            default_module: None,
            variants: BTreeMap::new(),
            conventional_fallback: true,
            max_candidates: 64,
            commands: BTreeMap::new(),
            cwd: None,
            timeout_secs: None,
            env_passthrough: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ForgeConfigFile {
    pub adapter_version: u32,
    pub min_androidforge_version: Option<String>,
    pub project: ProjectSection,
    pub build: BuildSection,
    pub fixtures: FixturesSection,
    pub hooks: BTreeMap<String, HookConfig>,
    pub conformance: ConformanceSection,
    pub ui: UiSection,
    pub maestro: MaestroSection,
    pub diagnostics: DiagnosticsSection,
    pub agent: AgentSection,
    pub storage: StorageSection,
    pub health: HealthSection,
    pub timeouts: TimeoutsSection,
    pub sdk: SdkSection,
    pub redaction: RedactionSection,
}

impl Default for ForgeConfigFile {
    fn default() -> Self {
        Self {
            adapter_version: ADAPTER_VERSION,
            min_androidforge_version: None,
            project: ProjectSection::default(),
            build: BuildSection::default(),
            fixtures: FixturesSection::default(),
            hooks: BTreeMap::new(),
            conformance: ConformanceSection::default(),
            ui: UiSection::default(),
            maestro: MaestroSection::default(),
            diagnostics: DiagnosticsSection::default(),
            agent: AgentSection::default(),
            storage: StorageSection::default(),
            health: HealthSection::default(),
            timeouts: TimeoutsSection::default(),
            sdk: SdkSection::default(),
            redaction: RedactionSection::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ForgeConfig {
    pub paths: ForgePaths,
    pub file: ForgeConfigFile,
    pub artifact_dir: PathBuf,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub issues: Vec<ValidationIssue>,
    pub project_root: String,
    pub project_config: Option<String>,
    pub global_config: Option<String>,
}

impl ForgeConfig {
    pub fn load(paths: &ForgePaths) -> Result<Self> {
        let mut merged = toml::Value::Table(toml::map::Map::new());
        let mut warnings = Vec::new();

        if paths.global_config_path.is_file() {
            let value = parse_toml_file(&paths.global_config_path, ErrorCode::ConfigInvalid)?;
            merge_toml(&mut merged, value);
        }

        if let Some(path) = &paths.project_config_path {
            let value = parse_toml_file(path, ErrorCode::AdapterInvalid)?;
            merge_toml(&mut merged, value);
        } else {
            warnings.push(
                "No .androidforge/project.toml found; using built-in defaults. Run `androidforge init` to create a project adapter."
                    .to_string(),
            );
        }

        let file: ForgeConfigFile = toml::Value::try_into(merged).map_err(|error| {
            ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!("configuration does not match the AndroidForge adapter schema: {error}"),
            )
            .with_suggestion("Run `androidforge config validate` for details.")
        })?;

        if file.adapter_version != ADAPTER_VERSION {
            return Err(ForgeError::new(
                ErrorCode::AdapterInvalid,
                format!(
                    "unsupported adapter_version {}; this build supports adapter_version {}",
                    file.adapter_version, ADAPTER_VERSION
                ),
            )
            .with_suggestion("Update AndroidForge or migrate .androidforge/project.toml."));
        }

        if let Some(required) = &file.min_androidforge_version
            && !version_at_least(CURRENT_TOOL_VERSION, required)
        {
            return Err(ForgeError::new(
                ErrorCode::AdapterInvalid,
                format!(
                    "this project requires AndroidForge {required} or newer; running {CURRENT_TOOL_VERSION}"
                ),
            ));
        }

        let artifact_dir = {
            let configured = Path::new(&file.project.artifact_dir);
            if configured.is_absolute() {
                configured.to_path_buf()
            } else {
                paths.project_root.join(configured)
            }
        };

        Ok(Self {
            paths: paths.clone(),
            file,
            artifact_dir,
            warnings,
        })
    }

    pub fn project_name(&self) -> String {
        self.file
            .project
            .name
            .clone()
            .or_else(|| {
                self.paths
                    .project_root
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "AndroidForge project".to_string())
    }

    pub fn package(&self) -> Option<&str> {
        self.file.project.package.as_deref()
    }

    pub fn debug_package(&self) -> Option<&str> {
        self.file
            .project
            .debug_package
            .as_deref()
            .or(self.package())
    }

    pub fn fixtures_dir(&self) -> PathBuf {
        let configured = Path::new(&self.file.fixtures.directory);
        if configured.is_absolute() {
            configured.to_path_buf()
        } else {
            self.paths.project_root.join(configured)
        }
    }

    pub fn fixture_path(&self, name: &str) -> PathBuf {
        let dir = self.fixtures_dir();
        for extension in ["toml", "json"] {
            let direct = dir.join(format!("{name}.{extension}"));
            if direct.is_file() {
                return direct;
            }
            let nested = dir.join(name).join(format!("fixture.{extension}"));
            if nested.is_file() {
                return nested;
            }
        }
        dir.join(name).join("fixture.toml")
    }

    pub fn conformance_path(&self, override_flow: Option<&Path>) -> Option<PathBuf> {
        if let Some(path) = override_flow {
            return Some(if path.is_absolute() {
                path.to_path_buf()
            } else {
                self.paths.project_root.join(path)
            });
        }
        self.file.conformance.flow.as_ref().map(|flow| {
            let path = Path::new(flow);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                self.paths.project_root.join(path)
            }
        })
    }

    pub fn hook(&self, name: &str) -> Option<&HookConfig> {
        self.file.hooks.get(name)
    }

    pub fn maestro_binary(&self) -> PathBuf {
        if let Some(path) = &self.file.sdk.maestro {
            return path.clone();
        }
        PathBuf::from(&self.file.maestro.binary)
    }

    pub fn validate(&self) -> ValidationReport {
        let mut issues = Vec::new();
        let file = &self.file;

        if file.adapter_version != ADAPTER_VERSION {
            issues.push(issue(
                Severity::Error,
                "adapter_version",
                format!(
                    "adapter_version must be {ADAPTER_VERSION}, found {}",
                    file.adapter_version
                ),
            ));
        }

        if let Some(required) = &file.min_androidforge_version
            && !version_at_least(CURRENT_TOOL_VERSION, required)
        {
            issues.push(issue(
                Severity::Error,
                "min_androidforge_version",
                format!("project requires {required}, running {CURRENT_TOOL_VERSION}"),
            ));
        }

        if let Some(package) = &file.project.package
            && !valid_package_name(package)
        {
            issues.push(issue(
                Severity::Error,
                "project.package",
                format!("'{package}' is not a valid Android package name"),
            ));
        }
        if let Some(package) = &file.project.debug_package
            && !valid_package_name(package)
        {
            issues.push(issue(
                Severity::Error,
                "project.debug_package",
                format!("'{package}' is not a valid Android package name"),
            ));
        }
        if file.project.retention_days == 0 {
            issues.push(issue(
                Severity::Error,
                "project.retention_days",
                "retention_days must be greater than zero".to_string(),
            ));
        }
        for fingerprint in &file.project.expected_cert_sha256 {
            if normalize_fingerprint(fingerprint).is_none() {
                issues.push(issue(
                    Severity::Error,
                    "project.expected_cert_sha256",
                    format!("'{fingerprint}' is not a SHA-256 fingerprint"),
                ));
            }
        }
        for (variant, patterns) in &file.build.variants {
            if patterns.is_empty() {
                issues.push(issue(
                    Severity::Warning,
                    "build.variants",
                    format!("variant '{variant}' has no APK output patterns"),
                ));
            }
            for pattern in patterns {
                if std::path::Path::new(pattern).is_absolute() {
                    issues.push(issue(
                        Severity::Error,
                        "build.variants",
                        format!("variant '{variant}' pattern '{pattern}' must be project-relative"),
                    ));
                }
                if pattern.contains("..") {
                    issues.push(issue(
                        Severity::Error,
                        "build.variants",
                        format!(
                            "variant '{variant}' pattern '{pattern}' must not escape the project root"
                        ),
                    ));
                }
            }
        }
        if file.build.max_candidates == 0 {
            issues.push(issue(
                Severity::Error,
                "build.max_candidates",
                "max_candidates must be greater than zero".to_string(),
            ));
        }
        for (variant, argv) in &file.build.commands {
            let field = format!("build.commands.{variant}");
            if !valid_variant_key(variant) {
                issues.push(issue(
                    Severity::Error,
                    "build.commands",
                    format!("variant key '{variant}' is empty or has invalid characters"),
                ));
            }
            if argv.is_empty() {
                issues.push(issue(
                    Severity::Error,
                    field.clone(),
                    "command array must not be empty".to_string(),
                ));
            }
            if argv.iter().any(String::is_empty) {
                issues.push(issue(
                    Severity::Error,
                    field.clone(),
                    "command array must not contain empty argv elements".to_string(),
                ));
            }
        }
        if let Some(cwd) = &file.build.cwd
            && !valid_relative_build_cwd(cwd)
        {
            issues.push(issue(
                Severity::Error,
                "build.cwd",
                format!("'{cwd}' must be a relative path without '..' or a drive prefix"),
            ));
        }
        if file.build.timeout_secs == Some(0) {
            issues.push(issue(
                Severity::Error,
                "build.timeout_secs",
                "timeout must be greater than zero".to_string(),
            ));
        }
        for name in &file.build.env_passthrough {
            if !valid_env_name(name) {
                issues.push(issue(
                    Severity::Error,
                    "build.env_passthrough",
                    format!("'{name}' is not a valid environment variable name"),
                ));
            }
        }
        if file.health.battery_warn_percent > 100 {
            issues.push(issue(
                Severity::Error,
                "health.battery_warn_percent",
                "battery_warn_percent must be between 0 and 100".to_string(),
            ));
        }
        if !(0..=6).contains(&file.health.thermal_warn_code) {
            issues.push(issue(
                Severity::Error,
                "health.thermal_warn_code",
                "thermal_warn_code must be an Android thermal status between 0 and 6".to_string(),
            ));
        }
        if !matches!(file.ui.backend.as_str(), "uiautomator") {
            issues.push(issue(
                Severity::Error,
                "ui.backend",
                format!(
                    "unsupported UI backend '{}'; only 'uiautomator' is implemented",
                    file.ui.backend
                ),
            ));
        }
        if file.ui.poll_interval_ms == 0 {
            issues.push(issue(
                Severity::Error,
                "ui.poll_interval_ms",
                "poll_interval_ms must be greater than zero".to_string(),
            ));
        }
        if file.ui.settle_interval_ms == 0 {
            issues.push(issue(
                Severity::Error,
                "ui.settle_interval_ms",
                "settle_interval_ms must be greater than zero".to_string(),
            ));
        }
        for (name, value) in [
            ("timeouts.command_secs", file.timeouts.command_secs),
            ("timeouts.adb_secs", file.timeouts.adb_secs),
            ("timeouts.install_secs", file.timeouts.install_secs),
            ("timeouts.boot_secs", file.timeouts.boot_secs),
            ("timeouts.ui_secs", file.timeouts.ui_secs),
            ("timeouts.fixture_secs", file.timeouts.fixture_secs),
            (
                "timeouts.emulator_stop_secs",
                file.timeouts.emulator_stop_secs,
            ),
            ("timeouts.app_launch_secs", file.timeouts.app_launch_secs),
            ("timeouts.screenshot_secs", file.timeouts.screenshot_secs),
        ] {
            if value == 0 {
                issues.push(issue(
                    Severity::Error,
                    name,
                    "timeout must be greater than zero".to_string(),
                ));
            }
        }
        if file.agent.max_items == 0 || file.agent.max_items > 500 {
            issues.push(issue(
                Severity::Error,
                "agent.max_items",
                "agent.max_items must be between 1 and 500".to_string(),
            ));
        }
        if !(16..=10_000).contains(&file.agent.max_text_chars) {
            issues.push(issue(
                Severity::Error,
                "agent.max_text_chars",
                "agent.max_text_chars must be between 16 and 10000".to_string(),
            ));
        }
        if !(2..=1000).contains(&file.storage.observation_retention) {
            issues.push(issue(
                Severity::Error,
                "storage.observation_retention",
                "storage.observation_retention must be between 2 and 1000".to_string(),
            ));
        }
        if file.diagnostics.max_log_bytes < 64 * 1024 {
            issues.push(issue(
                Severity::Warning,
                "diagnostics.max_log_bytes",
                "log capture is capped below 64 KiB and may lose useful evidence".to_string(),
            ));
        }
        if !self.fixtures_dir().is_dir() && !file.hooks.is_empty() {
            issues.push(issue(
                Severity::Warning,
                "fixtures.directory",
                format!(
                    "fixture directory {} does not exist",
                    self.fixtures_dir().display()
                ),
            ));
        }
        if let Some(path) = self.conformance_path(None)
            && !path.is_file()
        {
            issues.push(issue(
                Severity::Warning,
                "conformance.flow",
                format!("conformance flow {} does not exist", path.display()),
            ));
        }

        for (name, hook) in &file.hooks {
            let field = format!("hooks.{name}");
            match hook.kind {
                HookKind::Broadcast => {
                    if hook.action.as_deref().unwrap_or("").is_empty() {
                        issues.push(issue(
                            Severity::Error,
                            format!("{field}.action"),
                            "broadcast hooks require an action".to_string(),
                        ));
                    }
                }
                HookKind::DeepLink => {
                    if hook.uri.as_deref().unwrap_or("").is_empty() {
                        issues.push(issue(
                            Severity::Error,
                            format!("{field}.uri"),
                            "deep-link hooks require a uri".to_string(),
                        ));
                    }
                }
                HookKind::Shell => {
                    if hook.action.as_deref().unwrap_or("").is_empty() {
                        issues.push(issue(
                            Severity::Error,
                            format!("{field}.action"),
                            "shell hooks require an action".to_string(),
                        ));
                    }
                }
                HookKind::Instrumentation => {
                    if hook.component.as_deref().unwrap_or("").is_empty() {
                        issues.push(issue(
                            Severity::Error,
                            format!("{field}.component"),
                            "instrumentation hooks require component = \"package/runner\""
                                .to_string(),
                        ));
                    }
                }
                HookKind::Command => {
                    if hook.command.is_empty() {
                        issues.push(issue(
                            Severity::Error,
                            format!("{field}.command"),
                            "command hooks require a command array".to_string(),
                        ));
                    }
                }
            }
            for (key, value) in &hook.extras {
                let lower = key.to_ascii_lowercase();
                if (lower.contains("secret")
                    || lower.contains("password")
                    || lower.contains("token")
                    || lower.contains("api_key"))
                    && !value.contains("${ENV:")
                {
                    issues.push(issue(
                        Severity::Warning,
                        format!("{field}.extras.{key}"),
                        "this value looks like a secret; use ${ENV:NAME} so it is not stored in configuration or artifacts"
                            .to_string(),
                    ));
                }
            }
        }

        ValidationReport {
            valid: !issues.iter().any(|item| item.severity == Severity::Error),
            issues,
            project_root: self.paths.project_root.to_string_lossy().into_owned(),
            project_config: self
                .paths
                .project_config_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            global_config: if self.paths.global_config_path.is_file() {
                Some(self.paths.global_config_path.to_string_lossy().into_owned())
            } else {
                None
            },
        }
    }
}

fn parse_toml_file(path: &Path, code: ErrorCode) -> Result<toml::Value> {
    let text = fs::read_to_string(path).map_err(|error| {
        ForgeError::new(code, format!("could not read {}: {error}", path.display()))
    })?;
    toml::from_str::<toml::Value>(&text).map_err(|error| {
        ForgeError::new(code, format!("could not parse {}: {error}", path.display()))
            .with_suggestion("Run `androidforge config validate` after fixing the TOML.")
    })
}

fn merge_toml(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, value) in overlay_table {
                if let Some(existing) = base_table.get_mut(&key) {
                    merge_toml(existing, value);
                } else {
                    base_table.insert(key, value);
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

fn issue(
    severity: Severity,
    field: impl Into<String>,
    message: impl Into<String>,
) -> ValidationIssue {
    ValidationIssue {
        severity,
        field: field.into(),
        message: message.into(),
    }
}

fn valid_package_name(value: &str) -> bool {
    if value.is_empty() || !value.contains('.') {
        return false;
    }
    value.split('.').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .next()
                .map(|ch| ch.is_ascii_alphabetic() || ch == '_')
                .unwrap_or(false)
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    })
}

/// Variant names follow the same safe-name rule used elsewhere in AndroidForge:
/// non-empty and only ASCII letters, digits, '.', '_' or '-'. This admits
/// conventional Gradle variants such as `debug`, `release` and `freeDebug`.
fn valid_variant_key(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

/// A build working directory must stay inside the project: no absolute paths,
/// drive prefixes, UNC roots or `..` components.
fn valid_relative_build_cwd(value: &str) -> bool {
    if value.is_empty() || Path::new(value).is_absolute() || value.starts_with(['/', '\\']) {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return false;
    }
    if Path::new(value)
        .components()
        .any(|component| matches!(component, std::path::Component::Prefix(_)))
    {
        return false;
    }
    !value.split(['/', '\\']).any(|part| part == "..")
}

/// Environment variable names are POSIX-style identifiers. Names only: an
/// `=` would turn this into a `KEY=VALUE` assignment and a NUL cannot be passed
/// to a process environment.
fn valid_env_name(value: &str) -> bool {
    if value.is_empty() || value.contains('=') || value.contains('\0') {
        return false;
    }
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn default_true() -> bool {
    true
}

fn string_or_vec<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        One(String),
        Many(Vec<String>),
    }
    let value = Option::<StringOrVec>::deserialize(deserializer)?;
    Ok(match value {
        Some(StringOrVec::One(value)) => vec![value],
        Some(StringOrVec::Many(values)) => values,
        None => Vec::new(),
    })
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn default_project_toml(name: &str, package: Option<&str>, avd: Option<&str>) -> String {
    let package_line = package
        .map(|package| format!("package = \"{}\"\n", toml_escape(package)))
        .unwrap_or_default();
    let avd_line = avd
        .map(|avd| format!("default_avd = \"{}\"\n", toml_escape(avd)))
        .unwrap_or_default();
    format!(
        r#"# AndroidForge project adapter (version 1).
# Project-specific behaviour lives here, in fixtures and in debug-only app hooks.
# Never place signing secrets or keystores in this file.

adapter_version = 1
min_androidforge_version = "{version}"

[project]
name = "{name}"
{package_line}{avd_line}artifact_dir = ".artifacts"
retention_days = 14

[fixtures]
directory = ".androidforge/fixtures"
# default = "clean-install"

[storage]
observation_retention = 20

[ui]
backend = "uiautomator"
default_timeout_secs = 15
poll_interval_ms = 300
settle_interval_ms = 400

[timeouts]
adb_secs = 30
install_secs = 300
boot_secs = 300
ui_secs = 15

[diagnostics]
logcat_tail_lines = 2000
max_log_bytes = 5242880
capture_on_failure = true
redact = true
include_dumps = true

# [hooks.check-background-work]
# type = "broadcast"
# action = "com.example.DEBUG_RUN_CHECK"
# component = "com.example/.DebugReceiver"
# timeout_secs = 30
"#,
        version = CURRENT_TOOL_VERSION,
        name = toml_escape(name),
        package_line = package_line,
        avd_line = avd_line,
    )
}

pub fn default_global_toml(sdk_root: Option<&Path>) -> String {
    let sdk_line = sdk_root
        .map(|root| format!("root = \"{}\"\n", toml_escape(&root.to_string_lossy())))
        .unwrap_or_default();
    format!(
        r#"# Global AndroidForge defaults. Project adapters override these values.
# Do not store secrets or environment dumps in this file.

[sdk]
{sdk_line}

[timeouts]
command_secs = 30
adb_secs = 30
install_secs = 300
boot_secs = 300
ui_secs = 15

[ui]
backend = "uiautomator"
default_timeout_secs = 15
poll_interval_ms = 300
settle_interval_ms = 400

[diagnostics]
capture_on_failure = true
redact = true

[storage]
observation_retention = 20
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_from(project_toml: &str, global_toml: &str) -> ForgeConfig {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join(".androidforge")).unwrap();
        std::fs::write(root.join(".androidforge/project.toml"), project_toml).unwrap();
        let global_dir = root.join("global");
        std::fs::create_dir_all(&global_dir).unwrap();
        std::fs::write(global_dir.join("config.toml"), global_toml).unwrap();
        let mut paths = ForgePaths::discover(Some(root.to_path_buf()), None).unwrap();
        paths.global_config_path = global_dir.join("config.toml");
        ForgeConfig::load(&paths).unwrap()
    }

    #[test]
    fn project_overrides_global_error_handling() {
        let config = config_from(
            "adapter_version = 1\n[project]\npackage = \"com.example.app\"\n[timeouts]\nadb_secs = 45\n",
            "[timeouts]\nadb_secs = 10\ncommand_secs = 20\n",
        );
        assert_eq!(config.file.timeouts.adb_secs, 45);
        assert_eq!(config.file.timeouts.command_secs, 20);
        assert!(config.validate().valid);
    }

    #[test]
    fn agent_profile_and_limits_are_typed_and_validated() {
        let config = config_from(
            "adapter_version = 1\n[agent]\nprofile = \"brief\"\nmax_items = 12\nmax_text_chars = 80\n",
            "",
        );
        assert_eq!(config.file.agent.profile, AgentProfile::Brief);
        assert_eq!(config.file.agent.max_items, 12);
        assert_eq!(config.file.agent.max_text_chars, 80);

        let mut invalid = config.clone();
        invalid.file.agent.max_items = 0;
        let validation = invalid.validate();
        assert!(!validation.valid);
        assert!(
            validation
                .issues
                .iter()
                .any(|issue| issue.field == "agent.max_items")
        );

        let mut invalid = config.clone();
        invalid.file.agent.max_text_chars = 4;
        let validation = invalid.validate();
        assert!(!validation.valid);
        assert!(
            validation
                .issues
                .iter()
                .any(|issue| issue.field == "agent.max_text_chars")
        );
    }
    #[test]
    fn storage_retention_is_typed_and_validated() {
        let config = config_from(
            "adapter_version = 1\n[project]\npackage = \"com.example.app\"\n[storage]\nobservation_retention = 7\n",
            "",
        );
        assert_eq!(config.file.storage.observation_retention, 7);

        let mut invalid = config.clone();
        invalid.file.storage.observation_retention = 1;
        assert!(!invalid.validate().valid);
        let mut invalid = config;
        invalid.file.storage.observation_retention = 1001;
        assert!(!invalid.validate().valid);
    }

    #[test]
    fn rejects_unknown_adapter_fields() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".androidforge")).unwrap();
        std::fs::write(
            temp.path().join(".androidforge/project.toml"),
            "adapter_version = 1\nnot_a_real_field = true\n",
        )
        .unwrap();
        let paths = ForgePaths::discover(Some(temp.path().to_path_buf()), None).unwrap();
        assert!(ForgeConfig::load(&paths).is_err());
    }

    #[test]
    fn validates_package_and_fingerprints() {
        let config = config_from(
            "adapter_version = 1\n[project]\npackage = \"Invalid Package\"\nexpected_cert_sha256 = [\"abc\"]\n",
            "",
        );
        let report = config.validate();
        assert!(!report.valid);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.field == "project.package")
        );
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.field == "project.expected_cert_sha256")
        );
    }

    #[test]
    fn fixture_paths_support_files_and_directories() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".androidforge")).unwrap();
        std::fs::write(
            temp.path().join(".androidforge/project.toml"),
            "adapter_version = 1\n[fixtures]\ndirectory = \".androidforge/fixtures\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join(".androidforge/fixtures/dirfixture")).unwrap();
        std::fs::write(
            temp.path()
                .join(".androidforge/fixtures/dirfixture/fixture.toml"),
            "version = 1\nname = \"dirfixture\"\n",
        )
        .unwrap();
        std::fs::write(
            temp.path().join(".androidforge/fixtures/filefixture.toml"),
            "version = 1\nname = \"filefixture\"\n",
        )
        .unwrap();
        std::fs::write(
            temp.path().join(".androidforge/fixtures/jsonfixture.json"),
            "{\"version\":1,\"name\":\"jsonfixture\"}",
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join(".androidforge/fixtures/jsondir")).unwrap();
        std::fs::write(
            temp.path()
                .join(".androidforge/fixtures/jsondir/fixture.json"),
            "{\"version\":1,\"name\":\"jsondir\"}",
        )
        .unwrap();
        let paths = ForgePaths::discover(Some(temp.path().to_path_buf()), None).unwrap();
        let config = ForgeConfig::load(&paths).unwrap();
        assert!(config.fixture_path("filefixture").is_file());
        assert!(config.fixture_path("dirfixture").is_file());
        assert_eq!(
            config.fixture_path("jsonfixture").extension().unwrap(),
            "json"
        );
        assert_eq!(
            config.fixture_path("jsondir").file_name().unwrap(),
            "fixture.json"
        );
    }

    #[test]
    fn build_section_parses_commands_cwd_timeout_and_env() {
        let config = config_from(
            "adapter_version = 1
[build]
default_variant = \"debug\"
cwd = \"app\"
timeout_secs = 120
env_passthrough = [\"JAVA_HOME\", \"CI\"]
[build.commands]
debug = [\"./gradlew\", \"assembleDebug\"]
\"freeDebug\" = [\"gradle\", \":app:assembleFreeDebug\"]
",
            "",
        );
        let build = &config.file.build;
        assert_eq!(
            build.commands.get("debug"),
            Some(&vec!["./gradlew".to_string(), "assembleDebug".to_string()])
        );
        assert_eq!(
            build.commands.get("freeDebug"),
            Some(&vec![
                "gradle".to_string(),
                ":app:assembleFreeDebug".to_string()
            ])
        );
        assert_eq!(build.cwd.as_deref(), Some("app"));
        assert_eq!(build.timeout_secs, Some(120));
        assert_eq!(build.env_passthrough, vec!["JAVA_HOME", "CI"]);
        assert!(config.validate().valid);
    }

    #[test]
    fn build_commands_reject_bad_keys_and_argv() {
        let mut config = config_from("adapter_version = 1\n", "");
        config.file.build.commands.insert(
            String::new(),
            vec!["gradle".to_string(), "assembleDebug".to_string()],
        );
        config
            .file
            .build
            .commands
            .insert("bad key".to_string(), vec!["gradle".to_string()]);
        config
            .file
            .build
            .commands
            .insert("empty-argv".to_string(), Vec::new());
        config
            .file
            .build
            .commands
            .insert("debug".to_string(), vec![String::new()]);

        let report = config.validate();
        assert!(!report.valid);
        for field in [
            "build.commands",
            "build.commands.empty-argv",
            "build.commands.debug",
        ] {
            assert!(
                report.issues.iter().any(|issue| issue.field == field),
                "missing issue for {field}: {:?}",
                report.issues
            );
        }
    }

    #[test]
    fn build_cwd_rejects_escapes_and_absolute_paths() {
        let mut config = config_from("adapter_version = 1\n", "");
        for invalid in [
            "",
            "../outside",
            "app/../../outside",
            "/tmp",
            "C:\\work",
            "\\\\server\\share",
        ] {
            config.file.build.cwd = Some(invalid.to_string());
            let report = config.validate();
            assert!(!report.valid, "'{invalid}' should be rejected");
            assert!(
                report.issues.iter().any(|issue| issue.field == "build.cwd"),
                "'{invalid}' should produce a build.cwd issue: {:?}",
                report.issues
            );
        }
        config.file.build.cwd = Some("app/src".to_string());
        assert!(config.validate().valid);
        config.file.build.cwd = None;
        assert!(config.validate().valid);
    }

    #[test]
    fn build_timeout_must_be_positive() {
        let mut config = config_from("adapter_version = 1\n[build]\ntimeout_secs = 30\n", "");
        assert_eq!(config.file.build.timeout_secs, Some(30));
        assert!(config.validate().valid);

        config.file.build.timeout_secs = Some(0);
        let report = config.validate();
        assert!(!report.valid);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.field == "build.timeout_secs")
        );

        config.file.build.timeout_secs = None;
        assert!(config.validate().valid);
    }

    #[test]
    fn build_env_passthrough_requires_names_only() {
        let mut config = config_from(
            "adapter_version = 1\n[build]\nenv_passthrough = [\"JAVA_HOME\", \"_CI\"]\n",
            "",
        );
        assert!(config.validate().valid);

        for invalid in ["", "A=B", "BAD=NAME", "A\0B", "1BAD", "WITH SPACE"] {
            config.file.build.env_passthrough = vec![invalid.to_string()];
            let report = config.validate();
            assert!(!report.valid, "'{invalid}' should be rejected");
            assert!(
                report
                    .issues
                    .iter()
                    .any(|issue| issue.field == "build.env_passthrough"),
                "'{invalid}' should produce a build.env_passthrough issue: {:?}",
                report.issues
            );
        }
    }

    #[test]
    fn build_without_commands_remains_valid() {
        let config = config_from(
            "adapter_version = 1
[build]
default_variant = \"debug\"
[build.variants]
debug = [\"app/build/outputs/apk/debug/*.apk\"]
",
            "",
        );
        assert!(config.file.build.commands.is_empty());
        assert!(config.file.build.cwd.is_none());
        assert!(config.file.build.timeout_secs.is_none());
        assert!(config.file.build.env_passthrough.is_empty());
        assert!(config.validate().valid);
    }

    #[test]
    fn build_rejects_unknown_fields() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".androidforge")).unwrap();
        std::fs::write(
            temp.path().join(".androidforge/project.toml"),
            "adapter_version = 1\n[build]\nnot_a_build_field = true\n",
        )
        .unwrap();
        let paths = ForgePaths::discover(Some(temp.path().to_path_buf()), None).unwrap();
        assert!(ForgeConfig::load(&paths).is_err());
    }
}
