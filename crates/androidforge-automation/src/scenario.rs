//! Ergonomic, declarative scenario runner built on top of the already audited
//! fixture, conformance and flaky-analysis primitives.
//!
//! A scenario manifest is a small ordered list of **allowlisted** steps. Each
//! step maps to an existing audited operation (app launch, UI tap/text/wait/
//! assert, bounds or colour checks, screenshot capture, fixture apply/reset or
//! app install). The scenario layer deliberately does **not** reimplement any
//! of those operations and it does **not** expose arbitrary shell commands or
//! raw `adb`: the dispatch boundary is the [`ScenarioExecutor`] trait, which
//! the CLI wires to the audited operations when a scenario is actually run.
//!
//! Design properties:
//!
//! * Manifests are parsed from JSON (`.json`) or TOML (anything else), matching
//!   the fixture/conformance convention. No YAML dependency is introduced.
//! * Unknown or unsafe step kinds (`shell`, `adb`, `command`, `exec`, ...) are
//!   rejected during validation, before any dispatch can happen.
//! * [`run_scenario`] with `dry_run = true` only builds a plan; the executor is
//!   never called and no consent is required.
//! * Device-mutating steps require explicit consent (`confirmed` plus
//!   `allow_device_mutation`) at the execution boundary, mirroring the fixture
//!   data-mutation consent model. Core never infers consent from device kind.
//! * Repeated iterations are aggregated into a pass rate, duration variance
//!   and a flake summary produced by the existing
//!   [`androidforge_core::analysis::analyze_flaky`] helper.
//!
//! This module is host/device agnostic: it performs no device I/O itself.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::time::Instant;

use androidforge_core::analysis::{FlakyReport, analyze_flaky};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::run::{FailureInfo, RunReport, RunStatus, RunStep, StepStatus};
use androidforge_core::util::now_rfc3339;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCENARIO_VERSION: u32 = 1;

/// Step names that can never appear in a scenario. They are rejected by shape
/// validation with a clear "unsafe step" error before serde sees the manifest.
/// A scenario never runs a shell, an adb command or an arbitrary program.
pub const UNSAFE_STEP_KINDS: &[&str] = &[
    "shell",
    "adb",
    "command",
    "exec",
    "script",
    "run",
    "raw",
    "http",
    "network",
    "file_write",
    "delete",
];

/// The audited operation a scenario step dispatches to. These are labels only:
/// the real work lives in the audited modules the CLI wires in later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditedOperation {
    AppLaunch,
    UiTap,
    UiText,
    UiWait,
    UiAssert,
    UiBounds,
    UiColorCheck,
    ScreenshotCapture,
    FixtureApply,
    FixtureReset,
    AppInstall,
}

impl AuditedOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AppLaunch => "app.launch",
            Self::UiTap => "ui.tap",
            Self::UiText => "ui.text",
            Self::UiWait => "ui.wait",
            Self::UiAssert => "ui.assert",
            Self::UiBounds => "ui.bounds",
            Self::UiColorCheck => "ui.color_check",
            Self::ScreenshotCapture => "ui.screenshot",
            Self::FixtureApply => "fixture.apply",
            Self::FixtureReset => "fixture.reset",
            Self::AppInstall => "app.install",
        }
    }
}

/// What a step does when it fails: abort the current iteration or record the
/// failure and continue with the next step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepErrorPolicy {
    #[default]
    Fail,
    Continue,
}

impl StepErrorPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fail => "fail",
            Self::Continue => "continue",
        }
    }
}

/// A lightweight selector description that maps to the audited UI selector
/// lookup. At least one condition must be present.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SelectorSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_desc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

impl SelectorSpec {
    pub fn has_any(&self) -> bool {
        self.resource_id.is_some()
            || self.text.is_some()
            || self.content_desc.is_some()
            || self.class.is_some()
    }
}

/// A UI postcondition. At least one expectation must be present.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AssertSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_absent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
}

impl AssertSpec {
    pub fn has_any(&self) -> bool {
        self.text.is_some()
            || self.text_absent.is_some()
            || self.activity.is_some()
            || self.package.is_some()
    }
}

/// A colour check at a device-pixel coordinate. `expected` is a `#RGB` or
/// `#RRGGBB` hex value parsed with the audited colour helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorExpectation {
    pub x: u32,
    pub y: u32,
    pub expected: String,
    #[serde(default)]
    pub tolerance: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundsRelation {
    BottomYEq,
    BottomYGt,
    BottomYLt,
}

impl BoundsRelation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BottomYEq => "bottom_y_eq",
            Self::BottomYGt => "bottom_y_gt",
            Self::BottomYLt => "bottom_y_lt",
        }
    }
}

