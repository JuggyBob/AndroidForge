use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use androidforge_adb::AdbClient;
use androidforge_core::config::ForgeConfig;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::ArtifactRef;
use androidforge_core::run::RunContext;
use androidforge_core::session::Session;
use androidforge_core::toolchain::which;
use androidforge_process::{CommandSpec, Runner};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaestroResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub command: String,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
}

pub async fn run_maestro(
    config: &ForgeConfig,
    _adb: &AdbClient,
    session: &Session,
    run: &mut RunContext,
    flow_path: &Path,
    timeout: Duration,
    trace: bool,
) -> Result<MaestroResult> {
    if !config.file.maestro.enabled {
        return Err(ForgeError::new(
            ErrorCode::UnsupportedOperation,
            "Maestro integration is disabled in configuration",
        )
        .with_suggestion("Set [maestro].enabled = true if you want to use Maestro flows."));
    }
    if !flow_path.is_file() {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            format!("Maestro flow {} was not found", flow_path.display()),
        )
        .with_suggestion("Pass a .yaml Maestro flow file.")
        .session(session.id.clone()));
    }
    let maestro = config
        .file
        .sdk
        .maestro
        .clone()
        .or_else(|| which(&config.file.maestro.binary))
        .ok_or_else(|| {
            ForgeError::new(ErrorCode::MaestroNotFound, "Maestro was not found")
                .with_suggestion(
                    "Maestro is optional; native `androidforge ui` commands do not require it.",
                )
                .with_suggestion("Install Maestro or set [sdk].maestro in configuration.")
                .with_suggestion("Run `androidforge doctor` to inspect tool discovery.")
        })?;
    let serial = session.serial()?.to_string();
    let started = Instant::now();
    let step = run.start_step(
        format!(
            "maestro:{}",
            flow_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("flow")
        ),
        format!("maestro test {}", flow_path.display()),
    )?;
    let version = maestro_version(&maestro, timeout.min(Duration::from_secs(20)), trace)
        .await
        .unwrap_or(None);
    let maestro_dir = run.dir.join("maestro");
    std::fs::create_dir_all(&maestro_dir).ok();
    let output_dir_supported = maestro_help_supports_output_dir(&maestro, trace).await;
    let mut args = vec![
        "--device".to_string(),
        serial.clone(),
        "test".to_string(),
        flow_path.to_string_lossy().into_owned(),
    ];
    if output_dir_supported {
        args.push("--test-output-dir".to_string());
        args.push(maestro_dir.to_string_lossy().into_owned());
    } else {
        args.push("--format".to_string());
        args.push("junit".to_string());
        args.push("--output".to_string());
        args.push(
            maestro_dir
                .join("results.xml")
                .to_string_lossy()
                .into_owned(),
        );
    }

    let (program, mut full_args) = if is_batch_script(&maestro) {
        (
            "cmd.exe".to_string(),
            vec!["/c".to_string(), maestro.to_string_lossy().into_owned()],
        )
    } else {
        (maestro.to_string_lossy().into_owned(), Vec::new())
    };
    full_args.extend(args.clone());
    let command_display = format!("maestro {}", args.join(" "));
    let mut spec = CommandSpec::new(program)
        .args(full_args)
        .cwd(config.paths.project_root.clone())
        .env("MAESTRO_CLI_NO_ANALYTICS", "1")
        .env("MAESTRO_CLI_ANALYSIS_NOTIFICATION_DISABLED", "true")
        .timeout(timeout)
        .trace(trace)
        .hide_window(true)
        .max_stdout(16 * 1024 * 1024)
        .max_stderr(8 * 1024 * 1024);
    spec = spec.env(
        "MAESTRO_CLI_TEST_OUTPUT_DIR",
        maestro_dir.to_string_lossy().into_owned(),
    );
    let output = Runner::new(trace).run(spec).await;
    let (output, mut artifacts) = match output {
        Ok(output) => {
            let stdout = output.stdout_text();
            let stderr = output.stderr_text();
            let mut artifacts = Vec::new();
            artifacts.push(run.write_text_artifact(
                "maestro/stdout.txt",
                "maestro_output",
                Some("Maestro stdout"),
                &stdout,
            )?);
            if !stderr.trim().is_empty() {
                artifacts.push(run.write_text_artifact(
                    "maestro/stderr.txt",
                    "maestro_output",
                    Some("Maestro stderr"),
                    &stderr,
                )?);
            }
            (output, artifacts)
        }
        Err(error) => {
            let forge_error = ForgeError::from(error);
            run.finish_step_failure(step, &forge_error, Vec::new())?;
            return Err(forge_error);
        }
    };
    artifacts.extend(register_maestro_files(&maestro_dir, run)?);
    let text = output.combined_text();
    let success = output.success;
    let exit_code = output.exit_code;
    let result = MaestroResult {
        success,
        exit_code,
        version: version.clone(),
        command: command_display.clone(),
        duration_ms: started.elapsed().as_millis() as u64,
        artifacts: artifacts.clone(),
        warnings: if success {
            Vec::new()
        } else {
            vec![
                "Maestro flow failed; original output and any generated reports are preserved."
                    .to_string(),
            ]
        },
        output_preview: Some(text.chars().take(6000).collect()),
    };
    if success {
        run.finish_step_success(
            step,
            serde_json::json!({
                "maestro_version": version,
                "exit_code": exit_code,
                "command": command_display,
            }),
            artifacts,
        )?;
        Ok(result)
    } else {
        let error = ForgeError::new(
            ErrorCode::CommandFailed,
            format!("Maestro flow failed with exit code {:?}", exit_code),
        )
        .with_context_string("output", text.chars().take(6000).collect::<String>())
        .with_artifacts(artifacts)
        .with_suggestion("Inspect the preserved Maestro output and screenshots.")
        .with_suggestion("Maestro Cloud, accounts and API keys are never used.");
        run.finish_step_failure(step, &error, result.artifacts.clone())?;
        Err(error)
    }
}

