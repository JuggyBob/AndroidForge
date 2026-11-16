//! Transparent high-level agent pipeline.
//!
//! `agent test` is deliberately not a test language. It composes existing CLI
//! operations, records every stage, and never hides a device mutation.

use std::path::{Path, PathBuf};
use std::time::Instant;

use androidforge_app::{ApkInfo, AppStates};
use androidforge_core::diagnosis::explain_report;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::{ArtifactRef, Device};
use androidforge_core::output::CommandSuccess;
use androidforge_core::run::RunReport;
use androidforge_core::session::Session;
use androidforge_core::util::{now_rfc3339, read_json, write_json_atomic};
use serde_json::{Value, json};

use crate::cli::{
    AgentCommand, AgentTestArgs, AppInstallArgs, Cli, ConformanceCommand, ConformanceRunArgs,
    FixtureCommand, FixtureRunArgs, SessionEnsureArgs,
};
use crate::commands::common::success;
use crate::commands::{app, automation, session};
use crate::context::CliContext;

pub async fn agent(cli: &Cli, command: &AgentCommand) -> Result<CommandSuccess> {
    match command {
        AgentCommand::Test(args) => test(cli, args).await,
    }
}

struct RunState {
    stages: Vec<Value>,
    warnings: Vec<String>,
    artifacts: Vec<ArtifactRef>,
    run_id: Option<String>,
    session: Option<Session>,
    device: Option<Device>,
    apk: Option<ApkInfo>,
    installed_matches: Option<bool>,
    failure: Option<(String, ForgeError)>,
}

impl RunState {
    fn new() -> Self {
        Self {
            stages: Vec::new(),
            warnings: Vec::new(),
            artifacts: Vec::new(),
            run_id: None,
            session: None,
            device: None,
            apk: None,
            installed_matches: None,
            failure: None,
        }
    }

    fn stage(
        &mut self,
        name: &str,
        status: &str,
        started: Instant,
        argv: Vec<String>,
        details: Value,
    ) {
        self.stages.push(json!({
            "name": name,
            "status": status,
            "duration_ms": started.elapsed().as_millis() as u64,
            "command_argv": argv,
            "details": details,
        }));
    }

    fn fail(&mut self, name: &str, started: Instant, argv: Vec<String>, error: ForgeError) {
        if let Some(run_id) = error.body().context.get("run_id").and_then(Value::as_str) {
            self.run_id = Some(run_id.to_string());
        }
        self.stages.push(json!({
            "name": name,
            "status": "failed",
            "duration_ms": started.elapsed().as_millis() as u64,
            "command_argv": argv,
            "details": {
                "code": error.body().code.as_str(),
                "message": error.body().message,
            },
        }));
        self.failure = Some((name.to_string(), error));
    }

    fn absorb(&mut self, success: &CommandSuccess) {
        if let Some(run_id) = &success.run_id {
            self.run_id = Some(run_id.clone());
        }
        self.artifacts.extend(success.artifacts.clone());
        if let Some(warnings) = success.result.get("warnings").and_then(Value::as_array) {
            self.warnings.extend(
                warnings
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string),
            );
        }
    }
}

fn stage_argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_string()).collect()
}

fn absolute(ctx: &CliContext, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        ctx.paths.cwd.join(path)
    }
}

fn config_argv(ctx: &CliContext) -> Vec<String> {
    vec![
        "androidforge".to_string(),
        "config".to_string(),
        "validate".to_string(),
        "--project".to_string(),
        ctx.paths.project_root.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]
}

fn ensure_argv(args: &AgentTestArgs, avd: Option<&str>, serial: Option<&str>) -> Vec<String> {
    let mut argv = vec![
        "androidforge".to_string(),
        "session".to_string(),
        "ensure".to_string(),
    ];
    if let Some(avd) = avd {
        argv.extend(["--avd".to_string(), avd.to_string()]);
    }
    if let Some(serial) = serial {
        argv.extend(["--serial".to_string(), serial.to_string()]);
    }
    if let Some(owner) = &args.owner {
        argv.extend(["--owner".to_string(), owner.clone()]);
    }
    if let Some(profile) = &args.profile {
        argv.extend(["--profile".to_string(), profile.clone()]);
    }
    if let Some(ttl) = &args.ttl {
        argv.extend(["--ttl".to_string(), ttl.clone()]);
    }
    argv.push("--json".to_string());
    argv
}

fn install_argv(session: &Session, apk: &Path) -> Vec<String> {
    let mut argv = vec![
        "androidforge".to_string(),
        "app".to_string(),
        "install".to_string(),
        apk.to_string_lossy().into_owned(),
    ];
    argv.extend([
        "--session".to_string(),
        session.id.clone(),
        "--json".to_string(),
    ]);
    argv
}

fn fixture_argv(session: &Session, name: &str) -> Vec<String> {
    vec![
        "androidforge".to_string(),
        "fixture".to_string(),
        "run".to_string(),
        name.to_string(),
        "--session".to_string(),
        session.id.clone(),
        "--json".to_string(),
    ]
}

