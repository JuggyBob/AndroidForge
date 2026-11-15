use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use androidforge_adb::AdbClient;
use androidforge_app::AppManager;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::ArtifactRef;
use androidforge_core::run::RunContext;
use androidforge_core::session::Session;
use androidforge_core::util::{now_rfc3339, short_id};
use androidforge_diagnostics::{capture_failure_bundle, capture_screenshot, collect_logcat_tail};
use androidforge_process::{CommandSpec, Runner};
use androidforge_ui::UiManager;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct ExternalRunOptions {
    pub command: Vec<String>,
    pub name: String,
    pub timeout: Duration,
    pub capture_logcat: bool,
    pub capture_screen: bool,
    pub trace: bool,
    pub package: Option<String>,
}

impl ExternalRunOptions {
    pub fn new(command: Vec<String>, name: impl Into<String>) -> Self {
        Self {
            command,
            name: name.into(),
            timeout: Duration::from_secs(600),
            capture_logcat: false,
            capture_screen: false,
            trace: false,
            package: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalRunResult {
    pub name: String,
    pub command: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub timed_out: bool,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_artifact: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_artifact: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub started_at: String,
    pub ended_at: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_external(
    adb: &AdbClient,
    app: &AppManager,
    ui: &UiManager,
    session: &Session,
    run: &mut RunContext,
    project_root: &std::path::Path,
    options: &ExternalRunOptions,
) -> Result<ExternalRunResult> {
    if options.command.is_empty() {
        return Err(
            ForgeError::invalid_argument("run requires an external program after --")
                .with_suggestion("Example: androidforge run --session <ID> -- python my_script.py"),
        );
    }
    let serial = session.serial()?.to_string();
    let started_at = now_rfc3339();
    let started = Instant::now();
    let step = run.start_step(
        format!("external:{}", options.name),
        options.command.join(" "),
    )?;
    let mut artifacts = Vec::new();
    let mut warnings = Vec::new();

    if options.capture_screen {
        match capture_screenshot(
            adb,
            &serial,
            run,
            Some("run-start"),
            Duration::from_secs(30),
        )
        .await
        {
            Ok(screenshot) => artifacts.push(screenshot.artifact),
            Err(error) => warnings.push(format!("start screenshot failed: {}", error.message())),
        }
    }

    let program = options.command[0].clone();
    let args = options.command[1..].to_vec();
    let mut spec = CommandSpec::new(program)
        .args(args)
        .cwd(project_root.to_path_buf())
        .timeout(options.timeout)
        .trace(options.trace)
        .hide_window(false)
        .max_stdout(16 * 1024 * 1024)
        .max_stderr(16 * 1024 * 1024);
    for (key, value) in
        external_environment(adb, session, run, project_root, options.package.as_deref())
    {
        spec = spec.env(key, value);
    }
    let output = match Runner::new(options.trace).run(spec).await {
        Ok(output) => output,
        Err(error) => {
            let forge_error = ForgeError::from(error);
            let _ = run.finish_step_failure(step, &forge_error, artifacts.clone());
            return Err(forge_error);
        }
    };
    let duration_ms = elapsed_ms(started);
    let stdout_text = output.stdout_text();
    let stderr_text = output.stderr_text();
    let command_display = options.command.join(" ");

    let stdout_artifact = run
        .write_text_artifact(
            &format!("logs/run-{}-stdout.txt", short_id()),
            "command_output",
            Some("External command stdout"),
            &stdout_text,
        )
        .ok()
        .map(|artifact| {
            artifacts.push(artifact.clone());
            artifact.path
        });
    let stderr_artifact = run
        .write_text_artifact(
            &format!("logs/run-{}-stderr.txt", short_id()),
            "command_output",
            Some("External command stderr"),
            &stderr_text,
        )
        .ok()
        .map(|artifact| {
            artifacts.push(artifact.clone());
            artifact.path
        });

    if options.capture_logcat {
        match collect_logcat_tail(
            adb,
            &serial,
            options.package.as_deref(),
            Some(2000),
            Some(4 * 1024 * 1024),
        )
        .await
        {
            Ok(logcat) => {
                match run.write_text_artifact(
                    "logs/run-logcat.txt",
                    "logcat_raw",
                    Some("Logcat tail around the external command"),
                    &logcat.raw,
                ) {
                    Ok(artifact) => artifacts.push(artifact),
                    Err(error) => {
                        warnings.push(format!("could not write run logcat: {}", error.message()))
                    }
                }
                if let Ok(artifact) = run.write_text_artifact(
                    "logs/run-logcat-filtered.txt",
                    "logcat_filtered",
                    Some("Package/process-filtered logcat around the external command"),
                    &logcat.filtered,
                ) {
                    artifacts.push(artifact);
                }
            }
            Err(error) => warnings.push(format!("logcat collection failed: {}", error.message())),
        }
    }

    if options.capture_screen {
        match capture_screenshot(adb, &serial, run, Some("run-end"), Duration::from_secs(30)).await
        {
            Ok(screenshot) => artifacts.push(screenshot.artifact),
            Err(error) => warnings.push(format!("end screenshot failed: {}", error.message())),
        }
    }

    let success = output.success && !output.timed_out;
    if !success {
        let code = if output.timed_out {
            ErrorCode::CommandTimeout
        } else {
            ErrorCode::CommandFailed
        };
        let mut error = ForgeError::new(
            code,
            format!(
                "external command '{}' {}",
                command_display,
                if output.timed_out {
                    "timed out".to_string()
                } else {
                    format!(
                        "exited with code {}",
                        output
                            .exit_code
                            .map(|code| code.to_string())
                            .unwrap_or_else(|| "unknown".to_string())
                    )
                }
            ),
        )
        .with_context_string("command", command_display.clone())
        .with_context(
            "exit_code",
            serde_json::to_value(output.exit_code).unwrap_or(serde_json::Value::Null),
        )
        .with_context_string("stdout", truncate(&stdout_text, 4000))
        .with_context_string("stderr", truncate(&stderr_text, 4000))
        .with_artifacts(artifacts.clone());
        if options.capture_screen || options.capture_logcat {
            let bundle = capture_failure_bundle(
                adb,
                Some(ui),
                Some(app),
                run,
                &serial,
                options.package.as_deref(),
                &format!("external command '{}' failed", command_display),
                true,
            )
            .await;
            artifacts.extend(bundle.artifacts.clone());
            warnings.extend(bundle.warnings);
            error = error.with_artifacts(bundle.artifacts);
        }
        run.finish_step_failure(step, &error, artifacts.clone())?;
        return Ok(ExternalRunResult {
            name: options.name.clone(),
            command: command_display,
            exit_code: output.exit_code,
            success: false,
            timed_out: output.timed_out,
            duration_ms,
            stdout_artifact,
            stderr_artifact,
            artifacts,
            warnings,
            started_at,
            ended_at: now_rfc3339(),
        });
    }

    let details = serde_json::json!({
        "exit_code": output.exit_code,
        "duration_ms": duration_ms,
        "stdout_bytes": output.stdout.len(),
        "stderr_bytes": output.stderr.len(),
    });
    run.finish_step_success(step, details, artifacts.clone())?;
    Ok(ExternalRunResult {
        name: options.name.clone(),
        command: command_display,
        exit_code: output.exit_code,
        success: true,
        timed_out: false,
        duration_ms,
        stdout_artifact,
        stderr_artifact,
        artifacts,
        warnings,
        started_at,
        ended_at: now_rfc3339(),
    })
}

fn external_environment(
    adb: &AdbClient,
    session: &Session,
    run: &RunContext,
    project_root: &std::path::Path,
    package: Option<&str>,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("ANDROIDFORGE_SESSION".to_string(), session.id.clone());
    env.insert(
        "ANDROIDFORGE_SERIAL".to_string(),
        session.serial.clone().unwrap_or_default(),
    );
    env.insert(
        "ANDROIDFORGE_PACKAGE".to_string(),
        package.unwrap_or_default().to_string(),
    );
    env.insert(
        "ANDROIDFORGE_ARTIFACTS".to_string(),
        run.dir.to_string_lossy().into_owned(),
    );
    env.insert("ANDROIDFORGE_PHASE".to_string(), "run".to_string());
    env.insert(
        "ANDROIDFORGE_ADB".to_string(),
        adb.adb.to_string_lossy().into_owned(),
    );
    env.insert(
        "ANDROIDFORGE_ROOT".to_string(),
        project_root.to_string_lossy().into_owned(),
    );
    env.insert("ANDROIDFORGE_RUN_ID".to_string(), run.run_id.clone());
    if let Ok(exe) = std::env::current_exe() {
        env.insert(
            "ANDROIDFORGE_EXE".to_string(),
            exe.to_string_lossy().into_owned(),
        );
    }
    env
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
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
    use std::path::PathBuf;

    #[test]
    fn external_environment_has_required_contract() {
        let adb = AdbClient::new(PathBuf::from("adb"), false, Duration::from_secs(30));
        let session = Session::new(
            "owner",
            androidforge_core::session::SessionKind::Attached,
            Some("emulator-5554".to_string()),
            None,
            None,
            true,
        );
        let run = RunContext::create(
            androidforge_core::run::RunOptions::new(
                std::env::temp_dir().join("androidforge-test-artifacts"),
                std::env::temp_dir(),
                "Example",
                "run",
            ),
            androidforge_core::redact::Redactor::disabled(),
        )
        .unwrap();
        let env = external_environment(&adb, &session, &run, std::env::temp_dir().as_path(), None);
        assert!(env.contains_key("ANDROIDFORGE_SESSION"));
        assert!(env.contains_key("ANDROIDFORGE_ADB"));
        assert!(env.contains_key("ANDROIDFORGE_RUN_ID"));
    }
}