async fn maestro_version(maestro: &Path, timeout: Duration, trace: bool) -> Result<Option<String>> {
    let (program, mut args) = if is_batch_script(maestro) {
        (
            "cmd.exe".to_string(),
            vec!["/c".to_string(), maestro.to_string_lossy().into_owned()],
        )
    } else {
        (maestro.to_string_lossy().into_owned(), Vec::new())
    };
    args.push("--version".to_string());
    let spec = CommandSpec::new(program)
        .args(args)
        .timeout(timeout)
        .trace(trace)
        .hide_window(true);
    let output = Runner::new(trace).run(spec).await?;
    Ok(output
        .stdout_text()
        .lines()
        .chain(output.stderr_text().lines())
        .find(|line| line.to_ascii_lowercase().contains("maestro"))
        .map(str::trim)
        .map(str::to_string))
}

async fn maestro_help_supports_output_dir(maestro: &Path, trace: bool) -> bool {
    let (program, mut args) = if is_batch_script(maestro) {
        (
            "cmd.exe".to_string(),
            vec!["/c".to_string(), maestro.to_string_lossy().into_owned()],
        )
    } else {
        (maestro.to_string_lossy().into_owned(), Vec::new())
    };
    args.extend(["test".to_string(), "--help".to_string()]);
    let spec = CommandSpec::new(program)
        .args(args)
        .timeout(Duration::from_secs(15))
        .trace(trace)
        .hide_window(true);
    Runner::new(trace)
        .run(spec)
        .await
        .map(|output| output.combined_text().contains("--test-output-dir"))
        .unwrap_or(false)
}

fn register_maestro_files(directory: &Path, run: &mut RunContext) -> Result<Vec<ArtifactRef>> {
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_files(directory, &mut files);
    files.sort();
    files.truncate(200);
    let mut artifacts = Vec::new();
    for file in files {
        if let Ok(artifact) =
            run.register_artifact(&file, "maestro_artifact", Some("Maestro output"))
        {
            artifacts.push(artifact);
        }
    }
    Ok(artifacts)
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files);
        } else if path.is_file() {
            files.push(path);
        }
    }
}

fn is_batch_script(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd")
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_batch_scripts() {
        assert!(is_batch_script(Path::new("maestro.bat")));
        assert!(is_batch_script(Path::new("maestro.cmd")));
        assert!(!is_batch_script(Path::new("maestro")));
    }
}
