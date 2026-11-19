//! Multi-device matrix execution, sequential or parallel.
//!
//! Each target gets its own owned session and runs the existing `agent test`
//! pipeline as a child process. Parallel mode launches one independent child
//! matrix per target with late-binding, cross-process port allocation.

use std::path::{Path, PathBuf};
use std::time::Instant;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::ArtifactRef;
use androidforge_core::output::CommandSuccess;
use androidforge_core::session::{Session, SessionState};
use androidforge_core::util::{ensure_dir, now_rfc3339, short_id, slugify, write_json_atomic};
use androidforge_diagnostics::{HealthThresholds, collect_health};
use androidforge_process::{CommandSpec, Runner};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cli::{AgentMatrixArgs, Cli, SessionCommand, SessionEnsureArgs, SessionReleaseArgs};
use crate::commands::common::success;
use crate::commands::{apk, session};
use crate::context::CliContext;

fn default_matrix_parallel() -> usize {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum MatrixTargetSpec {
    Avd { name: String },
    Serial { serial: String },
}

impl MatrixTargetSpec {
    fn id(&self) -> String {
        match self {
            Self::Avd { name } => format!("avd:{name}"),
            Self::Serial { serial } => format!("serial:{serial}"),
        }
    }

    fn label(&self) -> &str {
        match self {
            Self::Avd { name } => name,
            Self::Serial { serial } => serial,
        }
    }

    fn key(&self) -> String {
        self.id().to_ascii_lowercase()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MatrixPlanTarget {
    target: MatrixTargetSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_level: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MatrixTargetResult {
    target: MatrixTargetSpec,
    status: String,
    started_at: String,
    ended_at: String,
    duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acquisition_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<androidforge_diagnostics::DeviceHealth>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub health_warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
    #[serde(default)]
    pub cleanup: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_report: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MatrixReport {
    schema_version: u32,
    matrix_run_id: String,
    project: String,
    project_root: String,
    flow: Option<String>,
    fixture: Option<String>,
    build: Value,
    targets_requested: usize,
    started_at: String,
    ended_at: String,
    duration_ms: u64,
    aggregate_status: String,
    passed: usize,
    failed: usize,
    skipped: usize,
    cleanup_policy: ValidationCleanupPolicy,
    #[serde(default = "default_matrix_parallel")]
    parallel: usize,
    target_results: Vec<MatrixTargetResult>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ValidationCleanupPolicy {
    sessions: String,
    emulators: String,
    note: String,
}

pub async fn matrix(cli: &Cli, args: &AgentMatrixArgs) -> Result<CommandSuccess> {
    let ctx = CliContext::new(cli)?;
    if args.parallel == 0 || args.parallel > 32 {
        return Err(ForgeError::invalid_argument(
            "--parallel must be between 1 and 32",
        ));
    }
    if args.avd.is_empty() && args.serial.is_empty() {
        return Err(ForgeError::new(
            ErrorCode::MatrixTargetInvalid,
            "agent matrix requires at least one --avd or --serial target",
        )
        .with_suggestion("Pass --avd <AVD> twice for two independent emulators."));
    }
    if args.apk.is_none() && args.variant.is_none() && args.flow.is_none() && args.fixture.is_none()
    {
        return Err(ForgeError::invalid_argument(
            "agent matrix needs a flow, fixture, APK or variant",
        )
        .with_suggestion("Example: --avd A --avd B --flow conformance --json"));
    }

    let mut targets = Vec::new();
    for avd in &args.avd {
        targets.push(MatrixTargetSpec::Avd { name: avd.clone() });
    }
    for serial in &args.serial {
        targets.push(MatrixTargetSpec::Serial {
            serial: serial.clone(),
        });
    }
    let mut seen = std::collections::BTreeSet::new();
    for target in &targets {
        if !seen.insert(target.key()) {
            return Err(ForgeError::new(
                ErrorCode::MatrixDuplicateTarget,
                format!(
                    "matrix target '{}' was requested more than once",
                    target.label()
                ),
            )
            .with_suggestion("Every target must be a distinct device or AVD."));
        }
    }

    let mut warnings = Vec::new();
    let apk_path = if let Some(path) = &args.apk {
        if args.variant.is_some() {
            warnings.push(
                "--apk takes precedence over --variant; the matrix will use the explicit APK."
                    .to_string(),
            );
        }
        Some(if path.is_absolute() {
            path.clone()
        } else {
            ctx.paths.cwd.join(path)
        })
    } else if let Some(variant) = &args.variant {
        let resolution = apk::resolve_variant(&ctx, variant, None).await?;
        warnings.extend(resolution.warnings.clone());
        Some(PathBuf::from(&resolution.selected.path))
    } else {
        None
    };
    let build = if let Some(path) = &apk_path {
        match ctx.inspector().inspect(path, false).await {
            Ok(info) => json!({
                "apk_path": info.path,
                "apk_sha256": info.sha256,
                "package": info.package,
                "version_name": info.version_name,
                "version_code": info.version_code,
                "cert_sha256": info.signature.cert_sha256,
            }),
            Err(error) => {
                return Err(error.with_context_string("matrix_stage", "resolve_build".to_string()));
            }
        }
    } else {
        json!({"apk_path": null, "source": "no --apk/--variant; the child pipeline will not install"})
    };

    let started_at = now_rfc3339();
    let started = Instant::now();
    let matrix_id = format!(
        "matrix-{}-{}",
        chrono::Utc::now().format("%Y%m%d%H%M%S"),
        short_id()
    );
    let matrix_dir = ctx.paths.artifacts_dir.join("matrix").join(&matrix_id);
    let owner = args.owner.clone().unwrap_or_else(|| "matrix".to_string());
    let child_timeout = ctx.parse_timeout(args.timeout.as_deref(), 30 * 60)?;

    let plan = build_plan(&ctx, &targets).await?;

    if ctx.dry_run {
        let result = json!({
            "schema_version": 1,
            "dry_run": true,
            "matrix_run_id": matrix_id,
            "project": ctx.config.project_name(),
            "project_root": ctx.paths.project_root,
            "flow": args.flow,
            "fixture": args.fixture,
            "build": build,
            "owner": owner,
            "parallel": args.parallel,
            "fail_fast": args.fail_fast,
            "timeout_seconds": child_timeout.as_secs(),
            "cleanup_policy": {
                "sessions": if args.keep_sessions { "keep matrix-created sessions" } else { "release sessions created by this matrix; existing sessions are untouched" },
                "emulators": "never stopped by default",
                "stop_emulators": false,
            },
            "health_preflight": ctx.config.file.health.matrix_preflight,
            "targets": plan.iter().map(|target| json!({
                "target": target.target,
                "label": target.target.label(),
                "expected_model": target.model,
                "expected_api_level": target.api_level,
                "status": "planned",
                "would_ensure_session": true,
                "would_collect_health": ctx.config.file.health.matrix_preflight,
                "would_run": "agent test",
            })).collect::<Vec<_>>(),
            "destructive_operations": [],
            "warnings": warnings,
        });
        return Ok(success(
            "agent.matrix",
            result,
            format!(
                "Matrix dry run: {} target(s) with {} worker(s), flow {:?}; no device or session was changed.\n",
                plan.len(),
                args.parallel,
                args.flow
            ),
        ));
    }

    if args.parallel > 1 {
        return parallel_matrix(
            &ctx,
            args,
            &plan,
            &build,
            &matrix_id,
            &matrix_dir,
            &owner,
            child_timeout,
            apk_path.as_deref(),
            &started_at,
            started,
            warnings,
        )
        .await;
    }

    ensure_dir(&matrix_dir)?;
    let mut target_results = Vec::new();
    let mut fail_fast_triggered = false;
    for (index, target) in plan.iter().enumerate() {
        let target_started_at = now_rfc3339();
        let target_started = Instant::now();
        if fail_fast_triggered {
            target_results.push(MatrixTargetResult {
                target: target.target.clone(),
                status: "skipped".to_string(),
                started_at: target_started_at.clone(),
                ended_at: target_started_at.clone(),
                duration_ms: 0,
                session_id: None,
                acquisition_mode: None,
                serial: None,
                api_level: target.api_level,
                model: target.model.clone(),
                health: None,
                health_warnings: Vec::new(),
                run_id: None,
                artifact_directory: None,
                failure_code: Some("E_MATRIX_FAIL_FAST".to_string()),
                failure_message: Some(
                    "skipped because --fail-fast stopped after an earlier target failure"
                        .to_string(),
                ),
                cleanup: "not_applicable".to_string(),
                child_status: None,
                child_exit_code: None,
                child_report: None,
                warnings: Vec::new(),
            });
            continue;
        }

        let mut target_result = MatrixTargetResult {
            target: target.target.clone(),
            status: "failed".to_string(),
            started_at: target_started_at.clone(),
            ended_at: target_started_at.clone(),
            duration_ms: 0,
            session_id: None,
            acquisition_mode: None,
            serial: None,
            api_level: target.api_level,
            model: target.model.clone(),
            health: None,
            health_warnings: Vec::new(),
            run_id: None,
            artifact_directory: None,
            failure_code: None,
            failure_message: None,
            cleanup: "not_applicable".to_string(),
            child_status: None,
            child_exit_code: None,
            child_report: None,
            warnings: Vec::new(),
        };

        let ensure_args = SessionEnsureArgs {
            avd: match &target.target {
                MatrixTargetSpec::Avd { name } => Some(name.clone()),
                _ => None,
            },
            serial: match &target.target {
                MatrixTargetSpec::Serial { serial } => Some(serial.clone()),
                _ => None,
            },
            owner: Some(owner.clone()),
            profile: args.profile.clone(),
            ttl: args.ttl.clone(),
            port: None,
            headless: true,
            snapshot_save: false,
            wait: false,
            timeout: "10m".to_string(),
            prepare: false,
        };
        let acquisition = match session::ensure(&ctx, &ensure_args).await {
            Ok(result) => result,
            Err(error) => {
                target_result.failure_code = Some(error.body().code.as_str().to_string());
                target_result.failure_message = Some(error.message().to_string());
                target_result.ended_at = now_rfc3339();
                target_result.duration_ms = target_started.elapsed().as_millis() as u64;
                target_result.cleanup = "not_acquired".to_string();
                target_results.push(target_result);
                write_target_report(&matrix_dir, index, target_results.last().unwrap())?;
                if args.fail_fast {
                    fail_fast_triggered = true;
                }
                continue;
            }
        };
        let session_id = acquisition
            .result
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let acquisition_mode = acquisition
            .result
            .get("acquisition_mode")
            .and_then(Value::as_str)
            .map(str::to_string);
        target_result.session_id = session_id.clone();
        target_result.acquisition_mode = acquisition_mode.clone();
        if let Some(session_id) = &session_id {
            match Session::load(&ctx.paths, session_id) {
                Ok(session) => {
                    target_result.serial = session.serial.clone();
                }
                Err(error) => target_result.warnings.push(format!(
                    "could not reload the matrix session: {}",
                    error.message()
                )),
            }
        }

        if let Some(session_id) = &session_id
            && ctx.config.file.health.matrix_preflight
            && let Ok(session) = Session::load(&ctx.paths, session_id)
            && let Ok(device) = ctx.resolve_device(Some(&session)).await
        {
            let thresholds = HealthThresholds::from(&ctx.config.file.health);
            if let Ok(adb) = ctx.adb() {
                let health = collect_health(&adb, &device, &thresholds).await;
                target_result.serial = Some(device.serial.clone());
                target_result.model = health.model.clone();
                target_result.api_level = health.api_level;
                target_result.health_warnings = health.warnings.clone();
                target_result.health = Some(health);
            }
        }

        let argv = child_argv(
            &ctx,
            args,
            &owner,
            session_id.as_deref(),
            apk_path.as_deref(),
        );
        let runner = Runner::new(ctx.trace);
        let spec = CommandSpec::new(
            std::env::current_exe()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "androidforge".to_string()),
        )
        .args(argv.clone())
        .cwd(ctx.paths.cwd.clone())
        .timeout(child_timeout)
        .trace(ctx.trace)
        .hide_window(true)
        .max_stdout(32 * 1024 * 1024)
        .max_stderr(8 * 1024 * 1024);
        match runner.run(spec).await {
            Ok(output) => {
                let stdout = output.stdout_text();
                match serde_json::from_str::<Value>(&stdout) {
                    Ok(envelope) => {
                        target_result.child_status = envelope
                            .get("status")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        target_result.child_exit_code = envelope
                            .get("exit_code")
                            .and_then(Value::as_i64)
                            .map(|code| code as i32);
                        let result = envelope.get("result").cloned().unwrap_or(Value::Null);
                        if let Some(child_warnings) =
                            result.get("warnings").and_then(Value::as_array)
                        {
                            target_result.warnings.extend(
                                child_warnings
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_string),
                            );
                        }
                        target_result.run_id = result
                            .get("run_id")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        target_result.child_report = result
                            .get("report_path")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        target_result.artifact_directory = target_result
                            .run_id
                            .as_ref()
                            .map(|run_id| ctx.paths.run_dir(run_id).to_string_lossy().into_owned());
                        if let Some(failure) = result.get("failure") {
                            target_result.failure_code = failure
                                .get("code")
                                .or_else(|| {
                                    failure.get("error").and_then(|error| error.get("code"))
                                })
                                .and_then(Value::as_str)
                                .map(str::to_string);
                            target_result.failure_message = failure
                                .get("message")
                                .or_else(|| {
                                    failure.get("error").and_then(|error| error.get("message"))
                                })
                                .and_then(Value::as_str)
                                .map(str::to_string);
                        }
                        if !output.success
                            || target_result.child_status.as_deref() == Some("failed")
                        {
                            target_result.status = "failed".to_string();
                        } else if target_result.child_status.as_deref() == Some("blocked") {
                            target_result.status = "failed".to_string();
                            target_result.failure_code =
                                Some("E_MATRIX_TARGET_BLOCKED".to_string());
                            target_result.failure_message = Some(
                                "the child agent pipeline was blocked before testing".to_string(),
                            );
                        } else {
                            target_result.status = "passed".to_string();
                        }
                        write_json_atomic(
                            &matrix_dir.join("targets").join(format!(
                                "{:02}-{}.json",
                                index + 1,
                                slugify(target.target.label(), 40)
                            )),
                            &json!({
                                "target": target.target,
                                "argv": argv,
                                "envelope": envelope,
                            }),
                        )?;
                    }
                    Err(error) => {
                        target_result.failure_code =
                            Some("E_MATRIX_CHILD_OUTPUT_INVALID".to_string());
                        target_result.failure_message = Some(format!(
                            "agent test returned invalid JSON: {error}; stderr: {}",
                            truncate(output.stderr_text().trim(), 2000)
                        ));
                    }
                }
            }
            Err(error) => {
                let code = match error {
                    androidforge_process::ProcessError::Timeout { .. } => ErrorCode::CommandTimeout,
                    _ => ErrorCode::CommandFailed,
                };
                target_result.failure_code = Some(code.as_str().to_string());
                target_result.failure_message = Some(error.to_string());
                target_result.warnings.push(format!(
                    "child agent process failed for target {}",
                    target.target.label()
                ));
            }
        }

        if !args.keep_sessions
            && acquisition_mode.as_deref() != Some("existing_session")
            && let Some(session_id) = &session_id
        {
            target_result.cleanup = release_matrix_session(&ctx, session_id, &owner).await?;
        } else {
            target_result.cleanup = if args.keep_sessions {
                "kept_by_request".to_string()
            } else {
                "kept_existing_session".to_string()
            };
        }

        target_result.ended_at = now_rfc3339();
        target_result.duration_ms = target_started.elapsed().as_millis() as u64;
        if target_result.status != "passed" && args.fail_fast {
            fail_fast_triggered = true;
        }
        target_results.push(target_result);
        write_target_report(&matrix_dir, index, target_results.last().unwrap())?;
    }

    let passed = target_results
        .iter()
        .filter(|target| target.status == "passed")
        .count();
    let failed = target_results
        .iter()
        .filter(|target| target.status == "failed")
        .count();
    let skipped = target_results
        .iter()
        .filter(|target| target.status == "skipped")
        .count();
    let aggregate_status = if failed == 0 && skipped == 0 {
        "passed"
    } else if passed > 0 {
        "partial"
    } else {
        "failed"
    };
    let report = MatrixReport {
        schema_version: 1,
        matrix_run_id: matrix_id.clone(),
        project: ctx.config.project_name(),
        project_root: ctx.paths.project_root.to_string_lossy().into_owned(),
        flow: args.flow.clone(),
        fixture: args.fixture.clone(),
        build,
        targets_requested: targets.len(),
        started_at,
        ended_at: now_rfc3339(),
        duration_ms: started.elapsed().as_millis() as u64,
        aggregate_status: aggregate_status.to_string(),
        passed,
        failed,
        skipped,
        cleanup_policy: ValidationCleanupPolicy {
            sessions: if args.keep_sessions {
                "kept matrix-created sessions".to_string()
            } else {
                "released sessions created by this matrix; existing sessions untouched".to_string()
            },
            emulators: "not stopped by default".to_string(),
            note: "Only sessions acquired by this matrix with the matrix owner are released."
                .to_string(),
        },
        parallel: 1,
        target_results,
        warnings: warnings.clone(),
    };
    let report_path = matrix_dir.join("matrix.json");
    write_json_atomic(&report_path, &report)?;
    let result = serde_json::to_value(&report)?;
    let mut response = success(
        "agent.matrix",
        result,
        format!(
            "Matrix {}: {} passed, {} failed, {} skipped across {} target(s).\nReport: {}\n",
            aggregate_status,
            passed,
            failed,
            skipped,
            targets.len(),
            report_path.display()
        ),
    );
    response.run_id = Some(matrix_id);
    response.artifacts = vec![
        ArtifactRef::new("matrix_report", report_path.to_string_lossy().into_owned())
            .with_description("Sequential matrix report"),
    ];
    if aggregate_status != "passed" {
        response.status = "failed";
        response.exit_code = if aggregate_status == "partial" {
            ErrorCode::PartialFailure.exit_code()
        } else {
            ErrorCode::CommandFailed.exit_code()
        };
    }
    Ok(response)
}

#[allow(clippy::too_many_arguments)]
async fn parallel_matrix(
    ctx: &CliContext,
    args: &AgentMatrixArgs,
    plan: &[MatrixPlanTarget],
    build: &Value,
    matrix_id: &str,
    matrix_dir: &Path,
    owner: &str,
    child_timeout: std::time::Duration,
    apk_path: Option<&Path>,
    started_at: &str,
    started: Instant,
    mut warnings: Vec<String>,
) -> Result<CommandSuccess> {
    ensure_dir(matrix_dir)?;
    warnings.push(
        "parallel matrix uses one independent child matrix process per target; --fail-fast is best-effort and already-running targets finish."
            .to_string(),
    );
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(
        args.parallel.min(plan.len().max(1)),
    ));
    let mut set = tokio::task::JoinSet::new();
    for (index, target) in plan.iter().cloned().enumerate() {
        let argv = matrix_child_argv(ctx, args, &target, owner, apk_path, child_timeout);
        let cwd = ctx.paths.cwd.clone();
        let target_dir = matrix_dir.join("targets");
        let semaphore = semaphore.clone();
        set.spawn(async move {
            let _permit = semaphore.acquire_owned().await;
            run_parallel_target(index, target, argv, cwd, target_dir, child_timeout).await
        });
    }
    let mut target_results = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((index, target, result, envelope)) => {
                write_json_atomic(
                    &matrix_dir.join("targets").join(format!(
                        "{:02}-{}.json",
                        index + 1,
                        slugify(target.target.label(), 40)
                    )),
                    &json!({"target": target.target, "argv": matrix_child_argv(ctx, args, &target, owner, apk_path, child_timeout), "envelope": envelope}),
                )?;
                target_results.push((index, result));
            }
            Err(error) => {
                warnings.push(format!("parallel worker failed: {error}"));
            }
        }
    }
    target_results.sort_by_key(|(index, _)| *index);
    let target_results = target_results
        .into_iter()
        .map(|(_, result)| result)
        .collect::<Vec<_>>();
    let passed = target_results
        .iter()
        .filter(|target| target.status == "passed")
        .count();
    let failed = target_results
        .iter()
        .filter(|target| target.status == "failed")
        .count();
    let skipped = target_results
        .iter()
        .filter(|target| target.status == "skipped")
        .count();
    let aggregate_status = if failed == 0 && skipped == 0 {
        "passed"
    } else if passed > 0 {
        "partial"
    } else {
        "failed"
    };
    let report = MatrixReport {
        schema_version: 1,
        matrix_run_id: matrix_id.to_string(),
        project: ctx.config.project_name(),
        project_root: ctx.paths.project_root.to_string_lossy().into_owned(),
        flow: args.flow.clone(),
        fixture: args.fixture.clone(),
        build: build.clone(),
        targets_requested: plan.len(),
        started_at: started_at.to_string(),
        ended_at: now_rfc3339(),
        duration_ms: started.elapsed().as_millis() as u64,
        aggregate_status: aggregate_status.to_string(),
        passed,
        failed,
        skipped,
        cleanup_policy: ValidationCleanupPolicy {
            sessions: if args.keep_sessions {
                "kept matrix-created sessions".to_string()
            } else {
                "released sessions created by this matrix; existing sessions untouched".to_string()
            },
            emulators: "not stopped by default".to_string(),
            note: "Parallel workers use independent devices; only sessions acquired by each worker are released."
                .to_string(),
        },
        parallel: args.parallel,
        target_results,
        warnings: warnings.clone(),
    };
    let report_path = matrix_dir.join("matrix.json");
    write_json_atomic(&report_path, &report)?;
    let mut response = success(
        "agent.matrix",
        serde_json::to_value(&report)?,
        format!(
            "Parallel matrix {}: {} passed, {} failed, {} skipped across {} target(s), {} worker(s).\nReport: {}\n",
            aggregate_status,
            passed,
            failed,
            skipped,
            plan.len(),
            args.parallel,
            report_path.display()
        ),
    );
    response.run_id = Some(matrix_id.to_string());
    response.artifacts = vec![
        ArtifactRef::new("matrix_report", report_path.to_string_lossy().into_owned())
            .with_description("Parallel matrix report"),
    ];
    if aggregate_status != "passed" {
        response.status = "failed";
        response.exit_code = if aggregate_status == "partial" {
            ErrorCode::PartialFailure.exit_code()
        } else {
            ErrorCode::CommandFailed.exit_code()
        };
    }
    Ok(response)
}

fn matrix_child_argv(
    ctx: &CliContext,
    args: &AgentMatrixArgs,
    target: &MatrixPlanTarget,
    owner: &str,
    apk_path: Option<&Path>,
    timeout: std::time::Duration,
) -> Vec<String> {
    let mut argv = vec![
        "--json".to_string(),
        "--project".to_string(),
        ctx.paths.project_root.to_string_lossy().into_owned(),
        "agent".to_string(),
        "matrix".to_string(),
        "--parallel".to_string(),
        "1".to_string(),
        "--owner".to_string(),
        owner.to_string(),
        "--timeout".to_string(),
        format!("{}s", timeout.as_secs().max(1)),
    ];
    match &target.target {
        MatrixTargetSpec::Avd { name } => {
            argv.push("--avd".to_string());
            argv.push(name.clone());
        }
        MatrixTargetSpec::Serial { serial } => {
            argv.push("--serial".to_string());
            argv.push(serial.clone());
        }
    }
    if let Some(profile) = &args.profile {
        argv.extend(["--profile".to_string(), profile.clone()]);
    }
    if let Some(ttl) = &args.ttl {
        argv.extend(["--ttl".to_string(), ttl.clone()]);
    }
    if args.keep_sessions {
        argv.push("--keep-sessions".to_string());
    }
    if args.install {
        argv.push("--install".to_string());
    }
    if let Some(apk) = apk_path {
        argv.extend(["--apk".to_string(), apk.to_string_lossy().into_owned()]);
    } else if let Some(variant) = &args.variant {
        argv.extend(["--variant".to_string(), variant.clone()]);
    }
    if let Some(flow) = &args.flow {
        argv.extend(["--flow".to_string(), flow.clone()]);
    }
    if let Some(fixture) = &args.fixture {
        argv.extend(["--fixture".to_string(), fixture.clone()]);
    }
    argv
}

async fn run_parallel_target(
    index: usize,
    target: MatrixPlanTarget,
    argv: Vec<String>,
    cwd: PathBuf,
    target_dir: PathBuf,
    timeout: std::time::Duration,
) -> (usize, MatrixPlanTarget, MatrixTargetResult, Value) {
    let started_at = now_rfc3339();
    let started = Instant::now();
    let mut result = MatrixTargetResult {
        target: target.target.clone(),
        status: "failed".to_string(),
        started_at: started_at.clone(),
        ended_at: started_at,
        duration_ms: 0,
        session_id: None,
        acquisition_mode: None,
        serial: None,
        api_level: target.api_level,
        model: target.model.clone(),
        health: None,
        health_warnings: Vec::new(),
        run_id: None,
        artifact_directory: None,
        failure_code: None,
        failure_message: None,
        cleanup: "not_applicable".to_string(),
        child_status: None,
        child_exit_code: None,
        child_report: None,
        warnings: Vec::new(),
    };
    let executable = std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "androidforge".to_string());
    let _ = ensure_dir(&target_dir);
    let label = slugify(target.target.label(), 40);
    let stdout_path = target_dir.join(format!("{:02}-{label}.stdout.json", index + 1));
    let stderr_path = target_dir.join(format!("{:02}-{label}.stderr.txt", index + 1));
    // Child output goes to files, not pipes. Emulators and other detached
    // grandchildren can inherit pipe handles on Windows, which makes reading
    // stdout until EOF hang long after the direct child exits. Waiting on the
    // process itself and then reading the files keeps the parent bounded.
    let envelope = match (
        std::fs::File::create(&stdout_path),
        std::fs::File::create(&stderr_path),
    ) {
        (Ok(stdout), Ok(stderr)) => {
            let spawned = tokio::process::Command::new(executable)
                .args(&argv)
                .current_dir(cwd)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::from(stdout))
                .stderr(std::process::Stdio::from(stderr))
                .kill_on_drop(true)
                .spawn();
            match spawned {
                Ok(mut child) => match tokio::time::timeout(timeout, child.wait()).await {
                    Ok(Ok(status)) => {
                        let stdout = std::fs::read_to_string(&stdout_path).unwrap_or_default();
                        let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
                        match serde_json::from_str::<Value>(stdout.trim()) {
                            Ok(envelope) => {
                                result.child_status = envelope
                                    .get("status")
                                    .and_then(Value::as_str)
                                    .map(str::to_string);
                                result.child_exit_code = envelope
                                    .get("exit_code")
                                    .and_then(Value::as_i64)
                                    .map(|code| code as i32);
                                if let Some(child) = envelope
                                    .get("result")
                                    .and_then(|value| value.get("target_results"))
                                    .and_then(Value::as_array)
                                    .and_then(|items| items.first())
                                    .cloned()
                                    && let Ok(child_result) =
                                        serde_json::from_value::<MatrixTargetResult>(child)
                                {
                                    result = child_result;
                                }
                                if status.success() && result.status == "passed" {
                                    result.status = "passed".to_string();
                                } else if !status.success() {
                                    result.status = "failed".to_string();
                                }
                                envelope
                            }
                            Err(error) => {
                                result.failure_code =
                                    Some("E_MATRIX_CHILD_OUTPUT_INVALID".to_string());
                                result.failure_message = Some(format!(
                                    "parallel child returned invalid JSON: {error}; stderr: {}",
                                    truncate(&stderr, 2000)
                                ));
                                json!({"status": "error", "error": {"code": "E_MATRIX_CHILD_OUTPUT_INVALID", "message": error.to_string()}})
                            }
                        }
                    }
                    Ok(Err(error)) => {
                        result.failure_code = Some(ErrorCode::CommandFailed.as_str().to_string());
                        result.failure_message =
                            Some(format!("parallel child wait failed: {error}"));
                        json!({"status": "error", "error": {"code": "E_COMMAND_FAILED", "message": error.to_string()}})
                    }
                    Err(_) => {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        result.failure_code = Some(ErrorCode::CommandTimeout.as_str().to_string());
                        result.failure_message = Some(format!(
                            "parallel child exceeded its {}s timeout and was killed",
                            timeout.as_secs()
                        ));
                        json!({"status": "error", "error": {"code": "E_COMMAND_TIMEOUT", "message": "parallel child timed out"}})
                    }
                },
                Err(error) => {
                    result.failure_code = Some(ErrorCode::CommandFailed.as_str().to_string());
                    result.failure_message =
                        Some(format!("parallel child failed to start: {error}"));
                    json!({"status": "error", "error": {"code": "E_COMMAND_FAILED", "message": error.to_string()}})
                }
            }
        }
        (stdout, stderr) => {
            let message = format!(
                "could not create parallel child log files: stdout={:?} stderr={:?}",
                stdout.err(),
                stderr.err()
            );
            result.failure_code = Some(ErrorCode::Internal.as_str().to_string());
            result.failure_message = Some(message.clone());
            json!({"status": "error", "error": {"code": "E_INTERNAL", "message": message}})
        }
    };
    result.ended_at = now_rfc3339();
    result.duration_ms = started.elapsed().as_millis() as u64;
    (index, target, result, envelope)
}

