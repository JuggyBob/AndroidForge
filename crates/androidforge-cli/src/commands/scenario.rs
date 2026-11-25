//! CLI/MCP wiring for the declarative scenario runner.
//!
//! The scenario core (`androidforge_automation::scenario`) owns parsing,
//! validation, planning, repeats, per-step error policy, pass rate, duration
//! variance and flake analysis. This module is the device-facing boundary: it
//! supplies a concrete [`ScenarioExecutor`] that maps every allowlisted step
//! to an already audited app/UI/fixture operation and records each attempt in
//! the current run report. No step here can run a shell or raw `adb`.

use std::future::Future;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, Instant};

use androidforge_automation::{
    AssertSpec, BoundsRelation, ColorExpectation, FixtureAction, FixtureStateExecution,
    ScenarioConsent, ScenarioExecutor, ScenarioManifest, ScenarioRunOptions, ScenarioRunResult,
    ScenarioStep, SelectorSpec, StepContext, StepOutcome, apply_fixture_state, load_fixture,
    parse_scenario_text, plan_fixture_state, plan_scenario, reset_fixture_state, run_scenario,
    validate_manifest,
};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::{Device, DeviceKind};
use androidforge_core::output::CommandSuccess;
use androidforge_core::run::RunContext;
use androidforge_diagnostics::capture_screenshot;
use androidforge_ui::Selector;
use androidforge_ui::color as ui_color;
use androidforge_ui::visual as ui_visual;
use serde_json::{Value, json};

use crate::cli::{Cli, ScenarioRunArgs, ScenarioValidateArgs};
use crate::commands::app::{self, InstallRequest, LaunchRequest};
use crate::commands::common::success;
use crate::context::{CliContext, Risk};
use crate::run_scope::RunScope;

/// Load a manifest, detecting JSON by extension *or* by a JSON object body.
/// The core parses `.json` as JSON and everything else as TOML; a file with a
/// TOML extension but a JSON body is re-parsed through a synthetic `.json`
/// path and the real source path is restored afterwards.
fn load_manifest(path: &Path) -> Result<ScenarioManifest> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        ForgeError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "could not read scenario manifest {}: {error}",
                path.display()
            ),
        )
    })?;
    let extension_json = path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("json"));
    let body_json = text.trim_start().starts_with('{');
    let parse_path = if body_json && !extension_json {
        Path::new("scenario.json")
    } else {
        path
    };
    let mut manifest = parse_scenario_text(parse_path, &text)?;
    manifest.source_path = Some(path.to_string_lossy().into_owned());
    Ok(manifest)
}

pub fn validate(args: &ScenarioValidateArgs) -> Result<CommandSuccess> {
    let manifest = load_manifest(&args.file)?;
    let plan = plan_scenario(&manifest);
    let value = json!({
        "valid": true,
        "name": manifest.name,
        "version": manifest.version,
        "source": manifest.source_path,
        "repeat": plan.repeat,
        "steps": plan.steps,
    });
    let human = format!(
        "Scenario '{}' is valid: {} step(s), repeat {}.\n",
        manifest.name,
        plan.steps.len(),
        plan.repeat
    );
    Ok(success("scenario.validate", value, human))
}

/// One compact NDJSON line per iteration. The full result stays in the final
/// summary envelope; these lines deliberately carry no duplicated result.
fn iteration_stream(result: &ScenarioRunResult) -> Vec<Value> {
    result
        .iterations
        .iter()
        .map(|iteration| {
            json!({
                "event": "iteration",
                "iteration": iteration.iteration,
                "status": if iteration.passed { "passed" } else { "failed" },
                "duration_ms": iteration.duration_ms,
                "steps": &iteration.steps,
            })
        })
        .collect()
}

fn with_jsonl_stream(
    response: CommandSuccess,
    cli: &Cli,
    result: &ScenarioRunResult,
) -> CommandSuccess {
    if cli.jsonl {
        response.with_stream(iteration_stream(result))
    } else {
        response
    }
}

