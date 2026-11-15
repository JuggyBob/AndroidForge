use std::io::{self, Write};

use serde::Serialize;
use serde_json::Value;

use crate::error::{ForgeError, Result};
use crate::model::ArtifactRef;

pub const ENVELOPE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct DeviceRef {
    pub serial: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Envelope {
    pub schema_version: u32,
    pub status: &'static str,
    pub command: String,
    pub exit_code: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::error::ErrorBody>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone)]
pub struct CommandSuccess {
    pub command: String,
    pub result: Value,
    pub human: String,
    pub artifacts: Vec<ArtifactRef>,
    pub session_id: Option<String>,
    pub device_serial: Option<String>,
    pub run_id: Option<String>,
    pub duration_ms: u64,
    pub exit_code: i32,
}

impl CommandSuccess {
    pub fn new(command: impl Into<String>, result: Value, human: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            result,
            human: human.into(),
            artifacts: Vec::new(),
            session_id: None,
            device_serial: None,
            run_id: None,
            duration_ms: 0,
            exit_code: 0,
        }
    }

    pub fn with_artifacts(mut self, artifacts: Vec<ArtifactRef>) -> Self {
        self.artifacts = artifacts;
        self
    }

    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_optional_session(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }

    pub fn with_optional_device(mut self, serial: Option<String>) -> Self {
        self.device_serial = serial;
        self
    }

    pub fn with_device(mut self, serial: impl Into<String>) -> Self {
        self.device_serial = Some(serial.into());
        self
    }

    pub fn with_run(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    pub fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OutputOptions {
    pub json: bool,
    pub quiet: bool,
}

impl OutputOptions {
    pub fn new(json: bool, quiet: bool) -> Self {
        Self { json, quiet }
    }
}

pub fn emit_success(options: OutputOptions, success: &CommandSuccess) -> Result<()> {
    if options.json {
        let envelope = Envelope {
            schema_version: ENVELOPE_SCHEMA_VERSION,
            status: "success",
            command: success.command.clone(),
            exit_code: success.exit_code,
            run_id: success.run_id.clone(),
            session_id: success.session_id.clone(),
            device: success.device_serial.as_ref().map(|serial| DeviceRef {
                serial: serial.clone(),
            }),
            result: Some(success.result.clone()),
            error: None,
            artifacts: success.artifacts.clone(),
            duration_ms: success.duration_ms,
        };
        write_json(&envelope)?;
    } else if !options.quiet {
        let mut stdout = io::stdout().lock();
        if !success.human.is_empty() {
            stdout
                .write_all(success.human.as_bytes())
                .map_err(|error| {
                    ForgeError::internal(format!("could not write output: {error}"))
                })?;
            if !success.human.ends_with('\n') {
                let _ = stdout.write_all(b"\n");
            }
        }
        if !success.artifacts.is_empty() {
            let _ = writeln!(stdout, "Artifacts:");
            for artifact in &success.artifacts {
                let _ = writeln!(
                    stdout,
                    "  [{}] {}{}",
                    artifact.kind,
                    artifact.path,
                    artifact
                        .description
                        .as_ref()
                        .map(|description| format!(" - {description}"))
                        .unwrap_or_default()
                );
            }
        }
        let _ = stdout.flush();
    }
    Ok(())
}

pub fn emit_error(
    options: OutputOptions,
    command: &str,
    error: &ForgeError,
    duration_ms: u64,
    session_id: Option<String>,
    device_serial: Option<String>,
    run_id: Option<String>,
) -> Result<()> {
    let mut body = error.body().clone();
    if body.command.is_none() {
        body.command = Some(command.to_string());
    }
    if body.session_id.is_none() {
        body.session_id = session_id.clone();
    }
    if body.device_serial.is_none() {
        body.device_serial = device_serial.clone();
    }
    if options.json {
        let envelope = Envelope {
            schema_version: ENVELOPE_SCHEMA_VERSION,
            status: "error",
            command: command.to_string(),
            exit_code: error.exit_code(),
            run_id,
            session_id,
            device: device_serial.map(|serial| DeviceRef { serial }),
            result: None,
            error: Some(body),
            artifacts: error.body().artifacts.clone(),
            duration_ms,
        };
        write_json(&envelope)?;
    } else if !options.quiet {
        let mut stderr = io::stderr().lock();
        let _ = writeln!(
            stderr,
            "error [{}]: {}",
            error.body().code.as_str(),
            error.body().message
        );
        if let Some(context) = &error.body().cause {
            let _ = writeln!(stderr, "  cause: {context}");
        }
        for (key, value) in &error.body().context {
            // Human output only: a failing run can carry the entire report
            // (including artifact lists) in context. The report path is
            // printed separately and JSON mode keeps the full object.
            if key == "report" {
                if let Some(report) = value.as_object() {
                    let status = report
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let steps = report.get("steps").and_then(Value::as_array);
                    let failed = steps
                        .map(|steps| {
                            steps
                                .iter()
                                .filter(|step| {
                                    step.get("status").and_then(Value::as_str) == Some("failed")
                                })
                                .count()
                        })
                        .unwrap_or(0);
                    let total = steps.map(Vec::len).unwrap_or(0);
                    let _ = writeln!(
                        stderr,
                        "  report: {status}, {failed} failed of {total} step(s)"
                    );
                    continue;
                }
            }
            if key == "junit" {
                if let Some(junit) = value.as_object() {
                    let _ = writeln!(
                        stderr,
                        "  junit: {} test(s), {} failure(s), {} error(s), {} skipped",
                        junit.get("tests").and_then(Value::as_u64).unwrap_or(0),
                        junit.get("failures").and_then(Value::as_u64).unwrap_or(0),
                        junit.get("errors").and_then(Value::as_u64).unwrap_or(0),
                        junit.get("skipped").and_then(Value::as_u64).unwrap_or(0),
                    );
                    continue;
                }
            }
            if key == "external_result" {
                if let Some(result) = value.as_object() {
                    let _ = writeln!(
                        stderr,
                        "  external_result: exit={}, timed_out={}, duration_ms={}",
                        result
                            .get("exit_code")
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "unknown".to_string()),
                        result
                            .get("timed_out")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        result
                            .get("duration_ms")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    );
                    continue;
                }
            }
            let _ = writeln!(stderr, "  {key}: {value}");
        }
        if !error.body().suggestions.is_empty() {
            let _ = writeln!(stderr, "Suggestions:");
            for suggestion in &error.body().suggestions {
                let _ = writeln!(stderr, "  - {suggestion}");
            }
        }
        if !error.body().artifacts.is_empty() {
            let _ = writeln!(stderr, "Evidence:");
            for artifact in &error.body().artifacts {
                let _ = writeln!(stderr, "  [{}] {}", artifact.kind, artifact.path);
            }
        }
        let _ = stderr.flush();
    }
    Ok(())
}