async fn build_plan(
    ctx: &CliContext,
    targets: &[MatrixTargetSpec],
) -> Result<Vec<MatrixPlanTarget>> {
    let avds = if targets
        .iter()
        .any(|target| matches!(target, MatrixTargetSpec::Avd { .. }))
    {
        ctx.emulator_manager()?.list_avds().await?
    } else {
        Vec::new()
    };
    let devices = if targets
        .iter()
        .any(|target| matches!(target, MatrixTargetSpec::Serial { .. }))
    {
        ctx.adb()?.devices_raw().await?
    } else {
        Vec::new()
    };
    let mut plan = Vec::new();
    for target in targets {
        let mut entry = MatrixPlanTarget {
            target: target.clone(),
            model: None,
            api_level: None,
        };
        match target {
            MatrixTargetSpec::Avd { name } => {
                if !avds.iter().any(|avd| avd.name == *name) {
                    return Err(ForgeError::new(
                        ErrorCode::MatrixTargetInvalid,
                        format!("AVD '{name}' was not found"),
                    )
                    .with_context(
                        "available_avds",
                        serde_json::json!(avds.iter().map(|avd| &avd.name).collect::<Vec<_>>()),
                    )
                    .with_suggestion("Run `androidforge avd list --json`."));
                }
            }
            MatrixTargetSpec::Serial { serial } => {
                let Some(device) = devices.iter().find(|device| &device.serial == serial) else {
                    return Err(ForgeError::new(
                        ErrorCode::MatrixTargetInvalid,
                        format!("device serial '{serial}' is not connected"),
                    )
                    .with_context(
                        "connected_devices",
                        serde_json::json!(
                            devices
                                .iter()
                                .map(|device| &device.serial)
                                .collect::<Vec<_>>()
                        ),
                    )
                    .with_suggestion("Run `androidforge devices list --json`."));
                };
                entry.model = device.model.clone();
                entry.api_level = device.api_level;
            }
        }
        plan.push(entry);
    }
    Ok(plan)
}