pub async fn run(cli: &Cli, args: &ScenarioRunArgs) -> Result<CommandSuccess> {
    let mut manifest = load_manifest(&args.file)?;
    if let Some(count) = args.repeat {
        manifest.repeat.count = count;
        validate_manifest(&manifest)?;
    }

    // A dry run builds a plan only: no session, no device, no consent and no
    // executor dispatch. The no-op executor exists solely to satisfy the core
    // trait bound and is never called.
    if cli.dry_run {
        let options = ScenarioRunOptions {
            dry_run: true,
            trace: cli.trace,
            ..ScenarioRunOptions::default()
        };
        let result = run_scenario(&manifest, &options, &NoopExecutor).await?;
        let value = serde_json::to_value(&result)?;
        let human = format!(
            "Dry run for scenario '{}': {} iteration(s), {} step(s) planned; no device calls, no consent required.\n",
            result.name,
            result.iterations.len(),
            result.plan.steps.len()
        );
        let response = success("scenario.run", value, human);
        return Ok(with_jsonl_stream(response, cli, &result));
    }

    let ctx = CliContext::new(cli)?;
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let serial = device.serial.clone();

    // Explicit scenario consent: --yes confirms the run, while device
    // mutation is authorised by the emulator's disposable status, the master
    // flag or any narrow physical-scope flag. Per-step risk checks below still
    // enforce the exact scoped flag for each mutating operation.
    let allow_device_mutation = device.kind == DeviceKind::Emulator
        || cli.allow_physical_mutation
        || cli.allow_physical_app_control
        || cli.allow_physical_data
        || cli.allow_physical_network
        || cli.allow_physical_install
        || cli.allow_physical_input;
    let options = ScenarioRunOptions {
        dry_run: false,
        consent: ScenarioConsent {
            confirmed: cli.yes,
            allow_device_mutation,
        },
        trace: cli.trace,
    };

    // Create one run up front so every audited step can append to it. Nested
    // app launch/install calls join this run through a cloned context whose
    // `--run` is pinned to the same id.
    let scope = RunScope::begin(
        &ctx,
        "scenario.run",
        Some(&format!("scenario {}", manifest.name)),
        session.as_ref(),
        Some(&device),
        false,
    )
    .await?;
    let run_id = scope.run_id();
    let implicit = scope.implicit;
    drop(scope);

    let executor = CliExecutor {
        ctx: &ctx,
        device: &device,
        serial: &serial,
        run_id: &run_id,
    };
    let result = run_scenario(&manifest, &options, &executor).await?;
    record_skipped_steps(&ctx, &run_id, &result)?;

    // Finalize the run. A scenario with any failed step is a failed run;
    // nested per-step recording already attached each step and its artifacts.
    let mut run = RunContext::open(&ctx.paths.artifacts_dir, &run_id, ctx.redactor())?;
    if result.is_pass() {
        if implicit {
            run.finalize_pass()?;
        } else {
            run.save()?;
        }
    } else {
        let error = ForgeError::new(
            ErrorCode::UiAssertionFailed,
            format!(
                "scenario '{}' failed: {} of {} iteration(s) passed",
                result.name, result.summary.passed, result.summary.iterations
            ),
        );
        if implicit {
            run.finalize_failure(&error)?;
        } else {
            run.save()?;
        }
    }

    let value = serde_json::to_value(&result)?;
    let human = format!(
        "Scenario '{}': {} of {} iteration(s) passed ({:.1}%), {} flaky step(s), run {}.\n",
        result.name,
        result.summary.passed,
        result.summary.iterations,
        result.summary.pass_rate * 100.0,
        result.summary.flake.flaky_steps.len(),
        run_id
    );
    let mut response = success("scenario.run", value, human).with_run(run_id);
    response.device_serial = Some(serial);
    response.session_id = session.as_ref().map(|session| session.id.clone());
    if !result.is_pass() {
        response.status = "failed";
        response.exit_code = 1;
    }
    Ok(with_jsonl_stream(response, cli, &result))
}

/// Steps skipped by a `fail` policy never reach the executor, so record them
/// in the run report here to keep one run step per planned step.
fn record_skipped_steps(ctx: &CliContext, run_id: &str, result: &ScenarioRunResult) -> Result<()> {
    for iteration in &result.iterations {
        for record in &iteration.steps {
            if record.status != "skipped" {
                continue;
            }
            let mut run = RunContext::open(&ctx.paths.artifacts_dir, run_id, ctx.redactor())?;
            let index = run.start_step(
                format!(
                    "{}#{}:{:03} {}",
                    result.name,
                    iteration.iteration,
                    record.index + 1,
                    record.kind
                ),
                &record.audited_operation,
            )?;
            run.finish_step_skipped(index, "aborted by an earlier failed step (on_error = fail)")?;
        }
    }
    Ok(())
}

struct NoopExecutor;

