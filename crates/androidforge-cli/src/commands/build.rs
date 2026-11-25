//! Host-side build engine MVP.
//!
//! This module deliberately has no CLI, session or device coupling. Build
//! commands come only from `[build.commands.<variant>]` as explicit argv
//! arrays; no shell string is interpreted and nothing is inferred. Execution
//! uses `androidforge_process`, which bounds capture, applies a timeout and
//! terminates the whole process tree on expiry.
//!
//! `Command::Build` wires this engine into the CLI. Build alone is host-side;
//! `--install`, `--launch` and `--cold` reuse the verified app paths and their
//! physical-device gates without duplicating install/launch logic.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use androidforge_core::config::{ForgeConfig, Severity};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::util::{
    ensure_dir, new_run_token, now_rfc3339, parse_duration, sha256_hex, slugify, write_atomic,
};
use androidforge_process::{CommandOutput, CommandSpec, Runner, SPAWN_FAILURE_EXIT_CODE};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cli::{BuildArgs, Cli};
use crate::commands::apk::{ResolvedApk, resolve_variant_with};
use crate::commands::app::{self, InstallRequest, LaunchRequest};
use crate::context::CliContext;

/// Hard cap on the redacted build log artifact. The runner already bounds each
/// stream; this second bound keeps the combined artifact predictable.
pub const BUILD_LOG_MAX_BYTES: usize = 256 * 1024;
const FIRST_ERROR_MAX_CHARS: usize = 500;

/// Caller intent. Nothing here is read from devices or sessions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildRequest {
    /// Variant to build. Falls back to `[build].default_variant`.
    pub variant: Option<String>,
    /// Optional module filter used when resolving the produced APK.
    pub module: Option<String>,
    /// Overrides `[build].timeout_secs` for this invocation.
    pub timeout: Option<Duration>,
    /// If set, the build is planned but no process is started and no file is
    /// written. The global `androidforge --dry-run` flag is honoured too.
    pub dry_run: bool,
    /// When present, the resolved APK package must match exactly.
    pub expected_package: Option<String>,
}

/// Fully resolved, typed execution plan. Contains environment *names* only;
/// values are read from the host environment when the command is launched and
/// are never copied into results, logs or error context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOptions {
    pub variant: String,
    /// Absolute working directory inside the project.
    pub cwd: PathBuf,
    pub timeout: Duration,
    pub env_names: Vec<String>,
    pub dry_run: bool,
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildStatus {
    Success,
    Failed,
    DryRun,
}

impl BuildStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::DryRun => "dry_run",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildLog {
    /// Absolute path of the persisted, redacted log artifact.
    pub path: String,
    /// SHA-256 of the exact bytes written to `path`.
    pub sha256: String,
    pub bytes: u64,
    pub truncated: bool,
    pub omitted_bytes: u64,
}