/// Allowlisted scenario steps. The `type` tag is the only way to select an
/// operation; `deny_unknown_fields` keeps manifests free of ad-hoc keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScenarioStep {
    Launch {
        package: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        activity: Option<String>,
        #[serde(default, skip_serializing_if = "is_default_policy")]
        on_error: StepErrorPolicy,
    },
    Tap {
        selector: SelectorSpec,
        #[serde(default, skip_serializing_if = "is_default_policy")]
        on_error: StepErrorPolicy,
    },
    Text {
        selector: SelectorSpec,
        value: String,
        #[serde(default, skip_serializing_if = "is_default_policy")]
        on_error: StepErrorPolicy,
    },
    Wait {
        selector: SelectorSpec,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "is_default_policy")]
        on_error: StepErrorPolicy,
    },
    Assert {
        expect: AssertSpec,
        #[serde(default, skip_serializing_if = "is_default_policy")]
        on_error: StepErrorPolicy,
    },
    Bounds {
        selector: SelectorSpec,
        relation: BoundsRelation,
        expected: i32,
        #[serde(default)]
        tolerance: i32,
        #[serde(default, skip_serializing_if = "is_default_policy")]
        on_error: StepErrorPolicy,
    },
    Color {
        expect: ColorExpectation,
        #[serde(default, skip_serializing_if = "is_default_policy")]
        on_error: StepErrorPolicy,
    },
    Screenshot {
        name: String,
        #[serde(default, skip_serializing_if = "is_default_policy")]
        on_error: StepErrorPolicy,
    },
    FixtureApply {
        name: String,
        #[serde(default, skip_serializing_if = "is_default_policy")]
        on_error: StepErrorPolicy,
    },
    FixtureReset {
        name: String,
        #[serde(default, skip_serializing_if = "is_default_policy")]
        on_error: StepErrorPolicy,
    },
    Install {
        apk: String,
        #[serde(default, skip_serializing_if = "is_default_policy")]
        on_error: StepErrorPolicy,
    },
}

fn is_default_policy(policy: &StepErrorPolicy) -> bool {
    *policy == StepErrorPolicy::Fail
}

impl ScenarioStep {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Launch { .. } => "launch",
            Self::Tap { .. } => "tap",
            Self::Text { .. } => "text",
            Self::Wait { .. } => "wait",
            Self::Assert { .. } => "assert",
            Self::Bounds { .. } => "bounds",
            Self::Color { .. } => "color",
            Self::Screenshot { .. } => "screenshot",
            Self::FixtureApply { .. } => "fixture_apply",
            Self::FixtureReset { .. } => "fixture_reset",
            Self::Install { .. } => "install",
        }
    }

    pub fn on_error(&self) -> StepErrorPolicy {
        match self {
            Self::Launch { on_error, .. }
            | Self::Tap { on_error, .. }
            | Self::Text { on_error, .. }
            | Self::Wait { on_error, .. }
            | Self::Assert { on_error, .. }
            | Self::Bounds { on_error, .. }
            | Self::Color { on_error, .. }
            | Self::Screenshot { on_error, .. }
            | Self::FixtureApply { on_error, .. }
            | Self::FixtureReset { on_error, .. }
            | Self::Install { on_error, .. } => *on_error,
        }
    }

    /// True when the step changes device state and therefore needs consent.
    pub fn mutates_device(&self) -> bool {
        matches!(
            self,
            Self::Launch { .. }
                | Self::Tap { .. }
                | Self::Text { .. }
                | Self::FixtureApply { .. }
                | Self::FixtureReset { .. }
                | Self::Install { .. }
        )
    }

    /// The audited operation this step is an ergonomic wrapper over.
    pub fn audited_operation(&self) -> AuditedOperation {
        match self {
            Self::Launch { .. } => AuditedOperation::AppLaunch,
            Self::Tap { .. } => AuditedOperation::UiTap,
            Self::Text { .. } => AuditedOperation::UiText,
            Self::Wait { .. } => AuditedOperation::UiWait,
            Self::Assert { .. } => AuditedOperation::UiAssert,
            Self::Bounds { .. } => AuditedOperation::UiBounds,
            Self::Color { .. } => AuditedOperation::UiColorCheck,
            Self::Screenshot { .. } => AuditedOperation::ScreenshotCapture,
            Self::FixtureApply { .. } => AuditedOperation::FixtureApply,
            Self::FixtureReset { .. } => AuditedOperation::FixtureReset,
            Self::Install { .. } => AuditedOperation::AppInstall,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Launch {
                package, activity, ..
            } => match activity {
                Some(activity) => format!("launch {package}/{activity}"),
                None => format!("launch {package}"),
            },
            Self::Tap { selector, .. } => format!("tap {}", describe_selector(selector)),
            Self::Text {
                selector, value, ..
            } => {
                format!("text {} = {:?}", describe_selector(selector), value)
            }
            Self::Wait {
                selector,
                timeout_ms,
                ..
            } => {
                format!(
                    "wait for {} ({:?} ms)",
                    describe_selector(selector),
                    timeout_ms
                )
            }
            Self::Assert { expect, .. } => format!("assert {}", describe_assert(expect)),
            Self::Bounds {
                selector,
                relation,
                expected,
                tolerance,
                ..
            } => format!(
                "bounds {} {} {} (±{})",
                describe_selector(selector),
                relation.as_str(),
                expected,
                tolerance
            ),
            Self::Color { expect, .. } => format!(
                "color ({},{}) {} ±{}",
                expect.x, expect.y, expect.expected, expect.tolerance
            ),
            Self::Screenshot { name, .. } => format!("screenshot {name}"),
            Self::FixtureApply { name, .. } => format!("fixture apply {name}"),
            Self::FixtureReset { name, .. } => format!("fixture reset {name}"),
            Self::Install { apk, .. } => format!("install {apk}"),
        }
    }
}