fn conformance_argv(session: &Session, flow: Option<&Path>) -> Vec<String> {
    let mut argv = vec![
        "androidforge".to_string(),
        "conformance".to_string(),
        "run".to_string(),
        "--session".to_string(),
        session.id.clone(),
        "--json".to_string(),
    ];
    if let Some(flow) = flow {
        argv.push("--flow".to_string());
        argv.push(flow.to_string_lossy().into_owned());
    }
    argv
}

fn compare_build(
    session: &Session,
    apk: &ApkInfo,
    installed: Option<&AppStates>,
) -> (bool, String, Vec<String>) {
    let Some(installed) = installed else {
        return (
            false,
            "unknown".to_string(),
            vec!["The installed package state could not be read.".to_string()],
        );
    };
    if !installed.installed {
        return (
            false,
            "not_installed".to_string(),
            vec![format!(
                "Package {} is not installed on the device.",
                apk.package.as_deref().unwrap_or("-")
            )],
        );
    }
    let mut notes = Vec::new();
    if installed.version_code.is_some()
        && apk.version_code.is_some()
        && installed.version_code != apk.version_code
    {
        notes.push(format!(
            "Installed version code {:?} differs from APK version code {:?}.",
            installed.version_code, apk.version_code
        ));
    }
    if installed.version_name.is_some()
        && apk.version_name.is_some()
        && installed.version_name != apk.version_name
    {
        notes.push(format!(
            "Installed version name {:?} differs from APK version name {:?}.",
            installed.version_name, apk.version_name
        ));
    }
    if !notes.is_empty() {
        return (false, "different_version".to_string(), notes);
    }
    let Some(build) = &session.build else {
        return (
            false,
            "no_recorded_build_identity".to_string(),
            vec![
                "The session has no recorded install identity, so the installed APK hash cannot be proven."
                    .to_string(),
            ],
        );
    };
    if build.package.as_deref() != apk.package.as_deref() {
        return (
            false,
            "different_package_identity".to_string(),
            vec![format!(
                "The recorded session build package {:?} differs from APK package {:?}.",
                build.package, apk.package
            )],
        );
    }
    let recorded = build
        .installed_apk_sha256
        .as_deref()
        .or(build.apk_sha256.as_deref());
    match recorded {
        Some(recorded) if recorded == apk.sha256 => (true, "sha256".to_string(), notes),
        Some(recorded) => (
            false,
            "different_build".to_string(),
            vec![format!(
                "The recorded installed APK SHA-256 {recorded} differs from the candidate APK {}.",
                apk.sha256
            )],
        ),
        None => (
            false,
            "no_recorded_hash".to_string(),
            vec!["The recorded build identity has no APK hash.".to_string()],
        ),
    }
}

fn explain_run(ctx: &CliContext, run_id: &str) -> Option<Value> {
    let path = ctx
        .paths
        .artifacts_dir
        .join("runs")
        .join(run_id)
        .join("report.json");
    let report: RunReport = read_json(&path).ok()?;
    serde_json::to_value(explain_report(&report)).ok()
}

fn failure_json(stage: &str, error: &ForgeError) -> Value {
    json!({
        "stage": stage,
        "code": error.body().code.as_str(),
        "message": error.body().message,
        "recoverable": error.body().recoverable,
        "suggestions": error.body().suggestions,
        "context": error.body().context,
        "exit_code": error.exit_code(),
        "destructive": false,
    })
}