impl ScenarioExecutor for NoopExecutor {
    fn execute<'a>(
        &'a self,
        _step: &'a ScenarioStep,
        _context: &'a StepContext,
    ) -> Pin<Box<dyn Future<Output = Result<StepOutcome>> + Send + 'a>> {
        Box::pin(async { Ok(StepOutcome::pass()) })
    }
}

fn to_selector(spec: &SelectorSpec) -> Selector {
    Selector {
        resource_id: spec.resource_id.clone(),
        text: spec.text.clone(),
        content_desc: spec.content_desc.clone(),
        class: spec.class.clone(),
        ..Selector::default()
    }
}

fn assertion_error(message: impl Into<String>) -> ForgeError {
    ForgeError::new(ErrorCode::UiAssertionFailed, message).with_suggestion(
        "Inspect the current screen with `androidforge ui observe --json` and adjust the scenario.",
    )
}

struct CliExecutor<'a> {
    ctx: &'a CliContext,
    device: &'a Device,
    serial: &'a str,
    run_id: &'a str,
}

impl CliExecutor<'_> {
    /// A context whose `--run` is pinned so launch/install add their audited
    /// steps to the same run instead of creating an implicit one.
    fn nested_ctx(&self) -> CliContext {
        let mut nested = self.ctx.clone();
        nested.run_arg = Some(self.run_id.to_string());
        nested
    }

    fn open_run(&self) -> Result<RunContext> {
        RunContext::open(
            &self.ctx.paths.artifacts_dir,
            self.run_id,
            self.ctx.redactor(),
        )
    }

    /// Dispatch one step to its audited operation. Returns the outcome, any
    /// artifacts to attach to the run step and the step's detail payload.
    async fn dispatch(
        &self,
        step: &ScenarioStep,
    ) -> (
        Result<StepOutcome>,
        Vec<androidforge_core::model::ArtifactRef>,
        Value,
    ) {
        let outcome: Result<StepOutcome> = match step {
            ScenarioStep::Launch {
                package, activity, ..
            } => {
                let request = LaunchRequest::simple(package.clone(), activity.clone(), false);
                app::launch_request(&self.nested_ctx(), &request)
                    .await
                    .map(|_| StepOutcome::pass())
            }
            ScenarioStep::Install { apk, .. } => {
                let request = InstallRequest {
                    apk: PathBuf::from(apk),
                    split: Vec::new(),
                    fresh: false,
                    downgrade: false,
                    no_verify: false,
                    expected_package: None,
                    expected_cert_sha256: None,
                    timeout: None,
                    only_if_changed: true,
                };
                app::install_request(&self.nested_ctx(), &request)
                    .await
                    .map(|_| StepOutcome::pass())
            }
            ScenarioStep::Tap { selector, .. } => {
                if let Err(error) =
                    self.ctx
                        .ensure_risk_allowed(self.device, Risk::Input, "injecting UI input")
                {
                    return (Err(error), Vec::new(), Value::Null);
                }
                let ui = match self.ctx.ui_manager() {
                    Ok(ui) => ui,
                    Err(error) => return (Err(error), Vec::new(), Value::Null),
                };
                ui.tap(self.serial, &to_selector(selector))
                    .await
                    .map(|_| StepOutcome::pass())
            }
            ScenarioStep::Text {
                selector, value, ..
            } => {
                if let Err(error) =
                    self.ctx
                        .ensure_risk_allowed(self.device, Risk::Input, "injecting UI input")
                {
                    return (Err(error), Vec::new(), Value::Null);
                }
                let ui = match self.ctx.ui_manager() {
                    Ok(ui) => ui,
                    Err(error) => return (Err(error), Vec::new(), Value::Null),
                };
                ui.text(
                    self.serial,
                    value,
                    Some(&to_selector(selector)),
                    false,
                    false,
                )
                .await
                .map(|_| StepOutcome::pass())
            }
            ScenarioStep::Wait {
                selector,
                timeout_ms,
                ..
            } => {
                let timeout = Duration::from_millis(timeout_ms.unwrap_or_else(|| {
                    self.ctx
                        .config
                        .file
                        .ui
                        .default_timeout_secs
                        .saturating_mul(1000)
                }));
                let ui = match self.ctx.ui_manager() {
                    Ok(ui) => ui,
                    Err(error) => return (Err(error), Vec::new(), Value::Null),
                };
                ui.wait(self.serial, &to_selector(selector), timeout, false)
                    .await
                    .map(|_| StepOutcome::pass())
            }
            ScenarioStep::Assert { expect, .. } => self
                .check_assertions(expect, Duration::from_secs(1))
                .await
                .map(|_| StepOutcome::pass()),
            ScenarioStep::Bounds {
                selector,
                relation,
                expected,
                tolerance,
                ..
            } => {
                let ui = match self.ctx.ui_manager() {
                    Ok(ui) => ui,
                    Err(error) => return (Err(error), Vec::new(), Value::Null),
                };
                let selector = to_selector(selector);
                let result = match relation {
                    BoundsRelation::BottomYEq => {
                        ui.assert_bottom_y(self.serial, &selector, *expected, *tolerance)
                            .await
                    }
                    BoundsRelation::BottomYGt => {
                        ui.assert_bottom_y_gt(self.serial, &selector, *expected, *tolerance)
                            .await
                    }
                    BoundsRelation::BottomYLt => {
                        ui.assert_bottom_y_lt(self.serial, &selector, *expected, *tolerance)
                            .await
                    }
                };
                result.map(|_| StepOutcome::pass())
            }
            ScenarioStep::Color { expect, .. } => {
                self.check_color(expect).await.map(|_| StepOutcome::pass())
            }
            ScenarioStep::Screenshot { name, .. } => {
                let mut run = match self.open_run() {
                    Ok(run) => run,
                    Err(error) => return (Err(error), Vec::new(), Value::Null),
                };
                let adb = match self.ctx.adb() {
                    Ok(adb) => adb,
                    Err(error) => return (Err(error), Vec::new(), Value::Null),
                };
                let timeout =
                    Duration::from_secs(self.ctx.config.file.timeouts.screenshot_secs.max(1));
                match capture_screenshot(&adb, self.serial, &mut run, Some(name), timeout).await {
                    Ok(result) => {
                        let details = serde_json::to_value(&result).unwrap_or(Value::Null);
                        return (Ok(StepOutcome::pass()), vec![result.artifact], details);
                    }
                    Err(error) => return (Err(error), Vec::new(), Value::Null),
                }
            }
            ScenarioStep::FixtureApply { name, .. } | ScenarioStep::FixtureReset { name, .. } => {
                match self.fixture_state(step, name).await {
                    Ok(details) => return (Ok(StepOutcome::pass()), Vec::new(), details),
                    Err(error) => return (Err(error), Vec::new(), Value::Null),
                }
            }
        };
        (outcome, Vec::new(), Value::Null)
    }

    async fn check_assertions(&self, expect: &AssertSpec, timeout: Duration) -> Result<()> {
        let ui = self.ctx.ui_manager()?;
        if let Some(text) = &expect.text {
            let selector = Selector {
                text: Some(text.clone()),
                ..Selector::default()
            };
            ui.assert(self.serial, &selector, timeout, false).await?;
        }
        if let Some(text) = &expect.text_absent {
            let result = ui
                .wait_texts_absent(self.serial, std::slice::from_ref(text), timeout)
                .await?;
            if !result.absent {
                return Err(assertion_error(format!(
                    "expected text {:?} to be absent, but it is still visible",
                    text
                )));
            }
        }
        if expect.package.is_some() || expect.activity.is_some() {
            let hierarchy = ui.dump(self.serial).await?;
            let focus = self.ctx.adb()?.current_focus(self.serial).await;
            let package = focus
                .as_ref()
                .map(|focus| focus.package.clone())
                .or_else(|| hierarchy.package_name.clone());
            let activity = focus.as_ref().and_then(|focus| focus.activity.clone());
            if let Some(expected) = &expect.package
                && package.as_deref() != Some(expected.as_str())
            {
                return Err(assertion_error(format!(
                    "expected foreground package {:?}, found {:?}",
                    expected, package
                )));
            }
            if let Some(expected) = &expect.activity
                && !activity
                    .as_deref()
                    .is_some_and(|value| value.contains(expected.as_str()))
            {
                return Err(assertion_error(format!(
                    "expected foreground activity containing {:?}, found {:?}",
                    expected, activity
                )));
            }
        }
        Ok(())
    }

    async fn check_color(&self, expect: &ColorExpectation) -> Result<()> {
        let expected = ui_color::parse_hex(&expect.expected)?;
        let timeout = Duration::from_secs(self.ctx.config.file.timeouts.screenshot_secs.max(1));
        let adb = self.ctx.adb()?;
        let output = adb
            .exec_out_args(
                self.serial,
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
            .device(self.serial.to_string()));
        }
        let image = ui_visual::decode_reader(Cursor::new(&output.stdout), "scenario colour check")?;
        let comparison =
            ui_color::check_pixel(&image, expect.x, expect.y, expected, expect.tolerance)?;
        if !comparison.passed {
            return Err(assertion_error(format!(
                "colour at ({}, {}) was {} (expected {}, delta {}, tolerance {})",
                expect.x,
                expect.y,
                comparison.actual.hex(),
                comparison.expected.hex(),
                comparison.max_channel_delta,
                comparison.tolerance
            )));
        }
        Ok(())
    }

    async fn fixture_state(&self, step: &ScenarioStep, name: &str) -> Result<Value> {
        let is_apply = matches!(step, ScenarioStep::FixtureApply { .. });
        self.ctx
            .ensure_risk_allowed(self.device, Risk::Data, "running a fixture")?;
        let session = self.ctx.require_session()?;
        let device = self.ctx.resolve_device(Some(&session)).await?;
        let serial = session.serial()?.to_string();
        let manifest = load_fixture(&self.ctx.config, name)?;
        let declared = manifest.state.as_ref().ok_or_else(|| {
            ForgeError::new(
                ErrorCode::FixtureInvalid,
                format!("fixture '{}' has no declarative state block", manifest.name),
            )
            .with_suggestion("Run `androidforge fixture validate <name>` to inspect the pack.")
        })?;
        let declared_package =
            manifest
                .steps
                .iter()
                .find_map(|fixture_step| match (&fixture_step.action, is_apply) {
                    (FixtureAction::ApplyState { package }, true) => package.clone(),
                    (FixtureAction::ResetState { package }, false) => package.clone(),
                    _ => None,
                });
        let package = declared_package
            .or_else(|| self.ctx.config.debug_package().map(str::to_string))
            .or_else(|| self.ctx.config.package().map(str::to_string))
            .ok_or_else(|| {
                ForgeError::new(
                    ErrorCode::FixtureInvalid,
                    "fixture state needs a package and the project adapter has none",
                )
                .with_suggestion("Set [project].package in .androidforge/project.toml.")
            })?;
        let plan = plan_fixture_state(
            declared,
            &std::collections::BTreeMap::new(),
            &self.ctx.paths.project_root,
        )?;
        let allow_data_mutation =
            device.kind == DeviceKind::Emulator || self.ctx.risk_allowed(Risk::Data);
        let timeout = Duration::from_secs(self.ctx.config.file.timeouts.fixture_secs.max(1));

        let mut run = self.open_run()?;
        let step_index = run.start_step(
            format!("fixture:{}", if is_apply { "apply" } else { "reset" }),
            step.audited_operation().as_str(),
        )?;
        let app = self.ctx.app_manager()?;
        let execution = FixtureStateExecution {
            app: &app,
            serial: &serial,
            package: &package,
            cwd: &self.ctx.paths.project_root,
            timeout,
            trace: self.ctx.trace,
        };
        let outcome = if is_apply {
            apply_fixture_state(&execution, &plan, allow_data_mutation, &mut run, step_index).await
        } else {
            reset_fixture_state(&execution, &plan, allow_data_mutation, &mut run, step_index).await
        };
        let outcome = outcome.map_err(|error| {
            let _ = run.finish_step_failure(step_index, &error, Vec::new());
            error
                .with_context_string("fixture", manifest.name.clone())
                .with_context_string("action", if is_apply { "apply" } else { "reset" })
        })?;
        let details = serde_json::to_value(&outcome).unwrap_or(Value::Null);
        run.finish_step_success(step_index, details.clone(), Vec::new())?;
        Ok(details)
    }

    /// Record one step attempt in the run report. Launch/install already add
    /// their own audited step through `execute_in_run!`, so they are skipped.
    fn record_step(
        &self,
        step: &ScenarioStep,
        sctx: &StepContext,
        outcome: &Result<StepOutcome>,
        artifacts: Vec<androidforge_core::model::ArtifactRef>,
        details: Value,
    ) -> Result<()> {
        // Launch/install and fixture steps add their own audited run step
        // through the audited operation (which already receives a `RunContext`).
        if matches!(
            step,
            ScenarioStep::Launch { .. }
                | ScenarioStep::Install { .. }
                | ScenarioStep::FixtureApply { .. }
                | ScenarioStep::FixtureReset { .. }
        ) {
            return Ok(());
        }
        let mut run = self.open_run()?;
        let name = format!(
            "{}#{}:{:03} {}",
            sctx.scenario,
            sctx.iteration,
            sctx.step_index + 1,
            step.describe()
        );
        let index = run.start_step(name, step.audited_operation().as_str())?;
        match outcome {
            Ok(outcome) if outcome.passed => {
                let mut details = details;
                if let Some(object) = details.as_object_mut() {
                    object.insert(
                        "step_kind".to_string(),
                        Value::String(step.kind().to_string()),
                    );
                    object.insert(
                        "on_error".to_string(),
                        Value::String(step.on_error().as_str().to_string()),
                    );
                }
                run.finish_step_success(index, details, artifacts)?;
            }
            Ok(outcome) => {
                let error = ForgeError::new(
                    ErrorCode::UiAssertionFailed,
                    outcome
                        .error_message
                        .clone()
                        .unwrap_or_else(|| format!("scenario step '{}' failed", step.kind())),
                );
                run.finish_step_failure(index, &error, artifacts)?;
            }
            Err(error) => run.finish_step_failure(index, error, artifacts)?,
        }
        Ok(())
    }
}