/// Stable, machine-readable failure details. Present on every failed outcome
/// and mirrored into [`ForgeError`] context so CLI/MCP callers do not need to
/// scrape free-form log text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildFailure {
    pub code: ErrorCode,
    /// Stable classification, for example `gradle_task_failed`, `timeout`,
    /// `toolchain_incompatible`, `non_zero` or `artifact_not_found`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_sha256: Option<String>,
    pub duration_ms: u64,
    pub argv: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildOutcome {
    pub schema_version: u32,
    pub status: BuildStatus,
    pub variant: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// Exact argv that was (or would be) executed.
    pub argv: Vec<String>,
    pub cwd: String,
    pub timeout_ms: u64,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<BuildLog>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ResolvedApk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<BuildFailure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl BuildOutcome {
    fn dry_run(options: &BuildOptions, module: Option<&str>) -> Self {
        Self {
            schema_version: 1,
            status: BuildStatus::DryRun,
            variant: options.variant.clone(),
            module: module.map(str::to_string),
            argv: options.argv.clone(),
            cwd: options.cwd.to_string_lossy().into_owned(),
            timeout_ms: options.timeout.as_millis().min(u128::from(u64::MAX)) as u64,
            duration_ms: 0,
            exit_code: None,
            timed_out: false,
            log: None,
            artifact: None,
            failure: None,
            warnings: Vec::new(),
        }
    }

    /// Convert a failed outcome into the error code and structured context
    /// expected by CLI, MCP and run reporting.
    pub fn into_error(self) -> ForgeError {
        let failure = self.failure.clone().unwrap_or_else(|| BuildFailure {
            code: ErrorCode::BuildFailed,
            kind: "unknown".to_string(),
            failed_task: None,
            first_error: None,
            log_path: None,
            log_sha256: None,
            duration_ms: self.duration_ms,
            argv: self.argv.clone(),
            exit_code: self.exit_code,
            timed_out: self.timed_out,
        });
        let message = if failure.code == ErrorCode::BuildOutputTooLarge {
            format!(
                "build output for variant '{}' exceeded the capture limit and could not be classified",
                self.variant
            )
        } else {
            match failure.kind.as_str() {
                "timeout" => format!(
                    "build for variant '{}' did not finish within {} ms",
                    self.variant, self.timeout_ms
                ),
                "artifact_not_found" => format!(
                    "no built APK was found for variant '{}' after a successful build",
                    self.variant
                ),
                "artifact_ambiguous" => format!(
                    "multiple APK candidates for variant '{}' could not be disambiguated",
                    self.variant
                ),
                "identity_mismatch" => format!(
                    "built APK identity did not match the expected package for variant '{}'",
                    self.variant
                ),
                "spawn_failed" => format!(
                    "could not start build command for variant '{}'",
                    self.variant
                ),
                "gradle_task_failed" | "gradle_failed" | "toolchain_incompatible" | "non_zero" => {
                    format!(
                        "build command for variant '{}' exited with code {}",
                        self.variant,
                        failure
                            .exit_code
                            .map(|code| code.to_string())
                            .unwrap_or_else(|| "unknown".to_string())
                    )
                }
                other => format!("build failed for variant '{}' ({other})", self.variant),
            }
        };
        let mut error = ForgeError::new(failure.code, message)
            .with_context(
                "build_outcome",
                serde_json::to_value(&self).unwrap_or_default(),
            )
            .with_context("build_status", serde_json::json!(self.status.as_str()))
            .with_context("variant", serde_json::json!(self.variant))
            .with_context("failed_task", serde_json::json!(failure.failed_task))
            .with_context("first_error", serde_json::json!(failure.first_error))
            .with_context("log_path", serde_json::json!(failure.log_path))
            .with_context("log_sha256", serde_json::json!(failure.log_sha256))
            .with_context("duration_ms", serde_json::json!(failure.duration_ms))
            .with_context("argv", serde_json::json!(failure.argv))
            .with_context("exit_code", serde_json::json!(failure.exit_code))
            .with_context("classification", serde_json::json!(failure.kind));
        for warning in &self.warnings {
            error = error.with_suggestion(warning.clone());
        }
        error = error.with_suggestions(if failure.code == ErrorCode::BuildOutputTooLarge {
            vec![
                "Reduce build output verbosity, or write build output to a file and print a short summary."
                    .to_string(),
                "Inspect the retained head and tail of the redacted build log at log_path."
                    .to_string(),
            ]
        } else {
            match failure.kind.as_str() {
                "timeout" => vec![
                    "Check for orphaned build processes before retrying.".to_string(),
                    "Retry with a longer timeout through [build].timeout_secs or BuildRequest.timeout."
                        .to_string(),
                ],
                "artifact_not_found" => vec![
                    "Run `androidforge apk resolve --variant <variant>` after a successful build."
                        .to_string(),
                    "Check [build.variants] patterns and the configured build.cwd.".to_string(),
                ],
                "artifact_ambiguous" => {
                    vec!["Configure one explicit APK output pattern for this variant.".to_string()]
                }
                _ => {
                    vec!["Inspect the redacted build log artifact referenced by log_path.".to_string()]
                }
            }
        });
        error
    }

    /// Short human-readable summary used by the CLI.
    pub fn human(&self) -> String {
        let mut out = format!(
            "build {} {}\nstatus: {}\ncommand: {}\ncwd: {}\nduration: {}ms\n",
            self.variant,
            self.module
                .as_deref()
                .map(|module| format!("({module})"))
                .unwrap_or_default(),
            self.status.as_str(),
            self.argv.join(" "),
            self.cwd,
            self.duration_ms,
        );
        if let Some(log) = &self.log {
            out.push_str(&format!(
                "log: {} (sha256 {}, {} bytes{})\n",
                log.path,
                log.sha256,
                log.bytes,
                if log.truncated {
                    format!(", truncated, {} bytes omitted", log.omitted_bytes)
                } else {
                    String::new()
                }
            ));
        }
        if let Some(artifact) = &self.artifact {
            out.push_str(&format!(
                "apk: {}\npackage: {}\nversion: {} ({})\n",
                artifact.path,
                artifact.package.as_deref().unwrap_or("-"),
                artifact.version_name.as_deref().unwrap_or("-"),
                artifact
                    .version_code
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ));
        }
        if let Some(failure) = &self.failure {
            out.push_str(&format!(
                "failure: {} (exit {:?})\n",
                failure.kind, failure.exit_code
            ));
            if let Some(task) = &failure.failed_task {
                out.push_str(&format!("failed task: {task}\n"));
            }
            if let Some(error) = &failure.first_error {
                out.push_str(&format!("first error: {error}\n"));
            }
        }
        for warning in &self.warnings {
            out.push_str(&format!("warning: {warning}\n"));
        }
        out
    }
}

/// CLI entry point. Build alone only touches host state; install and launch
/// reuse `app::install_request` / `app::launch_request` so identity checks and
/// physical-device gates stay single-sourced.
pub async fn build(cli: &Cli, args: &BuildArgs) -> Result<CommandSuccess> {
    let ctx = CliContext::new(cli)?;
    if args.fresh && args.only_if_changed {
        return Err(ForgeError::invalid_argument(
            "--fresh cannot be combined with --only-if-changed",
        )
        .with_suggestion("A fresh install always replaces the app and deletes its data."));
    }
    if args.activity.is_some() && !(args.launch || args.cold) {
        return Err(ForgeError::invalid_argument(
            "--activity requires --launch or --cold",
        ));
    }
    let timeout = match &args.timeout {
        Some(value) => Some(
            parse_duration(value)
                .map_err(ForgeError::invalid_argument)
                .map_err(|error| error.with_context_string("argument", "--timeout".to_string()))?,
        ),
        None => None,
    };
    let request = BuildRequest {
        variant: args.variant.clone(),
        module: args.module.clone(),
        timeout,
        dry_run: cli.dry_run,
        expected_package: args.expected_package.clone(),
    };
    let outcome = run_build(&ctx, &request).await?;
    let mut result = json!({ "build": serde_json::to_value(&outcome)? });
    let mut human = outcome.human();
    if let Some(artifact) = &outcome.artifact {
        result["apk"] = serde_json::to_value(artifact)?;
    }

    if !(args.install || args.launch || args.cold) {
        return Ok(build_success(
            &outcome,
            result,
            human,
            Vec::new(),
            None,
            None,
        ));
    }

    // Global --dry-run never resolves a session, an ADB device or a built APK.
    // The build engine itself is already a no-op in this mode.
    if cli.dry_run {
        result["install"] = json!({
            "dry_run": true,
            "planned": true,
            "install": {
                "fresh": args.fresh,
                "allow_downgrade": args.downgrade,
                "verify_installed": !args.no_verify,
                "expected_package": args.expected_package,
                "only_if_changed": args.only_if_changed,
            },
            "session_or_device": "not resolved during dry-run",
        });
        if args.launch || args.cold {
            result["launch"] = json!({
                "dry_run": true,
                "planned": true,
                "package": args.expected_package,
                "activity": args.activity,
                "cold": args.cold,
                "session_or_device": "not resolved during dry-run",
            });
        }
        human.push_str("dry-run: no session, device, install or launch was executed.\n");
        return Ok(build_success(
            &outcome,
            result,
            human,
            Vec::new(),
            None,
            None,
        ));
    }

    let artifact = outcome.artifact.as_ref().ok_or_else(|| {
        ForgeError::new(
            ErrorCode::BuildArtifactNotFound,
            "no built APK was resolved; cannot install",
        )
        .with_context("build_status", json!(outcome.status.as_str()))
    })?;
    let install_request = InstallRequest {
        apk: PathBuf::from(&artifact.path),
        split: artifact
            .splits
            .iter()
            .map(|split| PathBuf::from(&split.path))
            .collect(),
        fresh: args.fresh,
        downgrade: args.downgrade,
        no_verify: args.no_verify,
        expected_package: args
            .expected_package
            .clone()
            .or_else(|| artifact.package.clone()),
        // Keep the project's configured certificate policy rather than
        // replacing it with a certificate observed from the same artifact.
        expected_cert_sha256: None,
        timeout: None,
        only_if_changed: args.only_if_changed,
    };
    let install_success = app::install_request(&ctx, &install_request).await?;
    result["install"] = install_success.result.clone();
    human.push_str(&install_success.human);
    let mut artifacts = install_success.artifacts.clone();
    let session_id = install_success.session_id.clone();
    let device = install_success.device_serial.clone();

    if args.launch || args.cold {
        let package = install_success
            .result
            .get("build")
            .and_then(|build| build.get("package"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| artifact.package.clone())
            .or_else(|| args.expected_package.clone())
            .ok_or_else(|| {
                ForgeError::new(
                    ErrorCode::BuildIdentityMismatch,
                    "the built APK declares no package identity; refusing to launch",
                )
                .with_suggestion("Configure [project].package or pass --expected-package.")
            })?;
        let launch_success = app::launch_request(
            &ctx,
            &LaunchRequest::simple(package, args.activity.clone(), args.cold),
        )
        .await?;
        result["launch"] = launch_success.result.clone();
        human.push_str(&launch_success.human);
        artifacts.extend(launch_success.artifacts.iter().cloned());
    }

    Ok(build_success(
        &outcome, result, human, artifacts, session_id, device,
    ))
}

fn build_success(
    outcome: &BuildOutcome,
    result: Value,
    human: String,
    artifacts: Vec<androidforge_core::model::ArtifactRef>,
    session_id: Option<String>,
    device: Option<String>,
) -> CommandSuccess {
    CommandSuccess::new("build", result, human)
        .with_duration(outcome.duration_ms)
        .with_artifacts(artifacts)
        .with_optional_session(session_id)
        .with_optional_device(device)
}

/// Resolve a typed plan without executing anything. Config errors are rejected
/// here using the exact build-related validators from `ForgeConfig::validate`.
pub fn resolve_options(ctx: &CliContext, request: &BuildRequest) -> Result<BuildOptions> {
    reject_invalid_build_config(&ctx.config)?;
    let build = &ctx.config.file.build;
    let variant = request
        .variant
        .clone()
        .or_else(|| build.default_variant.clone())
        .ok_or_else(|| {
            ForgeError::new(
                ErrorCode::BuildCommandNotConfigured,
                "no build variant was specified",
            )
            .with_suggestion("Pass a variant in the build request or set [build].default_variant.")
        })?;
    let argv = resolve_command(&build.commands, &variant)
        .ok_or_else(|| {
            ForgeError::new(
                ErrorCode::BuildCommandNotConfigured,
                format!("no command is configured for build variant '{variant}'"),
            )
            .with_context("variant", serde_json::json!(variant))
            .with_context("configured_variants", serde_json::json!(build.commands.keys().collect::<Vec<_>>()))
            .with_suggestion(format!(
                "Add [build.commands.{variant}] as an explicit argv array in .androidforge/project.toml."
            ))
            .with_suggestion("AndroidForge never infers or shells out to a build command.")
        })?
        .to_vec();
    if argv.is_empty() || argv.iter().any(String::is_empty) {
        return Err(ForgeError::new(
            ErrorCode::ConfigInvalid,
            format!("[build.commands.{variant}] must be a non-empty argv array"),
        ));
    }

    let cwd = match &build.cwd {
        Some(relative) => {
            let candidate = ctx.paths.project_root.join(relative);
            let root = std::fs::canonicalize(&ctx.paths.project_root)
                .unwrap_or_else(|_| ctx.paths.project_root.clone());
            let resolved = std::fs::canonicalize(&candidate).unwrap_or(candidate);
            if !resolved.starts_with(&root) {
                return Err(ForgeError::new(
                    ErrorCode::ConfigInvalid,
                    format!("build.cwd '{relative}' escapes the project root"),
                )
                .with_context("cwd", serde_json::json!(resolved.to_string_lossy())));
            }
            resolved
        }
        None => ctx.paths.project_root.clone(),
    };

    let timeout = request
        .timeout
        .or_else(|| build.timeout_secs.map(Duration::from_secs))
        .unwrap_or_else(|| Duration::from_secs(ctx.config.file.timeouts.command_secs.max(1)));
    if timeout.is_zero() {
        return Err(ForgeError::invalid_argument(
            "build timeout must be greater than zero",
        ));
    }

    let mut seen = BTreeSet::new();
    let env_names = build
        .env_passthrough
        .iter()
        .filter(|name| seen.insert((*name).clone()))
        .cloned()
        .collect();

    Ok(BuildOptions {
        variant,
        cwd,
        timeout,
        env_names,
        dry_run: request.dry_run || ctx.dry_run,
        argv,
    })
}

/// Execute a configured build. A successful build resolves its APK through the
/// same deterministic logic as `apk resolve`; any failure is returned as a
/// `ForgeError` with stable structured context and a persisted redacted log.
pub async fn run_build(ctx: &CliContext, request: &BuildRequest) -> Result<BuildOutcome> {
    let options = resolve_options(ctx, request)?;
    if options.dry_run {
        return Ok(BuildOutcome::dry_run(&options, request.module.as_deref()));
    }

    let started = Instant::now();
    let spec = command_spec(&options, ctx.trace);
    let output = match Runner::new(ctx.trace).run(spec).await {
        Ok(output) => output,
        Err(error) => {
            let mut outcome = BuildOutcome::dry_run(&options, request.module.as_deref());
            outcome.status = BuildStatus::Failed;
            outcome.duration_ms = elapsed_ms(started);
            let log = write_build_log(ctx, &options, &error.to_string(), None)?;
            outcome.log = Some(log.clone());
            outcome.failure = Some(BuildFailure {
                code: ErrorCode::BuildFailed,
                kind: "spawn_failed".to_string(),
                failed_task: None,
                first_error: Some(redact_line(ctx, &error.to_string())),
                log_path: Some(log.path),
                log_sha256: Some(log.sha256),
                duration_ms: outcome.duration_ms,
                argv: options.argv.clone(),
                exit_code: Some(SPAWN_FAILURE_EXIT_CODE),
                timed_out: false,
            });
            return Err(outcome.into_error());
        }
    };

    let parsed = parse_failure(&output);
    let first_error = parsed
        .first_error
        .as_deref()
        .map(|line| redact_line(ctx, line));
    let log = write_build_log(
        ctx,
        &options,
        &output.combined_text(),
        first_error.as_deref(),
    )?;
    let duration_ms = output.duration_ms.max(elapsed_ms(started));
    let mut warnings = Vec::new();
    if log.truncated {
        warnings.push(format!(
            "Build log exceeded {} bytes; head and tail were preserved and {} bytes were omitted.",
            BUILD_LOG_MAX_BYTES, log.omitted_bytes
        ));
    }
    // Decision: E_BUILD_OUTPUT_TOO_LARGE is *not* emitted for truncation by
    // itself. It is reserved for the case where the bounded capture removed
    // the only failure marker, leaving a generic non-zero with no usable
    // classification (no failed task and no first error). Otherwise the
    // persisted head/tail log plus a warning is sufficient.
    let capture_truncated = output.stdout_truncated || output.stderr_truncated;
    let classification_usable =
        parsed.failed_task.is_some() || first_error.is_some() || parsed.kind != "non_zero";
    if capture_truncated {
        warnings.push(
            "Build output exceeded the bounded capture limit; head and tail were retained."
                .to_string(),
        );
    }

    let mut outcome = BuildOutcome {
        schema_version: 1,
        status: BuildStatus::Success,
        variant: options.variant.clone(),
        module: request.module.clone(),
        argv: options.argv.clone(),
        cwd: options.cwd.to_string_lossy().into_owned(),
        timeout_ms: options.timeout.as_millis().min(u128::from(u64::MAX)) as u64,
        duration_ms,
        exit_code: output.exit_code,
        timed_out: output.timed_out,
        log: Some(log.clone()),
        artifact: None,
        failure: None,
        warnings: warnings.clone(),
    };

    if output.timed_out || !output.success {
        let kind = if output.timed_out {
            "timeout"
        } else {
            parsed.kind
        };
        let code = if output.timed_out {
            ErrorCode::BuildTimeout
        } else if capture_truncated && !classification_usable {
            ErrorCode::BuildOutputTooLarge
        } else {
            ErrorCode::BuildFailed
        };
        outcome.status = BuildStatus::Failed;
        outcome.failure = Some(BuildFailure {
            code,
            kind: kind.to_string(),
            failed_task: parsed.failed_task.map(|task| redact_line(ctx, &task)),
            first_error,
            log_path: Some(log.path),
            log_sha256: Some(log.sha256),
            duration_ms,
            argv: options.argv.clone(),
            exit_code: output.exit_code,
            timed_out: output.timed_out,
        });
        return Err(outcome.into_error());
    }

    let resolution = match resolve_variant_with(
        &ctx.config.file,
        &ctx.paths.project_root,
        &ctx.inspector(),
        &options.variant,
        request.module.as_deref(),
    )
    .await
    {
        Ok(resolution) => resolution,
        Err(error) => {
            let kind = artifact_error_kind(&error);
            let code = match kind {
                "artifact_ambiguous" => ErrorCode::BuildArtifactAmbiguous,
                "identity_mismatch" => ErrorCode::BuildIdentityMismatch,
                _ => ErrorCode::BuildArtifactNotFound,
            };
            outcome.status = BuildStatus::Failed;
            outcome
                .warnings
                .extend(error.body().suggestions.iter().cloned());
            outcome.failure = Some(BuildFailure {
                code,
                kind: kind.to_string(),
                failed_task: None,
                first_error: Some(redact_line(ctx, error.message())),
                log_path: Some(log.path),
                log_sha256: Some(log.sha256),
                duration_ms,
                argv: options.argv.clone(),
                exit_code: output.exit_code,
                timed_out: false,
            });
            return Err(outcome.into_error());
        }
    };
    outcome.warnings.extend(resolution.warnings.iter().cloned());

    if let Some(expected) = &request.expected_package
        && resolution.selected.package.as_deref() != Some(expected.as_str())
    {
        outcome.status = BuildStatus::Failed;
        outcome.failure = Some(BuildFailure {
            code: ErrorCode::BuildIdentityMismatch,
            kind: "identity_mismatch".to_string(),
            failed_task: None,
            first_error: Some(redact_line(
                ctx,
                &format!(
                    "built APK declares {:?}, expected {expected}",
                    resolution.selected.package
                ),
            )),
            log_path: Some(log.path),
            log_sha256: Some(log.sha256),
            duration_ms,
            argv: options.argv.clone(),
            exit_code: output.exit_code,
            timed_out: false,
        });
        return Err(outcome.into_error());
    }

    outcome.artifact = Some(resolution.selected);
    Ok(outcome)
}

fn command_spec(options: &BuildOptions, trace: bool) -> CommandSpec {
    let mut spec = CommandSpec::new(options.argv[0].clone())
        .args(options.argv[1..].iter().cloned())
        .cwd(&options.cwd)
        .timeout(options.timeout)
        .max_stdout(BUILD_LOG_MAX_BYTES * 2)
        .max_stderr(BUILD_LOG_MAX_BYTES * 2)
        .trace(trace)
        .hide_window(true);
    // The host environment is inherited by the runner. Named variables are
    // copied explicitly to make the contract deterministic; their values are
    // never echoed into logs, results or error context.
    for name in &options.env_names {
        if let Ok(value) = std::env::var(name) {
            spec = spec.env(name.clone(), value);
        }
    }
    spec
}

fn reject_invalid_build_config(config: &ForgeConfig) -> Result<()> {
    let relevant = |field: &str| {
        field.starts_with("build.commands")
            || field == "build.cwd"
            || field == "build.timeout_secs"
            || field == "build.env_passthrough"
    };
    let issues: Vec<_> = config
        .validate()
        .issues
        .into_iter()
        .filter(|issue| issue.severity == Severity::Error && relevant(&issue.field))
        .collect();
    if issues.is_empty() {
        return Ok(());
    }
    Err(
        ForgeError::new(ErrorCode::ConfigInvalid, "build configuration is invalid")
            .with_context("issues", serde_json::to_value(&issues).unwrap_or_default())
            .with_suggestion(
                "Run `androidforge config validate` and fix the reported build issues.",
            ),
    )
}

fn resolve_command<'a>(
    commands: &'a BTreeMap<String, Vec<String>>,
    variant: &str,
) -> Option<&'a Vec<String>> {
    if let Some(argv) = commands.get(variant) {
        return Some(argv);
    }
    let mut matches = commands
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case(variant));
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first.1)
}