fn finalize(ctx: &CliContext, mut state: RunState) -> Result<CommandSuccess> {
    let mut next_actions = Vec::new();
    let failure = if let Some((stage, error)) = &state.failure {
        let mut value = failure_json(stage, error);
        if let Some(run_id) = state.run_id.as_deref()
            && let Some(explanation) = explain_run(ctx, run_id)
        {
            next_actions = explanation
                .get("next_actions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            value["explanation"] = explanation;
        }
        Some(value)
    } else {
        None
    };

    let explanation_started = Instant::now();
    if let Some(failure) = &failure {
        if failure.get("explanation").is_some() {
            state.stage(
                "explain_failure",
                "passed",
                explanation_started,
                vec![
                    "androidforge".to_string(),
                    "report".to_string(),
                    "explain".to_string(),
                    state.run_id.clone().unwrap_or_else(|| "latest".to_string()),
                    "--json".to_string(),
                ],
                json!({
                    "category": failure["explanation"]["diagnosis"]["category"],
                    "confidence": failure["explanation"]["diagnosis"]["confidence"],
                    "fingerprint": failure["explanation"]["fingerprint"],
                }),
            );
        } else {
            state.stage(
                "explain_failure",
                "skipped",
                explanation_started,
                Vec::new(),
                json!({"reason": "The failure occurred before a structured run report existed."}),
            );
        }
    } else {
        state.stage(
            "explain_failure",
            "skipped",
            explanation_started,
            Vec::new(),
            json!({"reason": "The pipeline succeeded; there is no failure to explain."}),
        );
    }

    let final_status = if state.failure.is_some() {
        "failed"
    } else {
        "success"
    };
    let session_value = state
        .session
        .as_ref()
        .map(|session| {
            json!({
                "id": session.id,
                "owner": session.owner,
                "state": session.state,
                "kind": session.kind,
                "serial": session.serial,
                "avd": session.avd,
                "profile": session.profile,
                "lease_expires_at": session.lease_expires_at,
                "lease_expired": session.lease_expired(),
            })
        })
        .unwrap_or(Value::Null);
    let device_value = state
        .device
        .as_ref()
        .map(|device| json!(device.compact()))
        .unwrap_or(Value::Null);
    let mut result = json!({
        "schema_version": 1,
        "pipeline": "agent.test",
        "dry_run": ctx.dry_run,
        "generated_at": now_rfc3339(),
        "project": {
            "name": ctx.config.project_name(),
            "root": ctx.paths.project_root,
            "package": ctx.config.package(),
            "debug_package": ctx.config.debug_package(),
        },
        "session": session_value,
        "device": device_value,
        "apk": state.apk,
        "stages": state.stages,
        "installed_build_matches_apk": state.installed_matches,
        "final_status": final_status,
        "failure": failure,
        "run_id": state.run_id,
        "artifacts": state.artifacts.clone(),
        "warnings": state.warnings.clone(),
        "next_actions": next_actions,
    });

    let report_started = Instant::now();
    let report_path = ctx.paths.state_dir.join("agent-tests").join(format!(
        "agent-test-{}-{}.json",
        chrono::Utc::now().format("%Y%m%d%H%M%S%3f"),
        androidforge_core::util::short_id()
    ));
    match write_json_atomic(&report_path, &result) {
        Ok(()) => {
            result["report_path"] = json!(report_path.to_string_lossy());
            state.stage(
                "consolidated_report",
                "passed",
                report_started,
                Vec::new(),
                json!({
                    "path": report_path.to_string_lossy(),
                    "kind": "inline_agent_test_result",
                }),
            );
        }
        Err(error) => {
            state.warnings.push(format!(
                "Could not write the consolidated agent test report: {}",
                error.message()
            ));
            state.stage(
                "consolidated_report",
                "warning",
                report_started,
                Vec::new(),
                json!({"reason": error.message()}),
            );
        }
    }
    result["stages"] = json!(state.stages);
    result["warnings"] = json!(state.warnings);

    let human = if final_status == "success" {
        format!(
            "Agent test {}: {} stage(s).\n{}",
            if ctx.dry_run { "dry run" } else { "completed" },
            state.stages.len(),
            state
                .session
                .as_ref()
                .map(|session| format!(
                    "session: {}\ndevice: {}\n",
                    session.id,
                    session.serial.as_deref().unwrap_or("-")
                ))
                .unwrap_or_default()
        )
    } else {
        match &state.failure {
            Some((stage, error)) => format!(
                "Agent test failed at {stage}: {} ({})\n",
                error.message(),
                error.body().code.as_str()
            ),
            None => "Agent test failed.\n".to_string(),
        }
    };
    let mut response = success(
        "agent.test",
        result,
        format!("{human}Consolidated report: {}\n", report_path.display()),
    );
    response.exit_code = state
        .failure
        .as_ref()
        .map(|(_, error)| error.exit_code())
        .unwrap_or(0);
    response.session_id = state.session.as_ref().map(|session| session.id.clone());
    response.device_serial = state.device.as_ref().map(|device| device.serial.clone());
    response.run_id = state.run_id.clone();
    response.artifacts = state.artifacts.clone();
    Ok(response)
}

fn pipeline_failure(cli: &Cli, stage: &str, error: ForgeError) -> Result<CommandSuccess> {
    let mut state = RunState::new();
    let started = Instant::now();
    state.fail(stage, started, Vec::new(), error);
    let mut ctx = match CliContext::new(cli) {
        Ok(ctx) => ctx,
        Err(context_error) => {
            let result = json!({
                "schema_version": 1,
                "pipeline": "agent.test",
                "dry_run": cli.dry_run,
                "generated_at": now_rfc3339(),
                "final_status": "failed",
                "failure": failure_json(stage, &context_error),
                "stages": state.stages,
                "warnings": ["The project could not be loaded, so no session or device was inspected."],
                "next_actions": [],
            });
            let mut response = success(
                "agent.test",
                result,
                format!(
                    "Agent test failed at {stage}: {}\n",
                    context_error.message()
                ),
            );
            response.exit_code = context_error.exit_code();
            return Ok(response);
        }
    };
    if cli.json {
        // Keep stdout machine-readable; diagnostics belong on stderr, which is
        // handled by the top-level JSON envelope.
        ctx.quiet = true;
    }
    finalize(&ctx, state)
}

async fn test(cli: &Cli, args: &AgentTestArgs) -> Result<CommandSuccess> {
    if args.apk.is_none() && args.flow.is_none() && args.fixture.is_none() {
        return pipeline_failure(
            cli,
            "validate_request",
            ForgeError::invalid_argument(
                "agent test needs at least one of --apk, --flow or --fixture",
            )
            .with_suggestion(
                "Provide an APK to install or inspect, a conformance flow, or a fixture.",
            )
            .with_suggestion("Use `androidforge context --json` for a read-only snapshot."),
        );
    }
    let mut ctx = match CliContext::new(cli) {
        Ok(ctx) => ctx,
        Err(error) => {
            return pipeline_failure(cli, "validate_config", error);
        }
    };
    let mut state = RunState::new();
    let validate_started = Instant::now();
    let validation = ctx.config.validate();
    if !validation.valid {
        let error = ForgeError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "project configuration has {} error(s)",
                validation
                    .issues
                    .iter()
                    .filter(|issue| {
                        issue.severity == androidforge_core::config::Severity::Error
                    })
                    .count()
            ),
        )
        .with_context(
            "issues",
            serde_json::to_value(&validation.issues).unwrap_or(Value::Null),
        )
        .with_suggestion("Run `androidforge config validate --json` and fix the reported fields.");
        state.fail(
            "validate_config",
            validate_started,
            config_argv(&ctx),
            error,
        );
        return finalize(&ctx, state);
    }
    state.stage(
        "validate_config",
        "passed",
        validate_started,
        config_argv(&ctx),
        json!({
            "project": ctx.config.project_name(),
            "package": ctx.config.package(),
            "config": ctx.paths.project_config_path,
            "issues": validation.issues.len(),
        }),
    );

    let target_avd = args
        .avd
        .clone()
        .or_else(|| ctx.config.file.project.default_avd.clone());
    let target_serial = args
        .serial
        .clone()
        .or_else(|| ctx.config.file.project.default_device.clone());

    if ctx.dry_run {
        return dry_run(&ctx, args, state, target_avd, target_serial).await;
    }

    let flow: Option<Option<PathBuf>> = match args.flow.as_deref() {
        None => None,
        Some(value) if value.eq_ignore_ascii_case("conformance") || value == "default" => {
            Some(None)
        }
        Some(value) => Some(Some(absolute(&ctx, Path::new(value)))),
    };

    'pipeline: {
        let session_started = Instant::now();
        let explicit_session = args.session_id.clone().or_else(|| ctx.session_arg.clone());
        let session = if let Some(id) = explicit_session {
            ctx.session_arg = Some(id.clone());
            match ctx.load_session(true) {
                Ok(Some(session)) => session,
                Ok(None) => {
                    state.fail(
                        "resolve_session",
                        session_started,
                        stage_argv(&[
                            "androidforge",
                            "session",
                            "inspect",
                            "--session",
                            &id,
                            "--json",
                        ]),
                        ForgeError::new(ErrorCode::SessionNotFound, "no session was specified")
                            .with_suggestion(
                                "Pass --session-id <ID> or run `androidforge session ensure`.",
                            ),
                    );
                    break 'pipeline;
                }
                Err(error) => {
                    state.fail(
                        "resolve_session",
                        session_started,
                        stage_argv(&[
                            "androidforge",
                            "session",
                            "inspect",
                            "--session",
                            &id,
                            "--json",
                        ]),
                        error,
                    );
                    break 'pipeline;
                }
            }
        } else {
            match ctx.load_session(true) {
                Ok(Some(session)) => {
                    if session.lease_expired() {
                        state.warnings.push(format!(
                            "Session {} has an expired advisory lease. Work continues; pass --ttl through `session ensure` to renew it.",
                            session.id
                        ));
                    }
                    session
                }
                _ => {
                    if target_avd.is_none() && target_serial.is_none() {
                        state.fail(
                            "resolve_session",
                            session_started,
                            ensure_argv(args, None, None),
                            ForgeError::new(
                                ErrorCode::ConfigInvalid,
                                "no session is active and no AVD or serial is configured for session ensure",
                            )
                            .with_suggestion("Pass --avd <AVD> or --serial <SERIAL>.")
                            .with_suggestion("Configure project.default_avd or run `androidforge session start` once."),
                        );
                        break 'pipeline;
                    }
                    let ensure_args = SessionEnsureArgs {
                        avd: target_avd.clone(),
                        serial: target_serial.clone(),
                        owner: args.owner.clone(),
                        profile: args.profile.clone(),
                        ttl: args.ttl.clone(),
                        headless: true,
                        prepare: false,
                    };
                    let acquisition = match session::ensure(&ctx, &ensure_args).await {
                        Ok(acquisition) => acquisition,
                        Err(error) => {
                            state.fail(
                                "resolve_session",
                                session_started,
                                ensure_argv(args, target_avd.as_deref(), target_serial.as_deref()),
                                error,
                            );
                            break 'pipeline;
                        }
                    };
                    let Some(id) = acquisition
                        .result
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                    else {
                        state.fail(
                            "resolve_session",
                            session_started,
                            ensure_argv(args, target_avd.as_deref(), target_serial.as_deref()),
                            ForgeError::internal("session ensure did not return a session id"),
                        );
                        break 'pipeline;
                    };
                    ctx.session_arg = Some(id.clone());
                    state
                        .warnings
                        .extend(acquisition_human_warnings(&acquisition.result));
                    state.stage(
                        "resolve_session",
                        "passed",
                        session_started,
                        ensure_argv(args, target_avd.as_deref(), target_serial.as_deref()),
                        json!({
                            "session_id": id,
                            "acquisition_mode": acquisition.result.get("acquisition_mode"),
                            "readiness": acquisition.result.get("readiness"),
                            "lease_expires_at": acquisition.result.get("lease_expires_at"),
                        }),
                    );
                    match ctx.load_session(true) {
                        Ok(Some(session)) => session,
                        Ok(None) => {
                            state.fail(
                                "resolve_session",
                                session_started,
                                ensure_argv(args, target_avd.as_deref(), target_serial.as_deref()),
                                ForgeError::new(ErrorCode::SessionNotFound, "the ensured session could not be loaded")
                                    .with_suggestion("Run `androidforge session list --json` and inspect the session."),
                            );
                            break 'pipeline;
                        }
                        Err(error) => {
                            state.fail(
                                "resolve_session",
                                session_started,
                                ensure_argv(args, target_avd.as_deref(), target_serial.as_deref()),
                                error,
                            );
                            break 'pipeline;
                        }
                    }
                }
            }
        };
        if !state
            .stages
            .iter()
            .any(|stage| stage["name"] == "resolve_session")
        {
            state.stage(
                "resolve_session",
                "passed",
                session_started,
                stage_argv(&[
                    "androidforge",
                    "session",
                    "inspect",
                    "--session",
                    session.id.as_str(),
                    "--json",
                ]),
                json!({
                    "session_id": session.id,
                    "acquisition_mode": "existing_session",
                    "lease_expires_at": session.lease_expires_at,
                    "lease_expired": session.lease_expired(),
                }),
            );
        }
        state.session = Some(session.clone());

        let device_started = Instant::now();
        let device = match ctx.resolve_device(Some(&session)).await {
            Ok(device) => device,
            Err(error) => {
                state.fail(
                    "verify_device",
                    device_started,
                    stage_argv(&["androidforge", "devices", "list", "--json"]),
                    error,
                );
                break 'pipeline;
            }
        };
        state.stage(
            "verify_device",
            "passed",
            device_started,
            stage_argv(&["androidforge", "devices", "list", "--json"]),
            json!({
                "serial": device.serial,
                "state": device.state,
                "boot_completed": device.boot_completed,
                "api_level": device.api_level,
            }),
        );
        state.device = Some(device.clone());

        let apk_started = Instant::now();
        let apk_path = args.apk.as_ref().map(|path| absolute(&ctx, path));
        let apk = if let Some(path) = &apk_path {
            let argv = vec![
                "androidforge".to_string(),
                "apk".to_string(),
                "inspect".to_string(),
                path.to_string_lossy().into_owned(),
                "--json".to_string(),
            ];
            match ctx.inspector().inspect(path, false).await {
                Ok(apk) => {
                    state.stage(
                        "inspect_apk",
                        "passed",
                        apk_started,
                        argv,
                        json!({
                            "path": apk.path,
                            "package": apk.package,
                            "version_name": apk.version_name,
                            "version_code": apk.version_code,
                            "sha256": apk.sha256,
                            "signature_verified": apk.signature.verified,
                            "certificate_sha256": apk.signature.cert_sha256,
                        }),
                    );
                    state.apk = Some(apk.clone());
                    Some(apk)
                }
                Err(error) => {
                    state.fail("inspect_apk", apk_started, argv, error);
                    break 'pipeline;
                }
            }
        } else {
            state.stage(
                "inspect_apk",
                "skipped",
                apk_started,
                Vec::new(),
                json!({"reason": "No --apk was supplied."}),
            );
            None
        };

        let compare_started = Instant::now();
        let needs_install = if let Some(apk) = &apk {
            let Some(package) = apk.package.as_deref() else {
                state.fail(
                    "compare_build",
                    compare_started,
                    stage_argv(&["androidforge", "context", "--json"]),
                    ForgeError::new(
                        ErrorCode::ApkInvalid,
                        "the APK does not declare a package name",
                    )
                    .with_suggestion("Rebuild the APK with a valid applicationId."),
                );
                break 'pipeline;
            };
            let app = match ctx.app_manager() {
                Ok(app) => app,
                Err(error) => {
                    state.fail("compare_build", compare_started, Vec::new(), error);
                    break 'pipeline;
                }
            };
            let installed = app.status(&device.serial, package).await.ok();
            let (matches, proof, notes) = compare_build(&session, apk, installed.as_ref());
            state.installed_matches = Some(matches);
            state.stage(
                "compare_build",
                "passed",
                compare_started,
                stage_argv(&[
                    "androidforge",
                    "app",
                    "status",
                    package,
                    "--session",
                    session.id.as_str(),
                    "--json",
                ]),
                json!({
                    "matches": matches,
                    "proof": proof,
                    "notes": notes,
                    "installed": installed.as_ref().map(|status| json!({
                        "installed": status.installed,
                        "version_name": status.version_name,
                        "version_code": status.version_code,
                        "process_exists": status.process_exists,
                    })),
                }),
            );
            !matches
        } else {
            state.stage(
                "compare_build",
                "skipped",
                compare_started,
                Vec::new(),
                json!({"reason": "No --apk was supplied."}),
            );
            false
        };

        let install_started = Instant::now();
        if let Some(apk) = &apk
            && needs_install
        {
            let path = apk_path.clone().unwrap_or_else(|| PathBuf::from(&apk.path));
            if !args.install {
                state.fail(
                    "install_apk",
                    install_started,
                    install_argv(&session, &path),
                    ForgeError::new(
                        ErrorCode::InvalidArgument,
                        "the installed build cannot be proven current; installing requires --install",
                    )
                    .with_context("apk", apk.path.clone())
                    .with_context("installed_matches", false)
                    .with_suggestion(
                        "Pass --install to install this APK on the selected session.",
                    )
                    .with_suggestion(
                        "Plain install preserves app data; AndroidForge never clears data in this pipeline.",
                    ),
                );
                break 'pipeline;
            }
            let install_args = AppInstallArgs {
                apk: path.clone(),
                fresh: false,
                downgrade: false,
                no_verify: false,
                expected_package: None,
                expected_cert_sha256: None,
                timeout: None,
            };
            match app::install(&ctx, &install_args).await {
                Ok(installed) => {
                    state.absorb(&installed);
                    state.installed_matches = Some(true);
                    state.stage(
                        "install_apk",
                        "passed",
                        install_started,
                        install_argv(&session, &path),
                        json!({
                            "installed": true,
                            "fresh": false,
                            "data_preserved": true,
                            "run_id": installed.run_id,
                            "build": installed.result.get("build"),
                        }),
                    );
                }
                Err(error) => {
                    state.fail(
                        "install_apk",
                        install_started,
                        install_argv(&session, &path),
                        error,
                    );
                    break 'pipeline;
                }
            }
        } else {
            state.stage(
                "install_apk",
                "skipped",
                install_started,
                Vec::new(),
                json!({
                    "reason": if apk.is_none() {
                        "No --apk was supplied."
                    } else {
                        "The installed build already matches the APK."
                    }
                }),
            );
        }

        let fixture_started = Instant::now();
        if let Some(name) = &args.fixture {
            let command = FixtureCommand::Run(FixtureRunArgs {
                name: name.clone(),
                phase: Vec::new(),
                apk: apk_path.clone(),
                vars: Vec::new(),
            });
            match automation::fixture(&ctx, &command).await {
                Ok(result) => {
                    state.absorb(&result);
                    state.stage(
                        "run_fixture",
                        "passed",
                        fixture_started,
                        fixture_argv(&session, name),
                        json!({
                            "fixture": name,
                            "run_id": result.run_id,
                            "status": result.result.get("status"),
                        }),
                    );
                }
                Err(error) => {
                    if let Some(run_id) = error.body().context.get("run_id").and_then(Value::as_str)
                    {
                        state.run_id = Some(run_id.to_string());
                    }
                    state.fail(
                        "run_fixture",
                        fixture_started,
                        fixture_argv(&session, name),
                        error,
                    );
                    break 'pipeline;
                }
            }
        } else {
            state.stage(
                "run_fixture",
                "skipped",
                fixture_started,
                Vec::new(),
                json!({"reason": "No --fixture was supplied."}),
            );
        }

        let conformance_started = Instant::now();
        if let Some(flow_override) = &flow {
            let flow_path = flow_override.as_deref();
            let command = ConformanceCommand::Run(ConformanceRunArgs {
                flow: flow_path.map(Path::to_path_buf),
            });
            match automation::conformance(&ctx, &command).await {
                Ok(result) => {
                    state.absorb(&result);
                    state.stage(
                        "run_conformance",
                        "passed",
                        conformance_started,
                        conformance_argv(&session, flow_path),
                        json!({
                            "run_id": result.run_id,
                            "passed": result.result.get("passed"),
                            "checks": result.result.get("checks"),
                        }),
                    );
                }
                Err(error) => {
                    if let Some(run_id) = error.body().context.get("run_id").and_then(Value::as_str)
                    {
                        state.run_id = Some(run_id.to_string());
                    }
                    state.fail(
                        "run_conformance",
                        conformance_started,
                        conformance_argv(&session, flow_path),
                        error,
                    );
                    break 'pipeline;
                }
            }
        } else {
            state.stage(
                "run_conformance",
                "skipped",
                conformance_started,
                Vec::new(),
                json!({"reason": "No --flow was supplied."}),
            );
        }
    }

    finalize(&ctx, state)
}