impl ScenarioExecutor for CliExecutor<'_> {
    fn execute<'a>(
        &'a self,
        step: &'a ScenarioStep,
        context: &'a StepContext,
    ) -> Pin<Box<dyn Future<Output = Result<StepOutcome>> + Send + 'a>> {
        Box::pin(async move {
            let started = Instant::now();
            let (outcome, artifacts, details) = self.dispatch(step).await;
            let recording = self.record_step(step, context, &outcome, artifacts, details);
            match outcome {
                Ok(mut outcome) => {
                    outcome.duration_ms =
                        started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                    recording?;
                    Ok(outcome)
                }
                Err(error) => Err(error),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use androidforge_automation::UNSAFE_STEP_KINDS;

    #[test]
    fn selector_spec_maps_every_condition_to_the_audited_selector() {
        let spec = SelectorSpec {
            resource_id: Some("com.example:id/save".to_string()),
            text: Some("Save".to_string()),
            content_desc: Some("Save changes".to_string()),
            class: Some("android.widget.Button".to_string()),
        };
        let selector = to_selector(&spec);
        assert_eq!(selector.resource_id.as_deref(), Some("com.example:id/save"));
        assert_eq!(selector.text.as_deref(), Some("Save"));
        assert_eq!(selector.content_desc.as_deref(), Some("Save changes"));
        assert_eq!(selector.class.as_deref(), Some("android.widget.Button"));
        assert!(to_selector(&SelectorSpec::default()).text.is_none());
    }

    #[test]
    fn every_allowlisted_step_has_an_audited_operation_and_no_unsafe_kind() {
        let steps = [
            (serde_json::from_value::<ScenarioStep>(json!({"type":"launch","package":"com.example"})).unwrap(), "app.launch"),
            (serde_json::from_value::<ScenarioStep>(json!({"type":"tap","selector":{"text":"Save"}})).unwrap(), "ui.tap"),
            (serde_json::from_value::<ScenarioStep>(json!({"type":"text","selector":{"text":"Email"},"value":"a@b.c"})).unwrap(), "ui.text"),
            (serde_json::from_value::<ScenarioStep>(json!({"type":"wait","selector":{"text":"Save"}})).unwrap(), "ui.wait"),
            (serde_json::from_value::<ScenarioStep>(json!({"type":"assert","expect":{"text":"Saved"}})).unwrap(), "ui.assert"),
            (serde_json::from_value::<ScenarioStep>(json!({"type":"bounds","selector":{"text":"Save"},"relation":"bottom_y_eq","expected":300})).unwrap(), "ui.bounds"),
            (serde_json::from_value::<ScenarioStep>(json!({"type":"color","expect":{"x":1,"y":2,"expected":"#FFFFFF"}})).unwrap(), "ui.color_check"),
            (serde_json::from_value::<ScenarioStep>(json!({"type":"screenshot","name":"home"})).unwrap(), "ui.screenshot"),
            (serde_json::from_value::<ScenarioStep>(json!({"type":"fixture_apply","name":"state"})).unwrap(), "fixture.apply"),
            (serde_json::from_value::<ScenarioStep>(json!({"type":"fixture_reset","name":"state"})).unwrap(), "fixture.reset"),
            (serde_json::from_value::<ScenarioStep>(json!({"type":"install","apk":"app.apk"})).unwrap(), "app.install"),
        ];
        for (step, operation) in steps {
            assert_eq!(step.audited_operation().as_str(), operation);
            assert!(
                !UNSAFE_STEP_KINDS.contains(&step.kind()),
                "{} must never be allowlisted",
                step.kind()
            );
        }
    }
}
