use std::collections::BTreeSet;
use std::io::{self, Write};

use serde::Serialize;
use serde_json::Value;

use crate::error::{ErrorCode, ForgeError, Result};
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
    /// Output profile used for this response. Absent in full mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// One-line decision summary in compact/brief mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Short next commands in compact/brief mode.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<String>,
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
    /// Envelope status for non-error results. Normally `success`; pipeline-style
    /// commands may report `failed` or `blocked` while preserving structured
    /// results and a meaningful exit code.
    pub status: &'static str,
    pub result: Value,
    pub human: String,
    pub artifacts: Vec<ArtifactRef>,
    pub session_id: Option<String>,
    pub device_serial: Option<String>,
    pub run_id: Option<String>,
    pub duration_ms: u64,
    pub exit_code: i32,
    /// One-line decision summary used by compact/brief output.
    pub summary: Option<String>,
    /// Short next commands used by compact/brief output.
    pub next: Vec<String>,
    /// Extra NDJSON lines emitted *before* the final envelope when `--jsonl`
    /// is active. Never serialized into the envelope and ignored for `--json`.
    pub stream: Vec<Value>,
}

impl CommandSuccess {
    pub fn new(command: impl Into<String>, result: Value, human: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            status: "success",
            result,
            human: human.into(),
            artifacts: Vec::new(),
            session_id: None,
            device_serial: None,
            run_id: None,
            duration_ms: 0,
            exit_code: 0,
            summary: None,
            next: Vec::new(),
            stream: Vec::new(),
        }
    }

    pub fn with_stream(mut self, stream: Vec<Value>) -> Self {
        self.stream = stream;
        self
    }

    pub fn with_status(mut self, status: &'static str) -> Self {
        self.status = status;
        self
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

#[derive(Debug, Clone)]
pub struct OutputOptions {
    pub json: bool,
    pub quiet: bool,
    /// Minified JSON and compact/brief result shaping.
    pub compact: bool,
    /// Profile name included in the envelope (`compact` or `brief`).
    pub profile: Option<String>,
    /// NDJSON mode: implies `json`, minifies every line and writes `stream`
    /// lines before the final envelope. Presentation-only flag.
    pub jsonl: bool,
    /// Write one stats line to stderr after the primary payload.
    pub stats: bool,
    /// Top-level keys of the envelope's `result` object to retain.
    /// Presentation-only flag; never removes envelope-level fields.
    pub fields: Option<Vec<String>>,
}

impl OutputOptions {
    pub fn new(json: bool, quiet: bool) -> Self {
        Self {
            json,
            quiet,
            compact: false,
            profile: None,
            jsonl: false,
            stats: false,
            fields: None,
        }
    }

    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.compact = true;
        self.profile = Some(profile.into());
        self
    }

    pub fn with_jsonl(mut self) -> Self {
        self.jsonl = true;
        self.json = true;
        self
    }

    pub fn with_stats(mut self) -> Self {
        self.stats = true;
        self
    }

    pub fn with_fields(mut self, fields: Vec<String>) -> Self {
        self.fields = Some(fields);
        self
    }
}

/// Keep only the requested top-level keys of a result object.
///
/// The envelope itself is never touched: `schema_version`, `status`,
/// `command`, `exit_code`, `result` and `error` always survive. Only the
/// contents of `result` are trimmed, and the command's original key order is
/// preserved.
fn trim_result_fields(value: &mut Value, fields: &[String]) -> Result<()> {
    let Some(object) = value.as_object_mut() else {
        let kind = match value {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        };
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            format!("--fields requires a JSON object result, but this command returned {kind}"),
        )
        .with_context_string("result_type", kind)
        .with_suggestion(
            "Choose a command whose result is an object, or omit --fields and use --output-profile compact|brief to trim the envelope.",
        ));
    };
    let mut available = object.keys().cloned().collect::<Vec<_>>();
    available.sort();
    let unknown = fields
        .iter()
        .filter(|field| !object.contains_key(field.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        let list = if available.is_empty() {
            "(none)".to_string()
        } else {
            available.join(", ")
        };
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            format!("unknown --fields key(s): {}", unknown.join(", ")),
        )
        .with_context_string("available_result_keys", list.clone())
        .with_suggestion(format!("Available top-level result keys: {list}."))
        .with_suggestion(
            "Envelope trimming is still available through --output-profile compact|brief.",
        ));
    }
    let keep = fields.iter().cloned().collect::<BTreeSet<_>>();
    object.retain(|key, _| keep.contains(key));
    Ok(())
}

