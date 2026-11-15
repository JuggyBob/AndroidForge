use std::time::{Duration, Instant};

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::ArtifactRef;
use androidforge_core::run::RunContext;
use androidforge_core::session::Session;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentationResult {
    pub success: bool,
    pub package: String,
    pub runner: String,
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_instrumentation(
    adb: &AdbClient,
    session: &Session,
    run: &mut RunContext,
    package: &str,
    runner: &str,
    extras: &[(String, String)],
    timeout: Duration,
    trace: bool,
) -> Result<InstrumentationResult> {
    if package.trim().is_empty() || runner.trim().is_empty() {
        return Err(ForgeError::invalid_argument(
            "instrumentation requires a test package and runner",
        )
        .with_suggestion("Example: androidforge test instrumentation com.example.test/androidx.test.runner.AndroidJUnitRunner"));
    }
    let serial = session.serial()?.to_string();
    let component = if package.contains('/') {
        package.to_string()
    } else {
        format!("{package}/{runner}")
    };
    let step = run.start_step(
        format!("instrumentation:{component}"),
        format!("am instrument -w -r {component}"),
    )?;
    let started = Instant::now();
    let mut args = vec![
        "am".to_string(),
        "instrument".to_string(),
        "-w".to_string(),
        "-r".to_string(),
    ];
    for (key, value) in extras {
        args.push("-e".to_string());
        args.push(key.clone());
        args.push(value.clone());
    }
    args.push(component.clone());
    let output = adb
        .shell_args(&serial, &args, Some(timeout), Some(16 * 1024 * 1024))
        .await;
    let (output, artifacts) = match output {
        Ok(output) => {
            let text = output.combined_text();
            let artifact = run.write_text_artifact(
                "logs/instrumentation.txt",
                "instrumentation_output",
                Some("Instrumentation stdout/stderr"),
                &text,
            )?;
            (output, vec![artifact])
        }
        Err(error) => {
            run.finish_step_failure(step, &error, Vec::new())?;
            return Err(error);
        }
    };
    let text = output.combined_text();
    let lower = text.to_ascii_lowercase();
    let failed = lower.contains("failures!!!")
        || lower.contains("process crashed")
        || lower.contains("instrumentation failed")
        || lower.contains("error:");
    let success = output.success && !failed;
    let result = InstrumentationResult {
        success,
        package: package.to_string(),
        runner: runner.to_string(),
        exit_code: output.exit_code,
        artifacts: artifacts.clone(),
        output_preview: Some(text.chars().take(12000).collect()),
        duration_ms: started.elapsed().as_millis() as u64,
        warnings: if success {
            Vec::new()
        } else {
            vec![
                "Instrumentation output indicates test failure; the complete output is preserved."
                    .to_string(),
            ]
        },
    };
    let _ = trace;
    if success {
        run.finish_step_success(
            step,
            serde_json::json!({
                "package": package,
                "runner": runner,
                "exit_code": output.exit_code,
            }),
            artifacts,
        )?;
        Ok(result)
    } else {
        let error = ForgeError::new(
            ErrorCode::CommandFailed,
            "instrumentation tests reported failure",
        )
        .with_context_string("output", text.chars().take(6000).collect::<String>())
        .with_artifacts(artifacts)
        .with_suggestion("Import the JUnit XML into the same run with `androidforge report import junit <file>`.")
        .session(session.id.clone());
        run.finish_step_failure(step, &error, result.artifacts.clone())?;
        Err(error)
    }
}