fn artifact_error_kind(error: &ForgeError) -> &'static str {
    let message = error.message();
    if message.contains("same timestamp but different content") {
        "artifact_ambiguous"
    } else if message.contains("identity") || message.contains("package") {
        "identity_mismatch"
    } else {
        "artifact_not_found"
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn redact_line(ctx: &CliContext, line: &str) -> String {
    truncate_chars(&ctx.redactor().redact(line), FIRST_ERROR_MAX_CHARS)
}

fn truncate_chars(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = text[..end].to_string();
    if end < text.len() {
        out.push_str("...");
    }
    out
}

/// Bound a redacted log to `max` bytes while keeping its head and tail.
fn bound_log(text: &str, max: usize) -> (Vec<u8>, bool, u64) {
    if text.len() <= max {
        return (text.as_bytes().to_vec(), false, 0);
    }
    // Keep the marker reserve small relative to the tail budget.
    let tail_budget = (max / 3).min(max.saturating_sub(96)).max(1);
    let head_budget = max.saturating_sub(tail_budget + 96).max(1);
    let mut head_end = head_budget.min(text.len());
    while head_end > 0 && !text.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = text.len().saturating_sub(tail_budget);
    while tail_start < text.len() && !text.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let omitted = tail_start.saturating_sub(head_end) as u64;
    let marker = format!(
        "\n[... build log truncated: {omitted} bytes omitted; head and tail preserved ...]\n"
    );
    let mut out = Vec::with_capacity(max);
    out.extend_from_slice(&text.as_bytes()[..head_end]);
    out.extend_from_slice(marker.as_bytes());
    out.extend_from_slice(&text.as_bytes()[tail_start..]);
    (out, true, omitted)
}

fn write_build_log(
    ctx: &CliContext,
    options: &BuildOptions,
    combined: &str,
    first_error: Option<&str>,
) -> Result<BuildLog> {
    let mut text = format!("[androidforge] build log at {}\n", now_rfc3339());
    if let Some(first_error) = first_error {
        text.push_str("[first_error] ");
        text.push_str(first_error);
        text.push('\n');
    }
    text.push_str(combined);
    let redacted = ctx.redactor().redact(&text);
    let (bytes, truncated, omitted_bytes) = bound_log(&redacted, BUILD_LOG_MAX_BYTES);
    let sha256 = sha256_hex(&bytes);
    let dir = ctx
        .config
        .artifact_dir
        .join("build")
        .join(slugify(&options.variant, 40));
    ensure_dir(&dir)?;
    let path = dir.join(format!("build-{}.log", new_run_token()));
    write_atomic(&path, &bytes).map_err(|error| {
        error.with_context_string("log_path", path.to_string_lossy().into_owned())
    })?;
    Ok(BuildLog {
        path: path.to_string_lossy().into_owned(),
        sha256,
        bytes: bytes.len() as u64,
        truncated,
        omitted_bytes,
    })
}

#[derive(Debug, Default)]
struct ParsedFailure {
    kind: &'static str,
    failed_task: Option<String>,
    first_error: Option<String>,
}

static TASK_FAILED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^> Task\s+(\S+)\s+FAILED").expect("task regex"));
static EXEC_FAILED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Execution failed for task\s+'([^']+)'").expect("exec regex"));
static KOTLIN_ERROR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^e:\s*(.+)$").expect("kotlin regex"));
static C_ERROR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^error:\s*(.+)$").expect("compiler regex"));