/// Streamed iteration objects have no envelope, so trim only when they carry
/// their own nested `result` object.
fn trim_stream_fields(stream: &mut [Value], fields: &[String]) -> Result<()> {
    for item in stream {
        if let Some(result) = item.get_mut("result") {
            trim_result_fields(result, fields)?;
        }
    }
    Ok(())
}

/// Number of newline-terminated stdout lines and exact byte count shared by
/// every payload builder below.
fn emit_stats_if_requested(options: &OutputOptions, payload: &[u8]) {
    if options.stats {
        let lines = payload.iter().filter(|&&byte| byte == b'\n').count();
        write_stats(payload.len(), lines);
    }
}

/// Write the single deterministic stats line to stderr. Public so argument
/// parsing failures can report zero-byte payloads before a command runs.
fn stats_line(stdout_bytes: usize, lines: usize) -> String {
    let estimated_tokens = stdout_bytes.div_ceil(4);
    format!(
        "{{\"stats\":{{\"schema_version\":1,\"stdout_bytes\":{},\"estimated_tokens\":{},\"token_method\":\"bytes_div_4\",\"lines\":{}}}}}",
        stdout_bytes, estimated_tokens, lines
    )
}

pub fn write_stats(stdout_bytes: usize, lines: usize) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "{}", stats_line(stdout_bytes, lines));
    let _ = stderr.flush();
}