fn child_argv(
    ctx: &CliContext,
    args: &AgentMatrixArgs,
    owner: &str,
    session_id: Option<&str>,
    apk_path: Option<&Path>,
) -> Vec<String> {
    let mut argv = vec![
        "--json".to_string(),
        "--project".to_string(),
        ctx.paths.project_root.to_string_lossy().into_owned(),
        "agent".to_string(),
        "test".to_string(),
    ];
    if let Some(session_id) = session_id {
        argv.extend(["--session-id".to_string(), session_id.to_string()]);
    }
    if let Some(apk) = apk_path {
        argv.extend(["--apk".to_string(), apk.to_string_lossy().into_owned()]);
    } else if let Some(variant) = &args.variant {
        argv.extend(["--variant".to_string(), variant.clone()]);
    }
    if args.install {
        argv.push("--install".to_string());
    }
    if let Some(flow) = &args.flow {
        argv.extend(["--flow".to_string(), flow.clone()]);
    }
    if let Some(fixture) = &args.fixture {
        argv.extend(["--fixture".to_string(), fixture.clone()]);
    }
    argv.extend(["--owner".to_string(), owner.to_string()]);
    argv
}

async fn release_matrix_session(ctx: &CliContext, session_id: &str, owner: &str) -> Result<String> {
    let session = match Session::load(&ctx.paths, session_id) {
        Ok(session) => session,
        Err(error) => return Ok(format!("not_released: {}", error.message())),
    };
    if session.owner != owner {
        return Ok("not_released: ownership changed".to_string());
    }
    if matches!(
        session.state,
        SessionState::Released | SessionState::Failed | SessionState::Abandoned
    ) {
        return Ok("already_released".to_string());
    }
    let mut reusable = ctx.clone();
    reusable.session_arg = Some(session_id.to_string());
    match session::session(
        &reusable,
        &SessionCommand::Release(SessionReleaseArgs {
            stop: false,
            stop_attached: false,
            keep_prepared: true,
            owner: Some(owner.to_string()),
            force: false,
        }),
    )
    .await
    {
        Ok(_) => Ok("released".to_string()),
        Err(error) => Ok(format!("release_failed: {}", error.message())),
    }
}