/// Parse common Gradle/Kotlin/compiler/SDK failure markers into stable fields.
/// A non-zero exit with no recognised marker stays `non_zero`.
fn parse_failure(output: &CommandOutput) -> ParsedFailure {
    let combined = format!("{}\n{}", output.stdout_text(), output.stderr_text());
    let mut parsed = ParsedFailure {
        kind: "non_zero",
        ..ParsedFailure::default()
    };
    let mut gradle = false;
    let mut toolchain = false;
    let mut after_what_went_wrong = false;
    for line in combined.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(captures) = TASK_FAILED.captures(line) {
            parsed.failed_task = Some(captures[1].to_string());
            gradle = true;
        }
        if let Some(captures) = EXEC_FAILED.captures(line) {
            if parsed.failed_task.is_none() {
                parsed.failed_task = Some(captures[1].to_string());
            }
            gradle = true;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.contains("failure: build failed with an exception")
            || lower == "build failed"
            || lower.contains("what went wrong:")
        {
            gradle = true;
            if lower.contains("what went wrong:") {
                after_what_went_wrong = true;
            }
        }
        if after_what_went_wrong && trimmed.starts_with("> ") && parsed.first_error.is_none() {
            parsed.first_error = Some(trimmed.trim_start_matches("> ").to_string());
            continue;
        }
        if let Some(captures) = KOTLIN_ERROR.captures(line) {
            parsed
                .first_error
                .get_or_insert_with(|| captures[1].to_string());
        }
        if let Some(captures) = C_ERROR.captures(line) {
            parsed
                .first_error
                .get_or_insert_with(|| captures[1].to_string());
        }
        if is_toolchain_marker(&lower) {
            toolchain = true;
            parsed
                .first_error
                .get_or_insert_with(|| trimmed.to_string());
        }
        if parsed.first_error.is_none() && lower.contains("error") {
            parsed.first_error = Some(trimmed.to_string());
        }
    }
    parsed.kind = if toolchain {
        // A concrete SDK/JDK incompatibility is more actionable than the
        // generic Gradle wrapper markers that surround it.
        "toolchain_incompatible"
    } else if gradle && parsed.failed_task.is_some() {
        "gradle_task_failed"
    } else if gradle {
        "gradle_failed"
    } else {
        "non_zero"
    };
    parsed
}

