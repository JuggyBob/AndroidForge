use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use androidforge_adb::AdbClient;
use androidforge_core::config::{ForgeConfig, HookKind, HookVerification};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::ArtifactRef;
use androidforge_core::run::RunContext;
use androidforge_core::session::Session;
use androidforge_core::util::now_rfc3339;
use androidforge_process::{CommandSpec, Runner};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResult {
    pub name: String,
    pub kind: String,
    pub command: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<HookVerificationResult>,
    pub started_at: String,
    pub ended_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookVerificationResult {
    pub passed: bool,
    pub attempts: u32,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_hook(
    config: &ForgeConfig,
    adb: &AdbClient,
    session: &Session,
    run: &mut RunContext,
    name: &str,
    trace: bool,
    dry_run: bool,
) -> Result<HookResult> {
    let hook = config.hook(name).ok_or_else(|| {
        let available = config
            .file
            .hooks
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        ForgeError::new(
            ErrorCode::HookNotFound,
            format!("hook '{name}' is not defined in the project adapter"),
        )
        .with_context_string("available_hooks", available)
        .with_suggestion("Add a named hook to [hooks.<name>] in .androidforge/project.toml.")
        .with_suggestion(
            "Hooks are explicit debug triggers; AndroidForge never runs them automatically.",
        )
    })?;
    let serial = session.serial()?.to_string();
    let started_at = now_rfc3339();
    let started = Instant::now();
    let step = run.start_step(format!("hook:{name}"), format!("hook {name}"))?;
    let timeout = Duration::from_secs(hook.timeout_secs.unwrap_or(30).max(1));
    if dry_run {
        run.finish_step_skipped(step, "dry run")?;
        return Ok(HookResult {
            name: name.to_string(),
            kind: format!("{:?}", hook.kind),
            command: describe_hook(hook),
            success: true,
            exit_code: None,
            duration_ms: 0,
            output: None,
            artifacts: Vec::new(),
            notes: vec!["dry run; hook was not executed".to_string()],
            verification: None,
            started_at,
            ended_at: now_rfc3339(),
        });
    }

    let mut artifacts = Vec::new();
    let mut notes = Vec::new();
    let verification_before = if let Some(verify) = &hook.verify {
        verification_once(config, adb, session, run, verify, trace)
            .await
            .ok()
    } else {
        None
    };
    let (command_display, output_text, exit_code) = match hook.kind {
        HookKind::Broadcast => {
            let mut args = Vec::new();
            if hook.run_as {
                let package = hook
                    .package
                    .as_deref()
                    .or_else(|| config.debug_package())
                    .or_else(|| config.package())
                    .ok_or_else(|| {
                        ForgeError::new(
                            ErrorCode::HookNotFound,
                            "run_as broadcast hook needs a package",
                        )
                        .with_suggestion("Set package on the hook or in [project].")
                    })?;
                args.extend([
                    "run-as".to_string(),
                    package.to_string(),
                    "am".to_string(),
                    "broadcast".to_string(),
                    "--user".to_string(),
                    "0".to_string(),
                ]);
            } else {
                args.extend(["am".to_string(), "broadcast".to_string()]);
            }
            args.extend(["-a".to_string(), hook.action.clone().unwrap_or_default()]);
            if let Some(component) = &hook.component {
                args.push("-n".to_string());
                args.push(component.clone());
            }
            for (key, value) in &hook.extras {
                args.push("--es".to_string());
                args.push(key.clone());
                args.push(expand_env_refs(value)?);
            }
            let command = if hook.run_as {
                format!(
                    "run-as {} am broadcast -a {}",
                    hook.package
                        .as_deref()
                        .or_else(|| config.debug_package())
                        .or_else(|| config.package())
                        .unwrap_or("<package>"),
                    hook.action.as_deref().unwrap_or("")
                )
            } else {
                format!("am broadcast -a {}", hook.action.as_deref().unwrap_or(""))
            };
            let output = adb
                .shell_args(&serial, &args, Some(timeout), Some(4 * 1024 * 1024))
                .await;
            match output {
                Ok(output) => {
                    let text = output.combined_text();
                    artifacts.push(write_output(run, name, "broadcast", &text)?);
                    (command, Some(text), output.exit_code)
                }
                Err(error) => {
                    run.finish_step_failure(step, &error, artifacts)?;
                    return Err(error);
                }
            }
        }
        HookKind::DeepLink => {
            let mut args = vec![
                "am".to_string(),
                "start".to_string(),
                "-W".to_string(),
                "-a".to_string(),
                "android.intent.action.VIEW".to_string(),
                "-d".to_string(),
                hook.uri.clone().unwrap_or_default(),
            ];
            let package = hook.package.as_deref().or(config.package());
            if let Some(package) = package {
                args.push("-p".to_string());
                args.push(package.to_string());
            }
            let command = format!(
                "am start -W -a android.intent.action.VIEW -d {}",
                hook.uri.as_deref().unwrap_or("")
            );
            let output = adb
                .shell_args(&serial, &args, Some(timeout), Some(4 * 1024 * 1024))
                .await;
            match output {
                Ok(output) => {
                    let text = output.combined_text();
                    artifacts.push(write_output(run, name, "deep-link", &text)?);
                    (command, Some(text), output.exit_code)
                }
                Err(error) => {
                    run.finish_step_failure(step, &error, artifacts)?;
                    return Err(error);
                }
            }
        }
        HookKind::Shell => {
            let command = hook.action.clone().unwrap_or_default();
            let output = adb
                .run_checked(
                    Some(&serial),
                    &["shell".to_string(), command.clone()],
                    Some(timeout),
                    Some(4 * 1024 * 1024),
                )
                .await;
            match output {
                Ok(output) => {
                    let text = output.combined_text();
                    artifacts.push(write_output(run, name, "shell", &text)?);
                    (command, Some(text), output.exit_code)
                }
                Err(error) => {
                    run.finish_step_failure(step, &error, artifacts)?;
                    return Err(error);
                }
            }
        }
        HookKind::Instrumentation => {
            let component = hook.component.clone().unwrap_or_default();
            let mut args = vec![
                "am".to_string(),
                "instrument".to_string(),
                "-w".to_string(),
                "-r".to_string(),
            ];
            for (key, value) in &hook.extras {
                args.push("-e".to_string());
                args.push(key.clone());
                args.push(expand_env_refs(value)?);
            }
            args.push(component.clone());
            let command = format!("am instrument -w -r {}", component);
            let output = adb
                .shell_args(&serial, &args, Some(timeout), Some(8 * 1024 * 1024))
                .await;
            match output {
                Ok(output) => {
                    let text = output.combined_text();
                    artifacts.push(write_output(run, name, "instrumentation", &text)?);
                    (command, Some(text), output.exit_code)
                }
                Err(error) => {
                    run.finish_step_failure(step, &error, artifacts)?;
                    return Err(error);
                }
            }
        }
        HookKind::Command => {
            let command = hook.command.clone();
            if command.is_empty() {
                let error = ForgeError::new(
                    ErrorCode::HookNotFound,
                    "command hook has no command",
                )
                .with_suggestion("Set command = [\"python\", \"scripts/trigger.py\"] in the hook definition.");
                run.finish_step_failure(step, &error, artifacts)?;
                return Err(error);
            }
            let mut env = hook_environment(config, session, run);
            for (key, value) in &hook.extras {
                env.insert(key.clone(), expand_env_refs(value)?);
            }
            let program = command[0].clone();
            let mut spec = CommandSpec::new(program)
                .args(command[1..].iter().cloned())
                .cwd(config.paths.project_root.clone())
                .timeout(timeout)
                .trace(trace)
                .hide_window(true);
            for (key, value) in &env {
                spec = spec.env(key, value);
            }
            let output = Runner::new(trace).run(spec).await;
            match output {
                Ok(output) => {
                    let text = format!("{}\n{}", output.stdout_text(), output.stderr_text());
                    artifacts.push(write_output(run, name, "command", &text)?);
                    if !output.success {
                        let error = ForgeError::new(
                            ErrorCode::CommandFailed,
                            format!("hook command exited with {:?}", output.exit_code),
                        )
                        .with_context_string("output", truncate(text.trim(), 4000))
                        .with_artifacts(artifacts.clone());
                        run.finish_step_failure(step, &error, artifacts)?;
                        return Err(error);
                    }
                    (command.join(" "), Some(text), output.exit_code)
                }
                Err(error) => {
                    let forge_error = ForgeError::from(error);
                    run.finish_step_failure(step, &forge_error, artifacts)?;
                    return Err(forge_error);
                }
            }
        }
    };

    if let Some(output) = &output_text {
        let lower = output.to_ascii_lowercase();
        if lower.contains("error") && exit_code.unwrap_or(0) != 0 {
            let error = ForgeError::new(ErrorCode::CommandFailed, format!("hook '{name}' failed"))
                .with_context_string("output", truncate(output.trim(), 4000))
                .with_artifacts(artifacts.clone());
            run.finish_step_failure(step, &error, artifacts)?;
            return Err(error);
        }
    }
    if !hook.idempotent {
        notes.push(
            "hook is marked non-idempotent; running it again may repeat an app-side action"
                .to_string(),
        );
    }
    let verification = if let Some(verify) = &hook.verify {
        match run_verification(
            config,
            adb,
            session,
            run,
            verify,
            verification_before.clone(),
            trace,
        )
        .await
        {
            Ok((result, artifact)) => {
                artifacts.push(artifact);
                Some(result)
            }
            Err(error) => {
                if let Ok(artifact) = write_output(
                    run,
                    name,
                    "verification-error",
                    &format!(
                        "{}\n{}",
                        error.message(),
                        error
                            .body()
                            .context
                            .get("verification")
                            .map(|value| value.to_string())
                            .unwrap_or_default()
                    ),
                ) {
                    artifacts.push(artifact);
                }
                run.finish_step_failure(step, &error, artifacts)?;
                return Err(error);
            }
        }
    } else {
        None
    };
    let details = serde_json::json!({
        "hook": name,
        "kind": format!("{:?}", hook.kind),
        "exit_code": exit_code,
        "verification": verification,
    });
    run.finish_step_success(step, details, artifacts.clone())?;
    Ok(HookResult {
        name: name.to_string(),
        kind: format!("{:?}", hook.kind),
        command: command_display,
        success: true,
        exit_code,
        duration_ms: elapsed_ms(started),
        output: output_text.map(|text| truncate(&text, 8000)),
        artifacts,
        notes,
        verification,
        started_at,
        ended_at: now_rfc3339(),
    })
}

async fn verification_once(
    config: &ForgeConfig,
    adb: &AdbClient,
    session: &Session,
    run: &RunContext,
    verify: &HookVerification,
    trace: bool,
) -> Result<String> {
    let serial = session.serial()?.to_string();
    let timeout = Duration::from_secs(verify.timeout_secs.unwrap_or(15).max(1));
    if !verify.command.is_empty() {
        let mut env = hook_environment(config, session, run);
        env.extend(verify.env.clone());
        let mut spec = CommandSpec::new(verify.command[0].clone())
            .args(verify.command[1..].iter().cloned())
            .cwd(config.paths.project_root.clone())
            .timeout(timeout)
            .trace(trace)
            .hide_window(true);
        for (key, value) in &env {
            spec = spec.env(key, value);
        }
        let output = Runner::new(trace).run(spec).await?;
        if !output.success {
            return Err(ForgeError::new(
                ErrorCode::HookVerificationFailed,
                "hook verification command exited unsuccessfully",
            )
            .with_context_string("stdout", truncate(output.stdout_text().trim(), 2000))
            .with_context_string("stderr", truncate(output.stderr_text().trim(), 2000)));
        }
        return Ok(output.stdout_text());
    }
    if let Some(shell) = &verify.shell {
        let output = adb
            .run_checked(
                Some(&serial),
                &["shell".to_string(), shell.clone()],
                Some(timeout),
                Some(4 * 1024 * 1024),
            )
            .await?;
        return Ok(output.combined_text());
    }
    Err(ForgeError::new(
        ErrorCode::HookNotFound,
        "hook verification has neither shell nor command",
    )
    .with_suggestion("Set verify.shell or verify.command in the hook."))
}

async fn run_verification(
    config: &ForgeConfig,
    adb: &AdbClient,
    session: &Session,
    run: &mut RunContext,
    verify: &HookVerification,
    before: Option<String>,
    trace: bool,
) -> Result<(HookVerificationResult, ArtifactRef)> {
    let started = Instant::now();
    let timeout = Duration::from_secs(verify.timeout_secs.unwrap_or(15).max(1));
    let mut attempts = 0u32;
    let mut last_output = String::new();
    let mut last_error = None;
    loop {
        attempts += 1;
        match verification_once(config, adb, session, run, verify, trace).await {
            Ok(after) => {
                last_output = after.clone();
                let changed = before.as_deref() != Some(after.as_str());
                let contains = verify
                    .expect_contains
                    .as_deref()
                    .is_none_or(|expected| after.contains(expected));
                let excluded = verify
                    .expect_not_contains
                    .as_deref()
                    .is_none_or(|unexpected| !after.contains(unexpected));
                if (!verify.must_change || changed) && contains && excluded {
                    let artifact = write_output(run, "verification", "hook-verify", &after)?;
                    return Ok((
                        HookVerificationResult {
                            passed: true,
                            attempts,
                            duration_ms: elapsed_ms(started),
                            before,
                            after: Some(after),
                            notes: Vec::new(),
                        },
                        artifact,
                    ));
                }
            }
            Err(error) => last_error = Some(error.message().to_string()),
        }
        if started.elapsed() >= timeout {
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let artifact = write_output(run, "verification", "hook-verify", &last_output)?;
    let detail = serde_json::json!({
        "before": before,
        "after": last_output,
        "must_change": verify.must_change,
        "expect_contains": verify.expect_contains,
        "expect_not_contains": verify.expect_not_contains,
        "attempts": attempts,
        "last_error": last_error,
    });
    Err(ForgeError::new(
        ErrorCode::HookVerificationFailed,
        "hook ran but its declared app-state postcondition was not met",
    )
    .with_context("verification", detail)
    .with_artifacts(vec![artifact])
    .with_suggestion(
        "Fix the hook trigger or the verification command; `adb` exit 0 alone does not prove app behavior.",
    )
    .session(session.id.clone()))
}

fn write_output(run: &mut RunContext, name: &str, kind: &str, text: &str) -> Result<ArtifactRef> {
    let safe = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    run.write_text_artifact(
        &format!("logs/hook-{safe}-{kind}.txt"),
        "hook_output",
        Some("Hook output"),
        text,
    )
}

fn describe_hook(hook: &androidforge_core::config::HookConfig) -> String {
    match hook.kind {
        HookKind::Broadcast => format!("am broadcast -a {}", hook.action.as_deref().unwrap_or("")),
        HookKind::DeepLink => format!("am start -d {}", hook.uri.as_deref().unwrap_or("")),
        HookKind::Shell => format!("shell {}", hook.action.as_deref().unwrap_or("")),
        HookKind::Instrumentation => {
            format!("am instrument {}", hook.component.as_deref().unwrap_or(""))
        }
        HookKind::Command => format!("command {}", hook.command.join(" ")),
    }
}

fn hook_environment(
    config: &ForgeConfig,
    session: &Session,
    run: &RunContext,
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
        "ANDROIDFORGE_ARTIFACTS".to_string(),
        run.dir.to_string_lossy().into_owned(),
    );
    env.insert("ANDROIDFORGE_PHASE".to_string(), "hook".to_string());
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
    if let Ok(exe) = std::env::current_exe() {
        env.insert(
            "ANDROIDFORGE_EXE".to_string(),
            exe.to_string_lossy().into_owned(),
        );
    }
    env
}

fn expand_env_refs(value: &str) -> Result<String> {
    let regex = regex::Regex::new(r"\$\{ENV:([A-Za-z_][A-Za-z0-9_]*)\}").expect("env regex");
    let mut output = String::new();
    let mut last = 0;
    for captures in regex.captures_iter(value) {
        let whole = captures.get(0).unwrap();
        output.push_str(&value[last..whole.start()]);
        let name = captures.get(1).unwrap().as_str();
        let value = std::env::var(name).map_err(|_| {
            ForgeError::new(
                ErrorCode::FixtureInvalid,
                format!("hook references missing environment variable {name}"),
            )
        })?;
        output.push_str(&value);
        last = whole.end();
    }
    output.push_str(&value[last..]);
    Ok(output)
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

    #[test]
    fn expands_environment_references() {
        assert_eq!(expand_env_refs("plain").unwrap(), "plain");
    }
}