fn describe_selector(selector: &SelectorSpec) -> String {
    let mut parts = Vec::new();
    if let Some(value) = &selector.resource_id {
        parts.push(format!("id={value}"));
    }
    if let Some(value) = &selector.text {
        parts.push(format!("text={value}"));
    }
    if let Some(value) = &selector.content_desc {
        parts.push(format!("desc={value}"));
    }
    if let Some(value) = &selector.class {
        parts.push(format!("class={value}"));
    }
    if parts.is_empty() {
        "<empty selector>".to_string()
    } else {
        parts.join(", ")
    }
}

fn describe_assert(expect: &AssertSpec) -> String {
    let mut parts = Vec::new();
    if let Some(value) = &expect.text {
        parts.push(format!("text={value}"));
    }
    if let Some(value) = &expect.text_absent {
        parts.push(format!("text_absent={value}"));
    }
    if let Some(value) = &expect.activity {
        parts.push(format!("activity={value}"));
    }
    if let Some(value) = &expect.package {
        parts.push(format!("package={value}"));
    }
    if parts.is_empty() {
        "<empty assertion>".to_string()
    } else {
        parts.join(", ")
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioRepeat {
    /// Number of times the step list is executed. Defaults to 1.
    pub count: u32,
}

impl Default for ScenarioRepeat {
    fn default() -> Self {
        Self { count: 1 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioManifest {
    #[serde(default = "default_version")]
    pub version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub repeat: ScenarioRepeat,
    #[serde(default)]
    pub steps: Vec<ScenarioStep>,
    #[serde(skip)]
    pub source_path: Option<String>,
}

fn default_version() -> u32 {
    SCENARIO_VERSION
}

impl Default for ScenarioManifest {
    fn default() -> Self {
        Self {
            version: SCENARIO_VERSION,
            name: String::new(),
            description: None,
            repeat: ScenarioRepeat::default(),
            steps: Vec::new(),
            source_path: None,
        }
    }
}

/// Explicit, device-agnostic consent for mutating steps. Core never infers
/// consent from whether the device is an emulator or a physical handset.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScenarioConsent {
    pub confirmed: bool,
    pub allow_device_mutation: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ScenarioRunOptions {
    pub dry_run: bool,
    pub consent: ScenarioConsent,
    pub trace: bool,
}

/// Context passed to the executor for a single step attempt.
#[derive(Debug, Clone)]
pub struct StepContext {
    pub scenario: String,
    pub iteration: u32,
    pub step_index: usize,
}

/// Result of one audited step dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepOutcome {
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default)]
    pub duration_ms: u64,
}

impl StepOutcome {
    pub fn pass() -> Self {
        Self {
            passed: true,
            error_code: None,
            error_message: None,
            duration_ms: 0,
        }
    }

    pub fn fail(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            passed: false,
            error_code: Some(code.into()),
            error_message: Some(message.into()),
            duration_ms: 0,
        }
    }
}

/// The dispatch boundary. A scenario never runs a shell or raw adb itself; the
/// CLI supplies an executor that forwards each step to the audited operation
/// named by [`ScenarioStep::audited_operation`].
pub trait ScenarioExecutor: Send + Sync {
    fn execute<'a>(
        &'a self,
        step: &'a ScenarioStep,
        context: &'a StepContext,
    ) -> Pin<Box<dyn Future<Output = Result<StepOutcome>> + Send + 'a>>;
}