fn is_toolchain_marker(lower: &str) -> bool {
    [
        "unsupported class file major version",
        "java version",
        "could not determine java version",
        "no java installation",
        "invalid java_home",
        "sdk location not found",
        "android sdk location",
        "unsupported compilesdkversion",
        "minimum supported gradle version",
        "please update android sdk",
        "android sdk platform",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::path::Path;
    use std::process::Command as StdCommand;
    use std::sync::OnceLock;

    fn toml_string(value: &str) -> String {
        value.replace('\\', "\\\\").replace('"', "\\\"")
    }

    fn ps_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    fn fake_aapt2() -> PathBuf {
        static PATH: OnceLock<PathBuf> = OnceLock::new();
        PATH.get_or_init(|| {
            let dir = std::env::temp_dir()
                .join(format!("androidforge-build-tools-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("create fake tool dir");
            let source = dir.join("fake_aapt2.rs");
            std::fs::write(
                &source,
                r#"fn main() {
    println!("package: name='com.example.build' versionCode='7' versionName='1.2.3'");
    println!("sdkVersion:'24' targetSdkVersion:'35'");
}
"#,
            )
            .expect("write fake aapt2 source");
            let executable = dir.join(if cfg!(windows) {
                "fake_aapt2.exe"
            } else {
                "fake_aapt2"
            });
            let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
            let status = StdCommand::new(rustc)
                .arg(&source)
                .arg("-o")
                .arg(&executable)
                .status()
                .expect("run rustc for fake aapt2");
            assert!(status.success(), "fake aapt2 compile failed");
            executable
        })
        .clone()
    }

    fn test_context(root: &Path, extra: &str) -> CliContext {
        let dir = root.join(".androidforge");
        std::fs::create_dir_all(&dir).expect("create adapter dir");
        let toml = format!(
            "adapter_version = 1\n[sdk]\naapt2 = \"{}\"\n[project]\nname = \"Build test\"\n{extra}",
            toml_string(&fake_aapt2().to_string_lossy())
        );
        std::fs::write(dir.join("project.toml"), toml).expect("write project.toml");
        let cli = crate::cli::Cli::try_parse_from([
            "androidforge",
            "--project",
            root.to_str().expect("utf-8 root"),
            "apk",
            "resolve",
        ])
        .expect("parse test CLI");
        CliContext::new(&cli).expect("test context")
    }

    fn script_argv(name: &str, windows_body: &str, unix_body: &str) -> Vec<String> {
        let dir =
            std::env::temp_dir().join(format!("androidforge-build-scripts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create script dir");
        if cfg!(windows) {
            let path = dir.join(format!("{name}.ps1"));
            std::fs::write(&path, windows_body).expect("write powershell script");
            vec![
                "powershell.exe".to_string(),
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                path.to_string_lossy().into_owned(),
            ]
        } else {
            let path = dir.join(format!("{name}.sh"));
            std::fs::write(&path, unix_body).expect("write shell script");
            vec!["sh".to_string(), path.to_string_lossy().into_owned()]
        }
    }

    fn toml_array(argv: &[String]) -> String {
        let values = argv
            .iter()
            .map(|value| format!("\"{}\"", toml_string(value)))
            .collect::<Vec<_>>()
            .join(", ");
        format!("[{values}]")
    }

    fn build_toml(
        variant: &str,
        argv: &[String],
        cwd: Option<&str>,
        timeout_secs: Option<u64>,
        env_names: &[&str],
    ) -> String {
        let mut out = format!("[build]\ndefault_variant = \"{variant}\"\n");
        if let Some(cwd) = cwd {
            out.push_str(&format!("cwd = \"{cwd}\"\n"));
        }
        if let Some(timeout) = timeout_secs {
            out.push_str(&format!("timeout_secs = {timeout}\n"));
        }
        if !env_names.is_empty() {
            let values = env_names
                .iter()
                .map(|name| format!("\"{name}\""))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("env_passthrough = [{values}]\n"));
        }
        out.push_str(&format!(
            "[build.commands]\n{variant} = {}\n",
            toml_array(argv)
        ));
        out.push_str(&format!(
            "[build.variants]\n{variant} = [\"app/build/outputs/apk/{variant}/*.apk\"]\n"
        ));
        out
    }

    fn context_value(error: &ForgeError, key: &str) -> serde_json::Value {
        error
            .body()
            .context
            .get(key)
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }

    fn log_bytes(outcome_or_error_log_path: &str) -> Vec<u8> {
        std::fs::read(outcome_or_error_log_path).expect("read build log")
    }

    #[tokio::test]
    async fn dry_run_executes_nothing_and_writes_nothing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let marker = root.join("marker.txt");
        let argv = script_argv(
            "dry_run",
            &format!(
                "Set-Content -LiteralPath {} -Value 'ran' -Encoding ascii\n",
                ps_quote(&marker.to_string_lossy())
            ),
            &format!("echo ran > '{}'\n", marker.display()),
        );
        let ctx = test_context(root, &build_toml("debug", &argv, None, None, &[]));
        let request = BuildRequest {
            variant: Some("debug".to_string()),
            dry_run: true,
            ..BuildRequest::default()
        };

        let outcome = run_build(&ctx, &request).await.expect("dry run succeeds");
        assert_eq!(outcome.status, BuildStatus::DryRun);
        assert_eq!(outcome.argv, argv);
        assert!(outcome.log.is_none());
        assert!(outcome.artifact.is_none());
        assert!(!marker.exists(), "dry-run must not execute the build");
        assert!(!root.join(".artifacts").join("build").exists());
    }

    #[tokio::test]
    async fn success_resolves_apk_identity_and_records_duration_and_log_hash() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        std::fs::create_dir_all(root.join("app")).expect("create module");
        let apk = root
            .join("app")
            .join("build")
            .join("outputs")
            .join("apk")
            .join("debug")
            .join("app-debug.apk");
        let parent = apk.parent().expect("apk parent");
        let argv = script_argv(
            "success",
            &format!(
                "$ErrorActionPreference = 'Stop'\nNew-Item -ItemType Directory -Force -Path {} | Out-Null\nSet-Content -LiteralPath {} -Value 'fake apk bytes' -Encoding ascii\nWrite-Output 'BUILD SUCCESSFUL'\n",
                ps_quote(&parent.to_string_lossy()),
                ps_quote(&apk.to_string_lossy())
            ),
            &format!(
                "mkdir -p '{}' && printf 'fake apk bytes' > '{}' && echo 'BUILD SUCCESSFUL'\n",
                parent.display(),
                apk.display()
            ),
        );
        let ctx = test_context(root, &build_toml("debug", &argv, Some("app"), None, &[]));

        let outcome = run_build(
            &ctx,
            &BuildRequest {
                variant: Some("debug".to_string()),
                ..BuildRequest::default()
            },
        )
        .await
        .expect("build succeeds");

        assert_eq!(outcome.status, BuildStatus::Success);
        assert_eq!(outcome.exit_code, Some(0));
        let artifact = outcome.artifact.expect("resolved APK");
        assert_eq!(artifact.package.as_deref(), Some("com.example.build"));
        assert_eq!(artifact.version_name.as_deref(), Some("1.2.3"));
        assert_eq!(artifact.version_code, Some(7));
        assert_eq!(
            artifact.sha256,
            sha256_hex(&std::fs::read(&apk).expect("read APK"))
        );
        let log = outcome.log.expect("log artifact");
        assert!(Path::new(&log.path).is_file());
        assert_eq!(log.sha256, sha256_hex(&log_bytes(&log.path)));
        assert!(log.bytes > 0);
    }

    #[tokio::test]
    async fn gradle_failure_returns_build_failed_with_stable_fields() {
        let temp = tempfile::tempdir().expect("tempdir");
        let argv = script_argv(
            "gradle_failure",
            "Write-Output '> Task :app:compileDebugKotlin FAILED'\nWrite-Output 'e: Unresolved reference: missingSymbol'\nWrite-Output 'FAILURE: Build failed with an exception.'\nWrite-Output '* What went wrong:'\nWrite-Output \"Execution failed for task ':app:compileDebugKotlin'.\"\nWrite-Output 'BUILD FAILED'\nexit 1\n",
            "echo '> Task :app:compileDebugKotlin FAILED'\necho 'e: Unresolved reference: missingSymbol'\necho 'FAILURE: Build failed with an exception.'\necho '* What went wrong:'\necho \"Execution failed for task ':app:compileDebugKotlin'.\"\necho 'BUILD FAILED'\nexit 1\n",
        );
        let ctx = test_context(temp.path(), &build_toml("debug", &argv, None, None, &[]));

        let error = run_build(
            &ctx,
            &BuildRequest {
                variant: Some("debug".to_string()),
                ..BuildRequest::default()
            },
        )
        .await
        .expect_err("non-zero exit fails");

        assert_eq!(error.code(), ErrorCode::BuildFailed);
        assert_eq!(
            context_value(&error, "failed_task").as_str(),
            Some(":app:compileDebugKotlin")
        );
        let first_error = context_value(&error, "first_error")
            .as_str()
            .expect("first_error")
            .to_string();
        assert!(
            first_error.contains("Unresolved reference"),
            "{first_error}"
        );
        assert_eq!(context_value(&error, "exit_code").as_i64(), Some(1));
        assert_eq!(
            context_value(&error, "argv").as_array().map(Vec::len),
            Some(argv.len())
        );
        assert!(context_value(&error, "duration_ms").as_u64().is_some());
        let log_path = context_value(&error, "log_path")
            .as_str()
            .expect("log_path")
            .to_string();
        assert!(Path::new(&log_path).is_file());
        let bytes = log_bytes(&log_path);
        assert_eq!(
            context_value(&error, "log_sha256").as_str(),
            Some(sha256_hex(&bytes).as_str())
        );
        let value = context_value(&error, "build_outcome");
        assert_eq!(value["failure"]["kind"], "gradle_task_failed");
    }

    #[tokio::test]
    async fn timeout_terminates_the_process_tree() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pid_file = temp.path().join("child.pid");
        let windows_body = format!(
            "$child = Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 30') -PassThru -WindowStyle Hidden\nSet-Content -LiteralPath {} -Value $child.Id -Encoding ascii\nWrite-Output 'starting long build'\nStart-Sleep -Seconds 30\n",
            ps_quote(&pid_file.to_string_lossy())
        );
        let unix_body = format!(
            "sleep 30 &\necho $! > '{}'\necho starting long build\nsleep 30\n",
            pid_file.display()
        );
        let argv = script_argv("timeout", &windows_body, &unix_body);
        let ctx = test_context(temp.path(), &build_toml("debug", &argv, None, None, &[]));

        let error = run_build(
            &ctx,
            &BuildRequest {
                variant: Some("debug".to_string()),
                timeout: Some(Duration::from_secs(2)),
                ..BuildRequest::default()
            },
        )
        .await
        .expect_err("timeout fails");

        assert_eq!(error.code(), ErrorCode::BuildTimeout);
        assert_eq!(context_value(&error, "build_outcome")["timed_out"], true);
        let pid = std::fs::read_to_string(&pid_file)
            .expect("child pid was recorded")
            .trim()
            .parse::<u32>()
            .expect("numeric child pid");
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            !androidforge_process::process_exists(pid),
            "timeout must kill the child process tree (pid {pid})"
        );
    }

    #[tokio::test]
    async fn missing_artifact_uses_build_artifact_not_found() {
        let temp = tempfile::tempdir().expect("tempdir");
        let argv = script_argv(
            "no_artifact",
            "Write-Output 'BUILD SUCCESSFUL'\n",
            "echo 'BUILD SUCCESSFUL'\n",
        );
        let ctx = test_context(temp.path(), &build_toml("debug", &argv, None, None, &[]));

        let error = run_build(
            &ctx,
            &BuildRequest {
                variant: Some("debug".to_string()),
                ..BuildRequest::default()
            },
        )
        .await
        .expect_err("missing APK fails");

        assert_eq!(error.code(), ErrorCode::BuildArtifactNotFound);
        assert_eq!(
            context_value(&error, "build_outcome")["failure"]["kind"],
            "artifact_not_found"
        );
    }

    #[tokio::test]
    async fn ambiguous_artifact_uses_build_artifact_ambiguous() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let dir = root
            .join("app")
            .join("build")
            .join("outputs")
            .join("apk")
            .join("debug");
        std::fs::create_dir_all(&dir).expect("create output dir");
        let first = dir.join("first.apk");
        let second = dir.join("second.apk");
        let windows_body = format!(
            "Set-Content -LiteralPath {} -Value 'first' -Encoding ascii\nSet-Content -LiteralPath {} -Value 'second' -Encoding ascii\n$fixed = [datetime]'2023-11-14T22:13:20Z'\n(Get-Item -LiteralPath {}).LastWriteTimeUtc = $fixed\n(Get-Item -LiteralPath {}).LastWriteTimeUtc = $fixed\nWrite-Output 'BUILD SUCCESSFUL'\n",
            ps_quote(&first.to_string_lossy()),
            ps_quote(&second.to_string_lossy()),
            ps_quote(&first.to_string_lossy()),
            ps_quote(&second.to_string_lossy())
        );
        let unix_body = format!(
            "printf first > '{}'\nprintf second > '{}'\ntouch -d '2023-11-14T22:13:20Z' '{}' '{}'\necho 'BUILD SUCCESSFUL'\n",
            first.display(),
            second.display(),
            first.display(),
            second.display()
        );
        let argv = script_argv("ambiguous", &windows_body, &unix_body);
        let ctx = test_context(root, &build_toml("debug", &argv, None, None, &[]));

        let error = run_build(
            &ctx,
            &BuildRequest {
                variant: Some("debug".to_string()),
                ..BuildRequest::default()
            },
        )
        .await
        .expect_err("ambiguous APKs fail");

        assert_eq!(error.code(), ErrorCode::BuildArtifactAmbiguous);
        assert_eq!(
            context_value(&error, "build_outcome")["failure"]["kind"],
            "artifact_ambiguous"
        );
    }

    #[tokio::test]
    async fn log_is_bounded_redacted_and_keeps_first_error_head_and_tail() {
        let temp = tempfile::tempdir().expect("tempdir");
        let argv = script_argv(
            "big_log",
            "[Console]::Out.Write(('HEADMARKER' + ('A' * 100000)))\nWrite-Output 'error: synthetic build error'\nWrite-Output 'token=SUPERSECRETVALUE123'\n[Console]::Out.Write(('B' * 200000) + 'TAILMARKER')\nexit 1\n",
            "printf 'HEADMARKER'; head -c 100000 /dev/zero | tr '\\0' 'A'; echo 'error: synthetic build error'; echo 'token=SUPERSECRETVALUE123'; head -c 200000 /dev/zero | tr '\\0' 'B'; printf 'TAILMARKER'; exit 1\n",
        );
        let ctx = test_context(temp.path(), &build_toml("debug", &argv, None, None, &[]));

        let error = run_build(
            &ctx,
            &BuildRequest {
                variant: Some("debug".to_string()),
                ..BuildRequest::default()
            },
        )
        .await
        .expect_err("non-zero exit fails");

        assert_eq!(error.code(), ErrorCode::BuildFailed);
        let log_path = context_value(&error, "log_path")
            .as_str()
            .expect("log_path")
            .to_string();
        let bytes = log_bytes(&log_path);
        assert!(bytes.len() <= BUILD_LOG_MAX_BYTES, "log must stay bounded");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("HEADMARKER"), "head must be preserved");
        assert!(text.contains("TAILMARKER"), "tail must be preserved");
        assert!(
            text.contains("[first_error]"),
            "first error must be preserved"
        );
        assert!(text.contains("[REDACTED]"), "secret must be redacted");
        assert!(!text.contains("SUPERSECRETVALUE123"), "secret value leaked");
        assert!(text.contains("truncated"), "truncation marker expected");
        assert_eq!(
            context_value(&error, "log_sha256").as_str(),
            Some(sha256_hex(&bytes).as_str())
        );
    }

    #[tokio::test]
    async fn no_configured_command_uses_build_command_not_configured() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ctx = test_context(temp.path(), "[build]\ndefault_variant = \"debug\"\n");
        let error = run_build(
            &ctx,
            &BuildRequest {
                variant: Some("debug".to_string()),
                ..BuildRequest::default()
            },
        )
        .await
        .expect_err("missing command fails");
        assert_eq!(error.code(), ErrorCode::BuildCommandNotConfigured);
    }

    #[tokio::test]
    async fn named_environment_variable_is_passed_but_its_value_is_never_logged() {
        let temp = tempfile::tempdir().expect("tempdir");
        const NAME: &str = "ANDROIDFORGE_BUILD_PROBE_TOKEN";
        const VALUE: &str = "probe-value-must-not-appear-in-logs";
        // SAFETY: this test is the only writer of NAME, and the child process
        // is spawned after the assignment completes.
        unsafe { std::env::set_var(NAME, VALUE) };
        let apk = temp
            .path()
            .join("app/build/outputs/apk/debug/app-debug.apk");
        let parent = apk.parent().expect("apk parent");
        let windows_body = format!(
            "if (-not $env:{NAME}) {{ Write-Output 'ENV_MISSING'; exit 1 }}\nWrite-Output 'ENV_PRESENT'\nNew-Item -ItemType Directory -Force -Path {} | Out-Null\nSet-Content -LiteralPath {} -Value 'fake apk bytes' -Encoding ascii\n",
            ps_quote(&parent.to_string_lossy()),
            ps_quote(&apk.to_string_lossy())
        );
        let unix_body = format!(
            "if [ -z \"${NAME}\" ]; then echo ENV_MISSING; exit 1; fi\necho ENV_PRESENT\nmkdir -p '{}' && printf 'fake apk bytes' > '{}'\n",
            parent.display(),
            apk.display()
        );
        let argv = script_argv("env_probe", &windows_body, &unix_body);
        let ctx = test_context(
            temp.path(),
            &build_toml("debug", &argv, None, None, &[NAME]),
        );
        let outcome = run_build(
            &ctx,
            &BuildRequest {
                variant: Some("debug".to_string()),
                ..BuildRequest::default()
            },
        )
        .await
        .expect("env passthrough build succeeds");
        // SAFETY: paired removal of the single test-owned variable.
        unsafe { std::env::remove_var(NAME) };
        let log = outcome.log.as_ref().expect("log").clone();
        let log_data = log_bytes(&log.path);
        let text = String::from_utf8_lossy(&log_data);
        assert!(text.contains("ENV_PRESENT"));
        assert!(!text.contains(VALUE), "environment value leaked into log");
        assert!(
            !serde_json::to_string(&outcome)
                .expect("outcome json")
                .contains(VALUE)
        );
    }

    #[test]
    fn parses_gradle_sdk_and_generic_failure_markers() {
        let output = CommandOutput {
            program: "fake".to_string(),
            args: Vec::new(),
            exit_code: Some(1),
            success: false,
            stdout: b"> Task :app:compileDebugKotlin FAILED\ne: Unresolved reference: x\nFAILURE: Build failed with an exception.\n".to_vec(),
            stderr: b"Execution failed for task ':app:compileDebugKotlin'.\n".to_vec(),
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
            timed_out: false,
        };
        let parsed = parse_failure(&output);
        assert_eq!(parsed.kind, "gradle_task_failed");
        assert_eq!(
            parsed.failed_task.as_deref(),
            Some(":app:compileDebugKotlin")
        );
        assert!(parsed.first_error.unwrap().contains("Unresolved reference"));

        let mut sdk = output;
        sdk.stdout =
            b"FAILURE: Build failed with an exception.\nUnsupported class file major version 65\n"
                .to_vec();
        sdk.stderr.clear();
        let parsed = parse_failure(&sdk);
        assert_eq!(parsed.kind, "toolchain_incompatible");
        assert!(
            parsed
                .first_error
                .unwrap()
                .contains("Unsupported class file")
        );

        let mut generic = sdk;
        generic.stdout = b"something broke\n".to_vec();
        generic.stderr.clear();
        assert_eq!(parse_failure(&generic).kind, "non_zero");
    }

    #[test]
    fn bound_log_keeps_head_and_tail_with_char_boundaries() {
        let text = format!("HEAD{}TAIL", "é".repeat(50_000));
        let (bytes, truncated, omitted) = bound_log(&text, 512);
        assert!(truncated);
        assert!(omitted > 0);
        assert!(bytes.len() <= 512);
        let out = String::from_utf8(bytes).expect("valid utf-8");
        assert!(out.starts_with("HEAD"));
        assert!(out.ends_with("TAIL"));
        assert!(out.contains("truncated"));
    }

    #[test]
    fn cold_request_stays_on_bare_package_force_stop_path() {
        use crate::commands::app::{LaunchRequest, launch_uses_intent};

        let cold = LaunchRequest::simple(
            "com.example".to_string(),
            Some(".MainActivity".to_string()),
            true,
        );
        assert!(cold.cold);
        assert!(cold.force_stop);
        assert!(
            !launch_uses_intent(&cold),
            "--cold with --activity must not enter the intent path"
        );

        let warm = LaunchRequest::simple(
            "com.example".to_string(),
            Some(".MainActivity".to_string()),
            false,
        );
        assert!(!warm.cold);
        assert!(!warm.force_stop);
        assert!(launch_uses_intent(&warm));
    }

    #[tokio::test]
    async fn unclassifiable_capped_output_uses_build_output_too_large() {
        let temp = tempfile::tempdir().expect("tempdir");
        let argv = script_argv(
            "capped_generic_failure",
            "[Console]::Out.Write(('A' * 600000))\nexit 1\n",
            "head -c 600000 /dev/zero | tr '\\0' 'A'\nexit 1\n",
        );
        let ctx = test_context(temp.path(), &build_toml("debug", &argv, None, None, &[]));
        let error = run_build(
            &ctx,
            &BuildRequest {
                variant: Some("debug".to_string()),
                ..BuildRequest::default()
            },
        )
        .await
        .expect_err("generic non-zero fails");

        assert_eq!(error.code(), ErrorCode::BuildOutputTooLarge);
        let outcome = context_value(&error, "build_outcome");
        assert_eq!(outcome["failure"]["kind"], "non_zero");
        assert!(outcome["failure"]["timed_out"] == false);
        assert!(
            outcome["warnings"]
                .as_array()
                .is_some_and(|warnings| warnings.iter().any(|warning| warning
                    .as_str()
                    .is_some_and(|text| text.contains("capture limit"))))
        );
    }
}