fn write_json<T: Serialize>(value: &T) -> Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, value)?;
    stdout
        .write_all(b"\n")
        .and_then(|_| stdout.flush())
        .map_err(|error| ForgeError::internal(format!("could not write JSON output: {error}")))
}

pub fn write_raw_stdout(bytes: &[u8]) -> Result<()> {
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(bytes)
        .and_then(|_| stdout.flush())
        .map_err(|error| ForgeError::internal(format!("could not write stdout: {error}")))
}

pub fn print_progress(options: OutputOptions, message: impl AsRef<str>) {
    if !options.quiet {
        eprintln!("{}", message.as_ref());
    }
}

pub fn format_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    if headers.is_empty() {
        return String::new();
    }
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if index < widths.len() {
                widths[index] = widths[index].max(cell.chars().count());
            }
        }
    }
    let mut out = String::new();
    for (index, header) in headers.iter().enumerate() {
        if index > 0 {
            out.push_str("  ");
        }
        out.push_str(&format!("{header:<width$}", width = widths[index]));
    }
    out.push('\n');
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            out.push_str("  ");
        }
        out.push_str(&"-".repeat(*width));
    }
    out.push('\n');
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if index > 0 {
                out.push_str("  ");
            }
            if index < widths.len() {
                out.push_str(&format!("{cell:<width$}", width = widths[index]));
            } else {
                out.push_str(cell);
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_serializes_success_without_error() {
        let envelope = Envelope {
            schema_version: 1,
            status: "success",
            command: "devices.list".to_string(),
            exit_code: 0,
            run_id: None,
            session_id: None,
            device: None,
            result: Some(serde_json::json!({"count": 0})),
            error: None,
            artifacts: Vec::new(),
            duration_ms: 12,
        };
        let json = serde_json::to_value(envelope).unwrap();
        assert_eq!(json["status"], "success");
        assert!(json.get("error").is_none());
    }

    #[test]
    fn table_is_aligned() {
        let table = format_table(
            &["serial", "state"],
            &[vec!["emulator-5554".to_string(), "device".to_string()]],
        );
        assert!(table.contains("emulator-5554"));
        assert!(table.lines().count() >= 3);
    }
}