pub fn emit_success(options: OutputOptions, success: &CommandSuccess) -> Result<()> {
    let mut result = success.result.clone();
    let mut stream = success.stream.clone();
    if let Some(fields) = &options.fields {
        trim_result_fields(&mut result, fields)?;
        trim_stream_fields(&mut stream, fields)?;
    }
    // Build the complete payload in memory so the stats line can report the
    // exact byte count and line count actually written to stdout.
    let mut payload = Vec::new();
    if options.json {
        let envelope = Envelope {
            schema_version: ENVELOPE_SCHEMA_VERSION,
            status: success.status,
            command: success.command.clone(),
            exit_code: success.exit_code,
            run_id: success.run_id.clone(),
            session_id: success.session_id.clone(),
            device: success.device_serial.as_ref().map(|serial| DeviceRef {
                serial: serial.clone(),
            }),
            profile: options.profile.clone(),
            summary: success.summary.clone(),
            next: success.next.clone(),
            result: Some(result),
            error: None,
            artifacts: success.artifacts.clone(),
            duration_ms: success.duration_ms,
        };
        if options.jsonl {
            for item in &stream {
                serde_json::to_writer(&mut payload, item)?;
                payload.push(b'\n');
            }
            serde_json::to_writer(&mut payload, &envelope)?;
            payload.push(b'\n');
        } else {
            write_json_bytes(&mut payload, &envelope, options.compact)?;
        }
    } else if !options.quiet {
        if !success.human.is_empty() {
            let _ = payload.write_all(success.human.as_bytes());
            if !success.human.ends_with('\n') {
                let _ = payload.write_all(b"\n");
            }
        }
        if !success.artifacts.is_empty() {
            let _ = writeln!(payload, "Artifacts:");
            for artifact in &success.artifacts {
                let _ = writeln!(
                    payload,
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
    }
    write_raw_stdout(&payload)?;
    emit_stats_if_requested(&options, &payload);
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
    let mut payload = Vec::new();
    if options.json {
        let envelope = Envelope {
            schema_version: ENVELOPE_SCHEMA_VERSION,
            status: "error",
            command: command.to_string(),
            exit_code: error.exit_code(),
            run_id,
            session_id,
            device: device_serial.map(|serial| DeviceRef { serial }),
            profile: options.profile.clone(),
            summary: options.compact.then(|| body.message.clone()),
            next: Vec::new(),
            result: None,
            error: Some(body),
            artifacts: error.body().artifacts.clone(),
            duration_ms,
        };
        // NDJSON mode minifies every line, including error envelopes emitted
        // before a command could be parsed or dispatched.
        write_json_bytes(&mut payload, &envelope, options.compact || options.jsonl)?;
        write_raw_stdout(&payload)?;
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
            if key == "report"
                && let Some(report) = value.as_object()
            {
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
            if key == "junit"
                && let Some(junit) = value.as_object()
            {
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
            if key == "external_result"
                && let Some(result) = value.as_object()
            {
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
    emit_stats_if_requested(&options, &payload);
    Ok(())
}

fn write_json_bytes<T: Serialize>(buffer: &mut Vec<u8>, value: &T, compact: bool) -> Result<()> {
    if compact {
        serde_json::to_writer(&mut *buffer, value)?;
    } else {
        serde_json::to_writer_pretty(&mut *buffer, value)?;
    }
    buffer.push(b'\n');
    Ok(())
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
            profile: None,
            summary: None,
            next: Vec::new(),
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
    fn envelope_preserves_pipeline_failed_status() {
        let success = CommandSuccess::new(
            "agent.test",
            serde_json::json!({"final_status": "failed"}),
            "",
        )
        .with_status("failed");
        let envelope = Envelope {
            schema_version: 1,
            status: success.status,
            command: success.command.clone(),
            exit_code: 6,
            run_id: None,
            session_id: None,
            device: None,
            profile: None,
            summary: None,
            next: Vec::new(),
            result: Some(success.result.clone()),
            error: None,
            artifacts: Vec::new(),
            duration_ms: 10,
        };
        let json = serde_json::to_value(envelope).unwrap();
        assert_eq!(json["status"], "failed");
        assert_eq!(json["exit_code"], 6);
        assert_eq!(json["result"]["final_status"], "failed");
    }

    #[test]
    fn fields_trim_only_the_requested_result_keys() {
        let mut result = serde_json::json!({"a": 1, "b": 2, "c": 3});
        trim_result_fields(&mut result, &["c".to_string(), "a".to_string()]).unwrap();
        assert_eq!(result, serde_json::json!({"a": 1, "c": 3}));
    }

    #[test]
    fn fields_reject_unknown_keys_with_the_available_list() {
        let mut result = serde_json::json!({"alpha": 1, "beta": 2});
        let error = trim_result_fields(&mut result, &["gamma".to_string()]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
        let message = error.message();
        assert!(message.contains("gamma"), "{message}");
        let body = error.body();
        let text = format!("{:?}", body.suggestions);
        assert!(text.contains("alpha") && text.contains("beta"), "{text}");
        assert_eq!(result, serde_json::json!({"alpha": 1, "beta": 2}));
    }

    #[test]
    fn fields_reject_non_object_results() {
        let mut result = serde_json::json!([1, 2, 3]);
        let error = trim_result_fields(&mut result, &["a".to_string()]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
        assert!(error.message().contains("array"), "{}", error.message());
        assert!(format!("{:?}", error.body().suggestions).contains("--output-profile"));
    }

    #[test]
    fn stream_trim_only_touches_nested_result_objects() {
        let mut stream = vec![
            serde_json::json!({"event": "iteration", "iteration": 1}),
            serde_json::json!({"event": "iteration", "result": {"a": 1, "b": 2}}),
        ];
        trim_stream_fields(&mut stream, &["b".to_string()]).unwrap();
        assert_eq!(stream[0]["iteration"], 1);
        assert_eq!(stream[1]["result"], serde_json::json!({"b": 2}));
    }

    #[test]
    fn stats_line_is_exact_and_uses_ceil_division() {
        assert_eq!(
            stats_line(0, 0),
            "{\"stats\":{\"schema_version\":1,\"stdout_bytes\":0,\"estimated_tokens\":0,\"token_method\":\"bytes_div_4\",\"lines\":0}}"
        );
        assert_eq!(
            stats_line(9, 3),
            "{\"stats\":{\"schema_version\":1,\"stdout_bytes\":9,\"estimated_tokens\":3,\"token_method\":\"bytes_div_4\",\"lines\":3}}"
        );
        assert_eq!(
            stats_line(10, 4),
            "{\"stats\":{\"schema_version\":1,\"stdout_bytes\":10,\"estimated_tokens\":3,\"token_method\":\"bytes_div_4\",\"lines\":4}}"
        );
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