impl<F> ScenarioExecutor for F
where
    F: Fn(&ScenarioStep, &StepContext) -> Result<StepOutcome> + Send + Sync,
{
    fn execute<'a>(
        &'a self,
        step: &'a ScenarioStep,
        context: &'a StepContext,
    ) -> Pin<Box<dyn Future<Output = Result<StepOutcome>> + Send + 'a>> {
        Box::pin(async move { self(step, context) })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedStep {
    pub index: usize,
    pub kind: String,
    pub description: String,
    pub audited_operation: String,
    pub mutates_device: bool,
    pub on_error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioPlan {
    pub scenario: String,
    pub repeat: u32,
    pub steps: Vec<PlannedStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub index: usize,
    pub kind: String,
    pub audited_operation: String,
    /// `planned`, `success`, `failed` or `skipped`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationResult {
    pub iteration: u32,
    pub passed: bool,
    pub steps: Vec<StepRecord>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakeSummary {
    pub min_runs: usize,
    pub steps_analyzed: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flaky_steps: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub classifications: BTreeMap<String, String>,
    pub heuristic: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioSummary {
    pub iterations: u32,
    pub passed: u32,
    pub failed: u32,
    pub pass_rate: f64,
    pub duration_mean_ms: f64,
    /// Population variance of the per-iteration wall-clock durations.
    pub duration_variance_ms2: f64,
    pub duration_stddev_ms: f64,
    pub flake: FlakeSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioRunResult {
    pub name: String,
    pub dry_run: bool,
    pub plan: ScenarioPlan,
    pub iterations: Vec<IterationResult>,
    pub summary: ScenarioSummary,
}

impl ScenarioRunResult {
    pub fn is_pass(&self) -> bool {
        !self.iterations.is_empty() && self.iterations.iter().all(|iteration| iteration.passed)
    }
}

fn invalid(message: impl Into<String>) -> ForgeError {
    ForgeError::new(ErrorCode::ConfigInvalid, message)
}

/// Parse a scenario manifest. `.json` is parsed as JSON; every other extension
/// is parsed as TOML. No YAML support is added (and no dependency introduced).
pub fn parse_scenario_text(path: &Path, text: &str) -> Result<ScenarioManifest> {
    let is_json = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
    let shape: Option<Value> = if is_json {
        serde_json::from_str(text).ok()
    } else {
        toml::from_str(text).ok()
    };
    if let Some(value) = &shape {
        validate_manifest_shape(value)?;
    }
    let mut manifest: ScenarioManifest = if is_json {
        serde_json::from_str(text).map_err(|error| {
            invalid(format!(
                "invalid scenario manifest {}: {error}",
                path.display()
            ))
        })?
    } else {
        toml::from_str(text).map_err(|error| {
            invalid(format!(
                "invalid scenario manifest {}: {error}",
                path.display()
            ))
        })?
    };
    manifest.source_path = Some(path.to_string_lossy().into_owned());
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub fn load_scenario(path: &Path) -> Result<ScenarioManifest> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        invalid(format!(
            "could not read scenario manifest {}: {error}",
            path.display()
        ))
    })?;
    parse_scenario_text(path, &text)
}

/// Reject disallowed step kinds before deserialization so callers get an
/// explicit "unsafe operation" error rather than a generic serde failure.
pub fn validate_manifest_shape(value: &Value) -> Result<()> {
    let Some(steps) = value.get("steps").and_then(Value::as_array) else {
        return Ok(());
    };
    for step in steps {
        let Some(kind) = step.get("type").and_then(Value::as_str) else {
            continue; // serde will report the missing tag
        };
        if UNSAFE_STEP_KINDS
            .iter()
            .any(|unsafe_kind| kind.eq_ignore_ascii_case(unsafe_kind))
        {
            return Err(invalid(format!(
                "unsafe scenario step '{kind}': scenarios may not run a shell, adb or arbitrary command"
            ))
            .with_context_string("step", kind)
            .with_suggestion(
                "Use an allowlisted step such as launch, tap, text, wait, assert, bounds, color, screenshot, fixture_apply, fixture_reset or install.",
            ));
        }
    }
    Ok(())
}

pub fn validate_manifest(manifest: &ScenarioManifest) -> Result<()> {
    if manifest.version != SCENARIO_VERSION {
        return Err(invalid(format!(
            "unsupported scenario version {}; expected {SCENARIO_VERSION}",
            manifest.version
        )));
    }
    if manifest.name.trim().is_empty() {
        return Err(invalid("scenario name must not be empty"));
    }
    if manifest.steps.is_empty() {
        return Err(invalid(format!(
            "scenario '{}' has no steps",
            manifest.name
        )));
    }
    if manifest.repeat.count == 0 {
        return Err(invalid(format!(
            "scenario '{}' repeat.count must be at least 1",
            manifest.name
        )));
    }
    for (index, step) in manifest.steps.iter().enumerate() {
        validate_step(step)
            .map_err(|error| error.with_context_string("step_index", index.to_string()))?;
    }
    Ok(())
}

fn validate_step(step: &ScenarioStep) -> Result<()> {
    let name = step.kind();
    let bad_input = |detail: &str| invalid(format!("scenario step '{name}' is invalid: {detail}"));
    match step {
        ScenarioStep::Launch { package, .. } => {
            if package.trim().is_empty() {
                return Err(bad_input("package must not be empty"));
            }
        }
        ScenarioStep::Tap { selector, .. }
        | ScenarioStep::Wait { selector, .. }
        | ScenarioStep::Bounds { selector, .. } => {
            if !selector.has_any() {
                return Err(bad_input("selector needs at least one condition"));
            }
        }
        ScenarioStep::Text {
            selector, value, ..
        } => {
            if !selector.has_any() {
                return Err(bad_input("selector needs at least one condition"));
            }
            if value.is_empty() {
                return Err(bad_input("text value must not be empty"));
            }
        }
        ScenarioStep::Assert { expect, .. } => {
            if !expect.has_any() {
                return Err(bad_input("assertion needs at least one expectation"));
            }
        }
        ScenarioStep::Color { expect, .. } => {
            androidforge_ui::color::parse_hex(&expect.expected)
                .map_err(|error| bad_input(error.message()))?;
        }
        ScenarioStep::Screenshot { name, .. }
        | ScenarioStep::FixtureApply { name, .. }
        | ScenarioStep::FixtureReset { name, .. } => {
            if name.trim().is_empty() {
                return Err(bad_input("name must not be empty"));
            }
        }
        ScenarioStep::Install { apk, .. } => {
            if apk.trim().is_empty() {
                return Err(bad_input("apk path must not be empty"));
            }
        }
    }
    Ok(())
}

/// Build the ordered, host-agnostic plan for a scenario. This is pure: it does
/// not touch a device and does not call an executor.
pub fn plan_scenario(manifest: &ScenarioManifest) -> ScenarioPlan {
    let steps = manifest
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| PlannedStep {
            index,
            kind: step.kind().to_string(),
            description: step.describe(),
            audited_operation: step.audited_operation().as_str().to_string(),
            mutates_device: step.mutates_device(),
            on_error: step.on_error().as_str().to_string(),
        })
        .collect();
    ScenarioPlan {
        scenario: manifest.name.clone(),
        repeat: manifest.repeat.count,
        steps,
    }
}

fn require_mutation_consent(plan: &ScenarioPlan, options: &ScenarioRunOptions) -> Result<()> {
    if plan.steps.iter().any(|step| step.mutates_device)
        && !(options.consent.confirmed && options.consent.allow_device_mutation)
    {
        return Err(ForgeError::new(
            ErrorCode::PhysicalDeviceMutationBlocked,
            format!(
                "scenario '{}' contains device-mutating steps but consent was not granted",
                plan.scenario
            ),
        )
        .with_context_string(
            "required_consent",
            "confirmed=true and allow_device_mutation=true",
        )
        .with_suggestion(
            "Set explicit consent on ScenarioRunOptions before dispatching mutating steps.",
        )
        .with_suggestion("Dry-run plans never dispatch and never require consent."));
    }
    Ok(())
}

/// Run a scenario, repeating the step list `repeat.count` times. When
/// `options.dry_run` is true the executor is never called and every step is
/// recorded as `planned`.
pub async fn run_scenario(
    manifest: &ScenarioManifest,
    options: &ScenarioRunOptions,
    executor: &dyn ScenarioExecutor,
) -> Result<ScenarioRunResult> {
    validate_manifest(manifest)?;
    let plan = plan_scenario(manifest);
    if !options.dry_run {
        require_mutation_consent(&plan, options)?;
    }
    let mut iterations = Vec::with_capacity(manifest.repeat.count as usize);
    for iteration in 1..=manifest.repeat.count {
        iterations.push(run_iteration(manifest, options, executor, iteration).await);
    }
    let summary = summarize(manifest, &iterations);
    Ok(ScenarioRunResult {
        name: manifest.name.clone(),
        dry_run: options.dry_run,
        plan,
        iterations,
        summary,
    })
}

async fn run_iteration(
    manifest: &ScenarioManifest,
    options: &ScenarioRunOptions,
    executor: &dyn ScenarioExecutor,
    iteration: u32,
) -> IterationResult {
    let started = Instant::now();
    let mut records = Vec::with_capacity(manifest.steps.len());
    let mut failed = false;
    for (index, step) in manifest.steps.iter().enumerate() {
        let context = StepContext {
            scenario: manifest.name.clone(),
            iteration,
            step_index: index,
        };
        let outcome = if options.dry_run {
            StepOutcome::pass()
        } else {
            match executor.execute(step, &context).await {
                Ok(outcome) => outcome,
                Err(error) => StepOutcome::fail(error.code().as_str(), error.message()),
            }
        };
        let status = if options.dry_run {
            "planned"
        } else if outcome.passed {
            "success"
        } else {
            "failed"
        };
        records.push(StepRecord {
            index,
            kind: step.kind().to_string(),
            audited_operation: step.audited_operation().as_str().to_string(),
            status: status.to_string(),
            error_code: outcome.error_code,
            error_message: outcome.error_message,
            duration_ms: outcome.duration_ms,
        });
        if !outcome.passed {
            failed = true;
            if step.on_error() == StepErrorPolicy::Fail {
                for (later_index, later_step) in manifest.steps.iter().enumerate().skip(index + 1) {
                    records.push(StepRecord {
                        index: later_index,
                        kind: later_step.kind().to_string(),
                        audited_operation: later_step.audited_operation().as_str().to_string(),
                        status: if options.dry_run {
                            "planned"
                        } else {
                            "skipped"
                        }
                        .to_string(),
                        error_code: None,
                        error_message: None,
                        duration_ms: 0,
                    });
                }
                break;
            }
        }
    }
    IterationResult {
        iteration,
        passed: !failed,
        steps: records,
        duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    }
}

/// Aggregate iterations into pass rate, duration variance and a flake summary.
/// This is pure and can be called on hand-built iterations in tests or by the
/// CLI without performing any device work.
pub fn summarize(manifest: &ScenarioManifest, iterations: &[IterationResult]) -> ScenarioSummary {
    let total = iterations.len() as u32;
    let passed = iterations
        .iter()
        .filter(|iteration| iteration.passed)
        .count() as u32;
    let pass_rate = if total == 0 {
        0.0
    } else {
        f64::from(passed) / f64::from(total)
    };
    let durations: Vec<f64> = iterations
        .iter()
        .map(|iteration| iteration.duration_ms as f64)
        .collect();
    let (duration_mean_ms, duration_variance_ms2, duration_stddev_ms) = variance_stats(&durations);
    ScenarioSummary {
        iterations: total,
        passed,
        failed: total.saturating_sub(passed),
        pass_rate,
        duration_mean_ms,
        duration_variance_ms2,
        duration_stddev_ms,
        flake: flake_summary(manifest, iterations),
    }
}

fn variance_stats(values: &[f64]) -> (f64, f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f64>()
        / values.len() as f64;
    (mean, variance, variance.sqrt())
}

fn flake_summary(manifest: &ScenarioManifest, iterations: &[IterationResult]) -> FlakeSummary {
    let min_runs = manifest.repeat.count.max(2) as usize;
    let reports: Vec<RunReport> = iterations
        .iter()
        .map(|iteration| iteration_report(manifest, iteration))
        .collect();
    let report: FlakyReport = analyze_flaky(&reports, min_runs);
    let mut classifications = BTreeMap::new();
    let mut flaky_steps = Vec::new();
    for step in &report.steps {
        classifications.insert(step.step_name.clone(), step.classification.clone());
        if step.classification == "flaky" {
            flaky_steps.push(step.step_name.clone());
        }
    }
    FlakeSummary {
        min_runs: report.min_runs,
        steps_analyzed: report.steps.len(),
        flaky_steps,
        classifications,
        heuristic: report.heuristic,
    }
}

fn iteration_report(manifest: &ScenarioManifest, iteration: &IterationResult) -> RunReport {
    let mut report = RunReport::new(
        format!("{}-{}", manifest.name, iteration.iteration),
        format!("scenario {}", manifest.name),
        String::new(),
        String::new(),
    );
    report.status = if iteration.passed {
        RunStatus::Passed
    } else {
        RunStatus::Failed
    };
    report.duration_ms = iteration.duration_ms;
    report.steps = iteration
        .steps
        .iter()
        .map(|record| {
            let status = match record.status.as_str() {
                "success" => StepStatus::Success,
                "failed" => StepStatus::Failed,
                _ => StepStatus::Skipped,
            };
            let failure = if status == StepStatus::Failed {
                let code = record
                    .error_code
                    .as_deref()
                    .and_then(ErrorCode::from_code)
                    .unwrap_or(ErrorCode::CommandFailed);
                Some(FailureInfo {
                    code,
                    message: record.error_message.clone().unwrap_or_default(),
                    failed_operation: record.audited_operation.clone(),
                    recoverable: true,
                    suggestions: Vec::new(),
                    context: BTreeMap::new(),
                    artifacts: Vec::new(),
                })
            } else {
                None
            };
            RunStep {
                id: format!("step-{:03}", record.index + 1),
                name: record.kind.clone(),
                command: record.audited_operation.clone(),
                status,
                exit_code: Some(if status == StepStatus::Failed { 1 } else { 0 }),
                started_at: now_rfc3339(),
                ended_at: Some(now_rfc3339()),
                duration_ms: record.duration_ms,
                failure,
                artifacts: Vec::new(),
                details: Value::Object(Default::default()),
                retries: 0,
            }
        })
        .collect();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TOML_MANIFEST: &str = r#"
version = 1
name = "smoke"
description = "host-agnostic smoke scenario"

[repeat]
count = 3

[[steps]]
type = "launch"
package = "com.example.app"

[[steps]]
type = "tap"
[steps.selector]
resource_id = "com.example.app:id/login"

[[steps]]
type = "assert"
[steps.expect]
text = "Welcome"
"#;

    fn json_manifest() -> Value {
        serde_json::json!({
            "version": 1,
            "name": "smoke",
            "repeat": { "count": 2 },
            "steps": [
                { "type": "screenshot", "name": "home" },
                {
                    "type": "text",
                    "selector": { "text": "Email" },
                    "value": "user@example.com",
                    "on_error": "continue"
                }
            ]
        })
    }

    fn validator() -> jsonschema::Validator {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/scenario.schema.json");
        let text = std::fs::read_to_string(&path).expect("read scenario schema");
        let schema: Value = serde_json::from_str(&text).expect("parse scenario schema");
        jsonschema::meta::validate(&schema).expect("scenario schema is not valid draft 2020-12");
        jsonschema::validator_for(&schema).expect("compile scenario schema")
    }

    struct ScriptedExecutor {
        calls: AtomicUsize,
        results: Mutex<VecDeque<Result<StepOutcome>>>,
    }

    impl ScriptedExecutor {
        fn new(results: Vec<Result<StepOutcome>>) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                results: Mutex::new(results.into()),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl ScenarioExecutor for ScriptedExecutor {
        fn execute<'a>(
            &'a self,
            _step: &'a ScenarioStep,
            _context: &'a StepContext,
        ) -> Pin<Box<dyn Future<Output = Result<StepOutcome>> + Send + 'a>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.results
                    .lock()
                    .expect("scripted results lock")
                    .pop_front()
                    .unwrap_or_else(|| Ok(StepOutcome::pass()))
            })
        }
    }

    fn consent() -> ScenarioConsent {
        ScenarioConsent {
            confirmed: true,
            allow_device_mutation: true,
        }
    }

    fn record(
        index: usize,
        kind: &str,
        status: &str,
        passed: bool,
        duration_ms: u64,
    ) -> StepRecord {
        StepRecord {
            index,
            kind: kind.to_string(),
            audited_operation: kind.to_string(),
            status: status.to_string(),
            error_code: if passed {
                None
            } else {
                Some(ErrorCode::UiAssertionFailed.as_str().to_string())
            },
            error_message: if passed {
                None
            } else {
                Some("assertion failed".to_string())
            },
            duration_ms,
        }
    }

    #[test]
    fn parses_toml_and_json_manifests_and_matches_schema() {
        let toml_manifest =
            parse_scenario_text(Path::new("scenario.toml"), TOML_MANIFEST).expect("parse TOML");
        assert_eq!(toml_manifest.name, "smoke");
        assert_eq!(toml_manifest.repeat.count, 3);
        assert_eq!(toml_manifest.steps.len(), 3);
        assert_eq!(toml_manifest.steps[0].kind(), "launch");
        assert_eq!(toml_manifest.steps[1].kind(), "tap");
        assert_eq!(toml_manifest.steps[2].kind(), "assert");

        let value = json_manifest();
        let json_manifest = parse_scenario_text(Path::new("scenario.json"), &value.to_string())
            .expect("parse JSON");
        assert_eq!(json_manifest.name, "smoke");
        assert_eq!(json_manifest.steps.len(), 2);
        assert_eq!(json_manifest.steps[1].on_error(), StepErrorPolicy::Continue);

        let schema = validator();
        assert!(schema.is_valid(&serde_json::to_value(&toml_manifest).unwrap()));
        assert!(schema.is_valid(&value));
    }

    #[test]
    fn rejects_unknown_and_unsafe_steps() {
        let mut unsafe_step = json_manifest();
        unsafe_step["steps"][0]["type"] = serde_json::json!("shell");
        let error = parse_scenario_text(Path::new("bad.json"), &unsafe_step.to_string())
            .expect_err("shell step must be rejected");
        assert_eq!(error.code(), ErrorCode::ConfigInvalid);
        assert!(error.message().contains("unsafe scenario step"));

        let mut unsafe_adb = json_manifest();
        unsafe_adb["steps"][0]["type"] = serde_json::json!("adb");
        assert!(parse_scenario_text(Path::new("bad.json"), &unsafe_adb.to_string()).is_err());

        let mut unknown_kind = json_manifest();
        unknown_kind["steps"][0]["type"] = serde_json::json!("teleport");
        assert!(parse_scenario_text(Path::new("bad.json"), &unknown_kind.to_string()).is_err());

        let mut unknown_field = json_manifest();
        unknown_field["steps"][0]["surprise"] = serde_json::json!(true);
        assert!(parse_scenario_text(Path::new("bad.json"), &unknown_field.to_string()).is_err());

        let mut unknown_top_level = json_manifest();
        unknown_top_level["mystery"] = serde_json::json!(1);
        assert!(
            parse_scenario_text(Path::new("bad.json"), &unknown_top_level.to_string()).is_err()
        );

        let schema = validator();
        assert!(!schema.is_valid(&unsafe_step));
        assert!(!schema.is_valid(&unknown_field));

        let mut empty_selector = json_manifest();
        empty_selector["steps"][1]["selector"] = serde_json::json!({});
        assert!(parse_scenario_text(Path::new("bad.json"), &empty_selector.to_string()).is_err());
    }

    #[tokio::test]
    async fn dry_run_builds_plan_without_dispatch() {
        let manifest =
            parse_scenario_text(Path::new("scenario.toml"), TOML_MANIFEST).expect("parse TOML");
        let plan = plan_scenario(&manifest);
        assert_eq!(plan.repeat, 3);
        assert_eq!(plan.steps.len(), 3);
        assert!(plan.steps[0].mutates_device);
        assert_eq!(plan.steps[0].audited_operation, "app.launch");
        assert!(!plan.steps[2].mutates_device);
        assert_eq!(plan.steps[2].audited_operation, "ui.assert");

        // No consent is supplied and the executor would dispatch on any call.
        let executor = ScriptedExecutor::new(Vec::new());
        let options = ScenarioRunOptions {
            dry_run: true,
            ..ScenarioRunOptions::default()
        };
        let result = run_scenario(&manifest, &options, &executor)
            .await
            .expect("dry run succeeds");
        assert_eq!(executor.calls(), 0);
        assert!(result.dry_run);
        assert_eq!(result.iterations.len(), 3);
        assert!(
            result
                .iterations
                .iter()
                .all(|iteration| { iteration.steps.iter().all(|step| step.status == "planned") })
        );
    }

    #[test]
    fn repeat_aggregates_pass_rate_and_duration_variance() {
        let manifest =
            parse_scenario_text(Path::new("scenario.toml"), TOML_MANIFEST).expect("parse TOML");
        let iterations = vec![
            IterationResult {
                iteration: 1,
                passed: true,
                steps: vec![record(0, "tap", "success", true, 10)],
                duration_ms: 100,
            },
            IterationResult {
                iteration: 2,
                passed: true,
                steps: vec![record(0, "tap", "success", true, 20)],
                duration_ms: 200,
            },
            IterationResult {
                iteration: 3,
                passed: false,
                steps: vec![record(0, "tap", "failed", false, 30)],
                duration_ms: 300,
            },
        ];
        let summary = summarize(&manifest, &iterations);
        assert_eq!(summary.iterations, 3);
        assert_eq!(summary.passed, 2);
        assert_eq!(summary.failed, 1);
        assert!((summary.pass_rate - 2.0 / 3.0).abs() < f64::EPSILON);
        assert!((summary.duration_mean_ms - 200.0).abs() < f64::EPSILON);
        assert!((summary.duration_variance_ms2 - 20000.0 / 3.0).abs() < 1e-9);
        assert!((summary.duration_stddev_ms - (20000.0 / 3.0_f64).sqrt()).abs() < 1e-9);
    }

    #[test]
    fn flake_summary_uses_existing_analysis() {
        let manifest =
            parse_scenario_text(Path::new("scenario.toml"), TOML_MANIFEST).expect("parse TOML");
        let outcomes = [true, false, true, false];
        let iterations = outcomes
            .into_iter()
            .enumerate()
            .map(|(index, passed)| IterationResult {
                iteration: (index + 1) as u32,
                passed,
                steps: vec![record(
                    0,
                    "tap",
                    if passed { "success" } else { "failed" },
                    passed,
                    5,
                )],
                duration_ms: 10,
            })
            .collect::<Vec<_>>();
        let summary = summarize(&manifest, &iterations);
        assert_eq!(summary.flake.min_runs, 3);
        assert_eq!(
            summary.flake.classifications.get("tap").map(String::as_str),
            Some("flaky")
        );
        assert_eq!(summary.flake.flaky_steps, vec!["tap".to_string()]);
        assert!(summary.flake.heuristic.contains("flaky"));
    }

    #[test]
    fn flake_summary_marks_insufficient_evidence_for_a_single_run() {
        let mut manifest =
            parse_scenario_text(Path::new("scenario.toml"), TOML_MANIFEST).expect("parse TOML");
        manifest.repeat.count = 1;
        let iterations = vec![IterationResult {
            iteration: 1,
            passed: true,
            steps: vec![record(0, "tap", "success", true, 5)],
            duration_ms: 10,
        }];
        let summary = summarize(&manifest, &iterations);
        assert_eq!(summary.flake.min_runs, 2);
        assert_eq!(
            summary.flake.classifications.get("tap").map(String::as_str),
            Some("insufficient_evidence")
        );
        assert!(summary.flake.flaky_steps.is_empty());
    }

    #[tokio::test]
    async fn failure_propagation_respects_per_step_policy() {
        let manifest: ScenarioManifest = serde_json::from_value(serde_json::json!({
            "version": 1,
            "name": "fail-policy",
            "steps": [
                { "type": "screenshot", "name": "before" },
                { "type": "assert", "expect": { "text": "Welcome" } },
                { "type": "screenshot", "name": "after" }
            ]
        }))
        .expect("parse manifest");

        // First step fails; `fail` policy aborts the iteration.
        let failing = ScriptedExecutor::new(vec![Err(ForgeError::new(
            ErrorCode::UiAssertionFailed,
            "screen mismatch",
        ))]);
        let options = ScenarioRunOptions::default(); // read-only steps need no consent
        let result = run_scenario(&manifest, &options, &failing)
            .await
            .expect("run completes");
        let iteration = &result.iterations[0];
        assert!(!iteration.passed);
        assert_eq!(iteration.steps.len(), 3);
        assert_eq!(iteration.steps[0].status, "failed");
        assert_eq!(
            iteration.steps[0].error_code.as_deref(),
            Some("E_UI_ASSERTION_FAILED")
        );
        assert_eq!(iteration.steps[1].status, "skipped");
        assert_eq!(iteration.steps[2].status, "skipped");
        assert_eq!(failing.calls(), 1);

        // `continue` policy records the failure and still runs the next step.
        let mut continuing_manifest = manifest.clone();
        if let ScenarioStep::Screenshot { on_error, .. } = &mut continuing_manifest.steps[0] {
            *on_error = StepErrorPolicy::Continue;
        }
        let continuing = ScriptedExecutor::new(vec![
            Err(ForgeError::new(
                ErrorCode::UiAssertionFailed,
                "screen mismatch",
            )),
            Ok(StepOutcome::pass()),
            Ok(StepOutcome::pass()),
        ]);
        let result = run_scenario(&continuing_manifest, &options, &continuing)
            .await
            .expect("run completes");
        let iteration = &result.iterations[0];
        assert!(!iteration.passed);
        assert_eq!(iteration.steps[0].status, "failed");
        assert_eq!(iteration.steps[1].status, "success");
        assert_eq!(iteration.steps[2].status, "success");
        assert_eq!(continuing.calls(), 3);
        assert!(!result.is_pass());
    }

    #[tokio::test]
    async fn mutating_steps_require_explicit_consent() {
        let manifest =
            parse_scenario_text(Path::new("scenario.toml"), TOML_MANIFEST).expect("parse TOML");
        let executor = ScriptedExecutor::new(Vec::new());
        let blocked = run_scenario(&manifest, &ScenarioRunOptions::default(), &executor)
            .await
            .expect_err("mutation without consent is blocked");
        assert_eq!(blocked.code(), ErrorCode::PhysicalDeviceMutationBlocked);
        assert_eq!(executor.calls(), 0);

        let options = ScenarioRunOptions {
            consent: consent(),
            ..ScenarioRunOptions::default()
        };
        let result = run_scenario(&manifest, &options, &executor)
            .await
            .expect("consent allows dispatch");
        assert_eq!(result.iterations.len(), 3);
        assert_eq!(executor.calls(), 9);
        assert!(result.is_pass());
    }

    #[tokio::test]
    async fn invalid_manifest_is_rejected_before_dispatch() {
        let mut manifest =
            parse_scenario_text(Path::new("scenario.toml"), TOML_MANIFEST).expect("parse TOML");
        manifest.steps.clear();
        let executor = ScriptedExecutor::new(Vec::new());
        let options = ScenarioRunOptions {
            consent: consent(),
            ..ScenarioRunOptions::default()
        };
        let error = run_scenario(&manifest, &options, &executor)
            .await
            .expect_err("empty steps are invalid");
        assert_eq!(error.code(), ErrorCode::ConfigInvalid);
        assert_eq!(executor.calls(), 0);
    }

    #[test]
    fn unsafe_step_kinds_are_documented_and_rejected_by_shape() {
        for kind in UNSAFE_STEP_KINDS {
            let value = serde_json::json!({ "steps": [{ "type": kind }] });
            assert!(
                validate_manifest_shape(&value).is_err(),
                "{kind} should be unsafe"
            );
        }
        let value =
            serde_json::json!({ "steps": [{ "type": "launch", "package": "com.example" }] });
        assert!(validate_manifest_shape(&value).is_ok());
    }
}