fn write_target_report(dir: &Path, index: usize, target: &MatrixTargetResult) -> Result<()> {
    let slug = slugify(target.target.label(), 40);
    write_json_atomic(
        &dir.join("targets")
            .join(format!("{:02}-{slug}-result.json", index + 1)),
        target,
    )
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect::<String>() + "\n[truncated]"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_value() -> Value {
        json!({
            "schema_version": 1,
            "matrix_run_id": "matrix-20261118120000-test",
            "project": "Example",
            "project_root": "A:/example",
            "flow": null,
            "fixture": null,
            "build": {},
            "targets_requested": 2,
            "started_at": "2026-11-18T12:00:00Z",
            "ended_at": "2026-11-18T12:00:01Z",
            "duration_ms": 1000,
            "aggregate_status": "passed",
            "passed": 2,
            "failed": 0,
            "skipped": 0,
            "cleanup_policy": {
                "sessions": "released",
                "emulators": "not stopped by default",
                "note": "test"
            },
            "target_results": [],
            "warnings": []
        })
    }

    #[test]
    fn matrix_report_serializes_parallel_and_defaults_legacy_reports() {
        let value = report_value();
        let report: MatrixReport = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(report.parallel, 1);
        assert_eq!(report.passed, 2);

        let mut parallel = value.clone();
        parallel["parallel"] = json!(4);
        let report: MatrixReport = serde_json::from_value(parallel).unwrap();
        assert_eq!(report.parallel, 4);

        let mut legacy = value;
        legacy.as_object_mut().unwrap().remove("parallel");
        let report: MatrixReport = serde_json::from_value(legacy).unwrap();
        assert_eq!(report.parallel, 1);
    }
}