fn acquisition_human_warnings(result: &Value) -> Vec<String> {
    result
        .get("warnings")
        .and_then(Value::as_array)
        .map(|warnings| {
            warnings
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

async fn dry_run(
    ctx: &CliContext,
    args: &AgentTestArgs,
    mut state: RunState,
    target_avd: Option<String>,
    target_serial: Option<String>,
) -> Result<CommandSuccess> {
    let explicit_session = args.session_id.clone().or_else(|| ctx.session_arg.clone());
    let session = if let Some(id) = explicit_session {
        ctx.load_session(false)
            .ok()
            .flatten()
            .or_else(|| Session::load(&ctx.paths, &id).ok())
    } else {
        ctx.session_id()
            .ok()
            .flatten()
            .and_then(|id| Session::load(&ctx.paths, &id).ok())
    };
    if let Some(session) = &session {
        state.session = Some(session.clone());
        state.stage(
            "resolve_session",
            "planned",
            Instant::now(),
            stage_argv(&[
                "androidforge",
                "session",
                "inspect",
                "--session",
                session.id.as_str(),
                "--json",
            ]),
            json!({
                "session_id": session.id,
                "acquisition_mode": "existing_session",
                "readiness": "would_verify",
                "lease_expires_at": session.lease_expires_at,
                "lease_expired": session.lease_expired(),
            }),
        );
    } else if target_avd.is_some() || target_serial.is_some() {
        state.stage(
            "resolve_session",
            "planned",
            Instant::now(),
            ensure_argv(args, target_avd.as_deref(), target_serial.as_deref()),
            json!({
                "acquisition_mode": "would_ensure",
                "avd": target_avd,
                "serial": target_serial,
                "would_check_existing_session": true,
                "would_launch_emulator": target_serial.is_none(),
            }),
        );
    } else {
        state.stage(
            "resolve_session",
            "failed",
            Instant::now(),
            Vec::new(),
            json!({
                "reason": "No session is active and no AVD or serial is configured.",
            }),
        );
    }
    state.stage(
        "verify_device",
        "planned",
        Instant::now(),
        stage_argv(&["androidforge", "devices", "list", "--json"]),
        json!({"would": "verify the session device is operational and fully booted"}),
    );

    let apk_started = Instant::now();
    let apk_path = args.apk.as_ref().map(|path| absolute(ctx, path));
    let apk = if let Some(path) = &apk_path {
        match ctx.inspector().inspect(path, false).await {
            Ok(apk) => {
                state.stage(
                    "inspect_apk",
                    "passed",
                    apk_started,
                    vec![
                        "androidforge".to_string(),
                        "apk".to_string(),
                        "inspect".to_string(),
                        path.to_string_lossy().into_owned(),
                        "--json".to_string(),
                    ],
                    json!({
                        "path": apk.path,
                        "package": apk.package,
                        "version_name": apk.version_name,
                        "version_code": apk.version_code,
                        "sha256": apk.sha256,
                    }),
                );
                state.apk = Some(apk.clone());
                Some(apk)
            }
            Err(error) => {
                state.fail(
                    "inspect_apk",
                    apk_started,
                    vec![
                        "androidforge".to_string(),
                        "apk".to_string(),
                        "inspect".to_string(),
                        path.to_string_lossy().into_owned(),
                        "--json".to_string(),
                    ],
                    error,
                );
                return finalize(ctx, state);
            }
        }
    } else {
        state.stage(
            "inspect_apk",
            "skipped",
            apk_started,
            Vec::new(),
            json!({"reason": "No --apk was supplied."}),
        );
        None
    };

    state.stage(
        "compare_build",
        "planned",
        Instant::now(),
        apk.as_ref()
            .and_then(|apk| apk.package.as_deref())
            .map(|package| stage_argv(&["androidforge", "app", "status", package, "--json"]))
            .unwrap_or_default(),
        json!({
            "would": "compare installed version and recorded APK SHA-256 identity",
            "requires_device": true,
            "candidate_sha256": apk.as_ref().map(|apk| &apk.sha256),
        }),
    );
    state.stage(
        "install_apk",
        "planned",
        Instant::now(),
        apk_path
            .as_ref()
            .map(|path| {
                vec![
                    "androidforge".to_string(),
                    "app".to_string(),
                    "install".to_string(),
                    path.to_string_lossy().into_owned(),
                    "--json".to_string(),
                ]
            })
            .unwrap_or_default(),
        json!({
            "would_install_only_if_mismatched": true,
            "requires_confirmation": apk.is_some() && !args.install,
            "confirmation_flag": "--install",
            "fresh": false,
            "data_preserved": true,
            "destructive": false,
        }),
    );
    state.stage(
        "run_fixture",
        "planned",
        Instant::now(),
        args.fixture
            .as_deref()
            .map(|name| {
                vec![
                    "androidforge".to_string(),
                    "fixture".to_string(),
                    "run".to_string(),
                    name.to_string(),
                    "--json".to_string(),
                ]
            })
            .unwrap_or_default(),
        json!({
            "fixture": args.fixture,
            "state_changing": args.fixture.is_some(),
            "would_run_only_if_requested": true,
        }),
    );
    let flow = args.flow.as_deref();
    state.stage(
        "run_conformance",
        "planned",
        Instant::now(),
        flow.map(|flow| {
            let mut argv = vec![
                "androidforge".to_string(),
                "conformance".to_string(),
                "run".to_string(),
            ];
            if !flow.eq_ignore_ascii_case("conformance") && flow != "default" {
                argv.extend(["--flow".to_string(), flow.to_string()]);
            }
            argv.push("--json".to_string());
            argv
        })
        .unwrap_or_default(),
        json!({
            "flow": args.flow,
            "state_changing": flow.is_some(),
            "would_run_only_if_requested": true,
        }),
    );
    state.stage(
        "explain_failure",
        "planned",
        Instant::now(),
        Vec::new(),
        json!({"would": "explain the first failed stage if the pipeline fails"}),
    );
    state.stage(
        "consolidated_report",
        "planned",
        Instant::now(),
        Vec::new(),
        json!({"would": "write an agent-test result under the AndroidForge state directory"}),
    );

    let mut required_confirmations = Vec::new();
    if apk.is_some() && !args.install {
        required_confirmations.push("--install".to_string());
    }
    if args.fixture.is_some() {
        required_confirmations.push("--fixture".to_string());
    }
    if flow.is_some() {
        required_confirmations.push("--flow".to_string());
    }
    if session.is_none() && (target_avd.is_none() && target_serial.is_none()) {
        required_confirmations.push("--avd or --serial".to_string());
    }
    let state_changing = [
        if apk.is_some() && args.install {
            Some("install_apk")
        } else {
            None
        },
        if args.fixture.is_some() {
            Some("run_fixture")
        } else {
            None
        },
        if flow.is_some() {
            Some("run_conformance")
        } else {
            None
        },
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    state.stage(
        "dry_run_summary",
        "planned",
        Instant::now(),
        Vec::new(),
        json!({
            "required_confirmations": required_confirmations,
            "potentially_destructive_operations": [],
            "state_changing_operations": state_changing,
        }),
    );

    let blocked = session.is_none() && target_avd.is_none() && target_serial.is_none();
    let result = json!({
        "schema_version": 1,
        "pipeline": "agent.test",
        "dry_run": true,
        "generated_at": now_rfc3339(),
        "project": {
            "name": ctx.config.project_name(),
            "root": ctx.paths.project_root,
            "package": ctx.config.package(),
            "debug_package": ctx.config.debug_package(),
        },
        "session": state.session.as_ref().map(|session| json!({
            "id": session.id,
            "owner": session.owner,
            "state": session.state,
            "serial": session.serial,
            "avd": session.avd,
            "lease_expires_at": session.lease_expires_at,
        })),
        "apk": state.apk,
        "stages": state.stages,
        "final_status": if blocked { "blocked" } else { "dry_run" },
        "blocking_issues": if blocked {
            vec!["No session is active and no AVD or serial target was supplied."]
        } else {
            Vec::new()
        },
        "failure": null,
        "required_confirmations": required_confirmations,
        "destructive_operations": [],
        "warnings": state.warnings,
        "next_actions": [
            {
                "reason": "Execute the validated plan with the required confirmation flags.",
                "priority": "normal",
                "command": {"argv": dry_run_execute_argv(ctx, args)},
            }
        ],
    });
    let human = format!(
        "Agent test dry run: {} stage(s), no changes made.\n{}",
        result["stages"].as_array().map(Vec::len).unwrap_or(0),
        if apk.is_some() {
            "Install is only planned; pass --install to permit it.\n"
        } else {
            ""
        }
    );
    let mut response = success("agent.test", result, human);
    response.exit_code = if blocked { 8 } else { 0 };
    response.session_id = state.session.as_ref().map(|session| session.id.clone());
    Ok(response)
}

fn dry_run_execute_argv(ctx: &CliContext, args: &AgentTestArgs) -> Vec<String> {
    let mut argv = vec![
        "androidforge".to_string(),
        "agent".to_string(),
        "test".to_string(),
        "--project".to_string(),
        ctx.paths.project_root.to_string_lossy().into_owned(),
    ];
    if let Some(apk) = &args.apk {
        argv.extend(["--apk".to_string(), apk.to_string_lossy().into_owned()]);
    }
    if let Some(session) = &args.session_id {
        argv.extend(["--session-id".to_string(), session.clone()]);
    }
    if let Some(avd) = &args.avd {
        argv.extend(["--avd".to_string(), avd.clone()]);
    }
    if let Some(serial) = &args.serial {
        argv.extend(["--serial".to_string(), serial.clone()]);
    }
    if let Some(owner) = &args.owner {
        argv.extend(["--owner".to_string(), owner.clone()]);
    }
    if let Some(flow) = &args.flow {
        argv.extend(["--flow".to_string(), flow.clone()]);
    }
    if let Some(fixture) = &args.fixture {
        argv.extend(["--fixture".to_string(), fixture.clone()]);
    }
    if args.install {
        argv.push("--install".to_string());
    }
    argv.push("--json".to_string());
    argv
}
