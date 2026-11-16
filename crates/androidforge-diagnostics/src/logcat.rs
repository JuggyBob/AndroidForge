use std::path::PathBuf;
use std::time::{Duration, Instant};

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::paths::ForgePaths;
use androidforge_core::run::RunContext;
use androidforge_core::util::{
    ensure_dir, now_rfc3339, read_json, short_id, tail_lines, write_atomic, write_json_atomic,
};
use androidforge_process::{DetachedSpec, kill_tree, process_exists, spawn_detached};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

const DEFAULT_TAIL_LINES: usize = 2000;
const DEFAULT_MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogcatResult {
    #[serde(skip)]
    pub raw: String,
    #[serde(skip)]
    pub filtered: String,
    #[serde(skip)]
    pub crash_buffer: String,
    pub raw_bytes: u64,
    pub filtered_bytes: u64,
    pub crash_bytes: u64,
    pub lines_requested: usize,
    pub pid_filter: Vec<u32>,
    pub truncated: bool,
    pub captured_at: String,
    pub preview: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogcatPeek {
    pub lines: Vec<String>,
    pub line_count: usize,
    pub raw_bytes: u64,
    pub pid_filter: Vec<u32>,
    pub truncated: bool,
    pub captured_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogcatCapture {
    pub schema_version: u32,
    pub session_id: String,
    pub serial: String,
    pub package: Option<String>,
    pub run_id: String,
    pub run_dir: String,
    pub raw_relative_path: String,
    pub stderr_relative_path: String,
    pub local_pid: u32,
    pub started_at: String,
    pub max_seconds: u64,
    pub max_bytes: u64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogcatStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<LogcatCapture>,
    pub process_alive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_size_bytes: Option<u64>,
    pub age_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogcatStopResult {
    pub raw_artifact: String,
    pub filtered_artifact: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crash_artifact: Option<String>,
    pub raw_bytes: u64,
    pub filtered_bytes: u64,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashEvent {
    pub kind: String,
    pub package: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashReport {
    pub package: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<CrashEvent>,
    pub best_effort: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    pub checked_at: String,
}

pub async fn collect_logcat_tail(
    adb: &AdbClient,
    serial: &str,
    package: Option<&str>,
    lines: Option<usize>,
    max_bytes: Option<usize>,
) -> Result<LogcatResult> {
    let lines = lines.unwrap_or(DEFAULT_TAIL_LINES).max(1);
    let max_bytes = max_bytes.unwrap_or(DEFAULT_MAX_BYTES);
    let output = adb
        .shell_args(
            serial,
            &[
                "logcat".to_string(),
                "-d".to_string(),
                "-v".to_string(),
                "threadtime".to_string(),
                "-t".to_string(),
                lines.to_string(),
            ],
            Some(Duration::from_secs(45)),
            Some(max_bytes + 1),
        )
        .await?;
    let raw = output.stdout_text();
    let mut notes = Vec::new();
    if output.stdout_truncated || raw.len() > max_bytes {
        notes.push(format!(
            "logcat output was capped at approximately {max_bytes} bytes"
        ));
    }
    let pids = if let Some(package) = package {
        adb.shell_args(
            serial,
            &["pidof".to_string(), package.to_string()],
            Some(Duration::from_secs(8)),
            None,
        )
        .await
        .ok()
        .map(|output| {
            output
                .stdout_text()
                .split_whitespace()
                .filter_map(|value| value.parse::<u32>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let filtered = if let Some(package) = package {
        filter_logcat(&raw, package, &pids)
    } else {
        raw.clone()
    };
    let crash_buffer = adb
        .shell_args(
            serial,
            &[
                "logcat".to_string(),
                "-b".to_string(),
                "crash".to_string(),
                "-d".to_string(),
                "-v".to_string(),
                "threadtime".to_string(),
                "-t".to_string(),
                "500".to_string(),
            ],
            Some(Duration::from_secs(20)),
            Some(4 * 1024 * 1024),
        )
        .await
        .map(|output| output.stdout_text())
        .unwrap_or_default();
    Ok(LogcatResult {
        raw_bytes: raw.len() as u64,
        filtered_bytes: filtered.len() as u64,
        crash_bytes: crash_buffer.len() as u64,
        lines_requested: lines,
        pid_filter: pids,
        truncated: output.stdout_truncated || raw.len() > max_bytes,
        captured_at: now_rfc3339(),
        preview: tail_lines(&filtered, 40),
        raw,
        filtered,
        crash_buffer,
        notes,
    })
}

pub async fn collect_logcat_peek(
    adb: &AdbClient,
    serial: &str,
    package: Option<&str>,
    lines: Option<usize>,
    max_bytes: Option<usize>,
    min_level: Option<&str>,
    since: Option<&str>,
) -> Result<LogcatPeek> {
    let requested = lines.unwrap_or(1000).clamp(1, 20_000);
    let max_bytes = max_bytes.unwrap_or(DEFAULT_MAX_BYTES);
    let mut args = vec![
        "logcat".to_string(),
        "-d".to_string(),
        "-v".to_string(),
        "threadtime".to_string(),
        "-t".to_string(),
    ];
    match since.filter(|value| !value.trim().is_empty()) {
        Some(since) => args.push(since.trim().to_string()),
        None => args.push(requested.to_string()),
    }
    let output = adb
        .shell_args(
            serial,
            &args,
            Some(Duration::from_secs(20)),
            Some(max_bytes + 1),
        )
        .await?;
    let raw = output.stdout_text();
    let pids = if let Some(package) = package {
        adb.shell_args(
            serial,
            &["pidof".to_string(), package.to_string()],
            Some(Duration::from_secs(5)),
            None,
        )
        .await
        .ok()
        .map(|output| {
            output
                .stdout_text()
                .split_whitespace()
                .filter_map(|value| value.parse::<u32>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let filtered = if let Some(package) = package {
        filter_logcat(&raw, package, &pids)
    } else {
        raw.clone()
    };
    let min_priority = min_level.and_then(parse_priority).unwrap_or(0);
    let mut selected = Vec::new();
    for line in filtered.lines() {
        if min_priority > 0 {
            let Some(priority) = line_priority(line) else {
                continue;
            };
            if priority < min_priority {
                continue;
            }
        }
        selected.push(line.to_string());
    }
    let truncated = output.stdout_truncated || raw.len() > max_bytes || selected.len() > requested;
    if selected.len() > requested {
        selected = selected.split_off(selected.len() - requested);
    }
    let line_count = selected.len();
    let raw_bytes = raw.len() as u64;
    let mut notes = Vec::new();
    if output.stdout_truncated || raw.len() > max_bytes {
        notes.push(format!(
            "logcat output was capped at approximately {max_bytes} bytes"
        ));
    }
    if pids.is_empty() && package.is_some() {
        notes.push(
            "the package process was not running; package filtering may be incomplete".to_string(),
        );
    }
    Ok(LogcatPeek {
        lines: selected,
        line_count,
        raw_bytes,
        pid_filter: pids,
        truncated,
        captured_at: now_rfc3339(),
        notes,
    })
}

fn parse_priority(value: &str) -> Option<u8> {
    match value.trim().to_ascii_lowercase().as_str() {
        "v" | "verbose" => Some(0),
        "d" | "debug" => Some(1),
        "i" | "info" => Some(2),
        "w" | "warn" | "warning" => Some(3),
        "e" | "error" => Some(4),
        "f" | "fatal" | "assert" => Some(5),
        _ => None,
    }
}

fn line_priority(line: &str) -> Option<u8> {
    let token = line.split_whitespace().nth(4)?;
    match token.as_bytes().first().copied()? {
        b'V' => Some(0),
        b'D' => Some(1),
        b'I' => Some(2),
        b'W' => Some(3),
        b'E' => Some(4),
        b'F' | b'A' => Some(5),
        _ => None,
    }
}

pub fn filter_logcat(raw: &str, package: &str, pids: &[u32]) -> String {
    let pid_pattern =
        Regex::new(r"^\S+\s+\S+\s+(\d+)\s+\d+\s+[VDIWEF]/").expect("valid logcat pid regex");
    let mut output = String::new();
    for line in raw.lines() {
        let keep = line.contains(package)
            || line.starts_with("--------- beginning of")
            || pid_pattern
                .captures(line)
                .and_then(|captures| captures.get(1))
                .and_then(|value| value.as_str().parse::<u32>().ok())
                .map(|pid| pids.contains(&pid))
                .unwrap_or(false);
        if keep {
            output.push_str(line);
            output.push('\n');
        }
    }
    output
}

pub async fn detect_crashes(adb: &AdbClient, serial: &str, package: &str) -> Result<CrashReport> {
    let events_output = adb
        .shell_args(
            serial,
            &[
                "logcat".to_string(),
                "-b".to_string(),
                "events".to_string(),
                "-d".to_string(),
                "-v".to_string(),
                "threadtime".to_string(),
                "-t".to_string(),
                "10000".to_string(),
            ],
            Some(Duration::from_secs(20)),
            Some(8 * 1024 * 1024),
        )
        .await
        .map(|output| output.stdout_text())
        .unwrap_or_default();
    let main_output = adb
        .shell_args(
            serial,
            &[
                "logcat".to_string(),
                "-d".to_string(),
                "-v".to_string(),
                "threadtime".to_string(),
                "-t".to_string(),
                "4000".to_string(),
            ],
            Some(Duration::from_secs(30)),
            Some(8 * 1024 * 1024),
        )
        .await
        .map(|output| output.stdout_text())
        .unwrap_or_default();
    Ok(parse_crashes(package, &events_output, &main_output))
}

fn parse_crashes(package: &str, events: &str, main: &str) -> CrashReport {
    let mut crash_events = Vec::new();
    for line in events.lines().filter(|line| line.contains("am_crash:")) {
        if let Some(event) = parse_event_line(line, package, "java_crash", "am_crash:") {
            crash_events.push(event);
        }
    }
    for line in events.lines().filter(|line| line.contains("am_anr:")) {
        if let Some(event) = parse_event_line(line, package, "anr", "am_anr:") {
            crash_events.push(event);
        }
    }
    for line in events.lines().filter(|line| line.contains("am_proc_died:")) {
        if let Some(event) = parse_event_line(line, package, "process_death", "am_proc_died:") {
            crash_events.push(event);
        }
    }

    let mut fatal_block: Vec<CrashEvent> = Vec::new();
    let mut fatal_active = false;
    for line in main.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("fatal exception") {
            if fatal_active {
                flush_fatal_block(&mut fatal_block, package, &mut crash_events);
            }
            fatal_active = true;
            fatal_block.clear();
            fatal_block.push(CrashEvent {
                kind: "java_crash".to_string(),
                package: package.to_string(),
                message: "FATAL EXCEPTION in logcat".to_string(),
                timestamp: log_timestamp(line),
                evidence: line.to_string(),
            });
            continue;
        }
        if fatal_active {
            let trimmed = line.trim();
            let looks_like_stack = trimmed.contains("Process:")
                || trimmed.contains("at ")
                || trimmed.contains("\\tat ")
                || trimmed.contains("Caused by:")
                || trimmed.contains("Suppressed:")
                || trimmed.contains("... ")
                || trimmed.contains("Exception")
                || trimmed.contains("Error")
                || trimmed.starts_with("java.")
                || trimmed.starts_with("kotlin.");
            if trimmed.is_empty() || !looks_like_stack {
                flush_fatal_block(&mut fatal_block, package, &mut crash_events);
                fatal_active = false;
            } else if fatal_block.len() < 80 {
                fatal_block.push(CrashEvent {
                    kind: "java_crash".to_string(),
                    package: package.to_string(),
                    message: trimmed.to_string(),
                    timestamp: log_timestamp(line),
                    evidence: line.to_string(),
                });
            }
        }
        if lower.contains("anr in") && line.contains(package) {
            crash_events.push(CrashEvent {
                kind: "anr".to_string(),
                package: package.to_string(),
                message: line.trim().to_string(),
                timestamp: log_timestamp(line),
                evidence: line.to_string(),
            });
        }
        if (lower.contains("fatal signal") || lower.contains("debug : *** ***"))
            && line.contains(package)
        {
            crash_events.push(CrashEvent {
                kind: "native_crash".to_string(),
                package: package.to_string(),
                message: line.trim().to_string(),
                timestamp: log_timestamp(line),
                evidence: line.to_string(),
            });
        }
    }
    if fatal_active {
        flush_fatal_block(&mut fatal_block, package, &mut crash_events);
    }
    crash_events.truncate(200);
    CrashReport {
        package: package.to_string(),
        events: crash_events,
        best_effort: true,
        notes: vec![
            "Crash and ANR detection uses the current logcat/event buffers; Android may have evicted older evidence."
                .to_string(),
            "A normal process exit and an ANR are not the same condition; inspect app/task state as well."
                .to_string(),
        ],
        checked_at: now_rfc3339(),
    }
}

fn flush_fatal_block(block: &mut Vec<CrashEvent>, package: &str, events: &mut Vec<CrashEvent>) {
    let belongs = block
        .iter()
        .any(|event| event.evidence.contains(package) || event.message.contains(package));
    if belongs {
        events.append(block);
    } else {
        block.clear();
    }
}

fn parse_event_line(line: &str, package: &str, kind: &str, marker: &str) -> Option<CrashEvent> {
    let rest = &line[line.find(marker)? + marker.len()..];
    let start = rest.find('[')?;
    let end = rest[start..].find(']')? + start;
    let fields = rest[start + 1..end]
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    if fields.len() < 3 {
        return None;
    }
    if fields[2] != package {
        return None;
    }
    Some(CrashEvent {
        kind: kind.to_string(),
        package: package.to_string(),
        message: rest[start + 1..end].to_string(),
        timestamp: log_timestamp(line),
        evidence: line.to_string(),
    })
}

fn log_timestamp(line: &str) -> Option<String> {
    let fields = line.split_whitespace().collect::<Vec<_>>();
    if fields.len() >= 2 {
        Some(format!("{} {}", fields[0], fields[1]))
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub struct LogcatStream {
    pub adb: AdbClient,
    pub paths: ForgePaths,
}

impl LogcatStream {
    pub fn new(adb: AdbClient, paths: ForgePaths) -> Self {
        Self { adb, paths }
    }

    pub fn state_path(&self, session_id: &str) -> PathBuf {
        self.paths
            .state_dir
            .join("logcat")
            .join(format!("{}.json", sanitize_key(session_id)))
    }

    pub fn load(&self, session_id: &str) -> Result<Option<LogcatCapture>> {
        let path = self.state_path(session_id);
        if !path.is_file() {
            return Ok(None);
        }
        match read_json(&path) {
            Ok(state) => Ok(Some(state)),
            Err(_) => {
                let _ = std::fs::remove_file(path);
                Ok(None)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        &self,
        session_id: &str,
        serial: &str,
        package: Option<&str>,
        run: &RunContext,
        trace: bool,
        max_seconds: u64,
        max_bytes: usize,
    ) -> Result<LogcatCapture> {
        if self.load(session_id)?.is_some() {
            return Err(ForgeError::new(
                ErrorCode::DeviceBusy,
                format!("logcat capture is already active for session {session_id}"),
            )
            .with_suggestion("Run `androidforge diagnostics logcat stop` first.")
            .session(session_id.to_string())
            .device(serial.to_string()));
        }
        let id = short_id();
        let raw_relative = format!("logs/logcat-{id}.txt");
        let stderr_relative = format!("logs/logcat-{id}.stderr.txt");
        let raw_path = run.dir.join(&raw_relative);
        let stderr_path = run.dir.join(&stderr_relative);
        if let Some(parent) = raw_path.parent() {
            ensure_dir(parent)?;
        }
        let max_seconds = max_seconds.max(1);
        let max_bytes = max_bytes.max(64 * 1024);
        let remote_command =
            format!("timeout {max_seconds} logcat -v threadtime | head -c {max_bytes}");
        let spec = DetachedSpec::new(self.adb.adb.to_string_lossy().into_owned())
            .args([
                "-s".to_string(),
                serial.to_string(),
                "shell".to_string(),
                remote_command,
            ])
            .stdout_path(raw_path)
            .stderr_path(stderr_path.clone())
            .hide_window(true)
            .trace(trace);
        let child = spawn_detached(&spec)?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        if !process_exists(child.pid) {
            let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                "logcat capture process exited immediately",
            )
            .with_context_string("stderr", truncate(stderr.trim(), 3000))
            .with_suggestion("Continuous capture uses Android's toybox timeout command; verify it is available with `androidforge adb shell which timeout`.")
            .device(serial.to_string()));
        }
        let state = LogcatCapture {
            schema_version: 1,
            session_id: session_id.to_string(),
            serial: serial.to_string(),
            package: package.map(str::to_string),
            run_id: run.run_id.clone(),
            run_dir: run.dir.to_string_lossy().into_owned(),
            raw_relative_path: raw_relative,
            stderr_relative_path: stderr_relative,
            local_pid: child.pid,
            started_at: now_rfc3339(),
            max_seconds,
            max_bytes: max_bytes as u64,
            status: "capturing".to_string(),
        };
        if let Some(parent) = self.state_path(session_id).parent() {
            ensure_dir(parent)?;
        }
        write_json_atomic(&self.state_path(session_id), &state)?;
        Ok(state)
    }

    pub async fn status(&self, session_id: &str) -> Result<LogcatStatus> {
        let Some(state) = self.load(session_id)? else {
            return Ok(LogcatStatus {
                capture: None,
                process_alive: false,
                local_size_bytes: None,
                age_ms: 0,
            });
        };
        let process_alive = process_exists(state.local_pid);
        let local_path = PathBuf::from(&state.run_dir).join(&state.raw_relative_path);
        let local_size_bytes = std::fs::metadata(&local_path).ok().map(|meta| meta.len());
        let age_ms = DateTime::parse_from_rfc3339(&state.started_at)
            .map(|started| {
                Utc::now()
                    .signed_duration_since(started.with_timezone(&Utc))
                    .num_milliseconds()
                    .max(0) as u64
            })
            .unwrap_or(0);
        Ok(LogcatStatus {
            capture: Some(state),
            process_alive,
            local_size_bytes,
            age_ms,
        })
    }

    pub async fn stop(&self, session_id: &str, run: &mut RunContext) -> Result<LogcatStopResult> {
        let Some(state) = self.load(session_id)? else {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                format!("no logcat capture is active for session {session_id}"),
            )
            .with_suggestion("Run `androidforge diagnostics logcat start` first.")
            .session(session_id.to_string()));
        };
        if state.run_id != run.run_id {
            return Err(ForgeError::new(
                ErrorCode::SessionOwnershipMismatch,
                format!(
                    "logcat capture belongs to run {}, not the active run {}",
                    state.run_id, run.run_id
                ),
            )
            .session(session_id.to_string()));
        }
        let started = Instant::now();
        kill_tree(state.local_pid);
        while process_exists(state.local_pid) && started.elapsed() < Duration::from_secs(8) {
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        let raw_path = PathBuf::from(&state.run_dir).join(&state.raw_relative_path);
        let raw = std::fs::read(&raw_path).map_err(|error| {
            ForgeError::new(
                ErrorCode::FileNotFound,
                format!(
                    "logcat capture file {} is missing: {error}",
                    raw_path.display()
                ),
            )
        })?;
        let raw_text = String::from_utf8_lossy(&raw);
        let hit_cap = raw.len() as u64 >= state.max_bytes;
        let redacted_raw = run.redactor().redact(&raw_text);
        write_atomic(&raw_path, redacted_raw.as_bytes())?;
        let raw_artifact =
            run.register_artifact(&raw_path, "logcat_raw", Some("Raw logcat capture"))?;
        let filtered = if let Some(package) = &state.package {
            let pids = self
                .adb
                .shell_args(
                    &state.serial,
                    &["pidof".to_string(), package.clone()],
                    Some(Duration::from_secs(8)),
                    None,
                )
                .await
                .ok()
                .map(|output| {
                    output
                        .stdout_text()
                        .split_whitespace()
                        .filter_map(|value| value.parse::<u32>().ok())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            filter_logcat(&redacted_raw, package, &pids)
        } else {
            redacted_raw.clone()
        };
        let filtered_relative = state.raw_relative_path.replace(".txt", "-filtered.txt");
        let filtered_artifact = run.write_text_artifact(
            &filtered_relative,
            "logcat_filtered",
            Some("Package/process-filtered logcat capture"),
            &filtered,
        )?;
        let crash = self
            .adb
            .shell_args(
                &state.serial,
                &[
                    "logcat".to_string(),
                    "-b".to_string(),
                    "crash".to_string(),
                    "-d".to_string(),
                    "-v".to_string(),
                    "threadtime".to_string(),
                    "-t".to_string(),
                    "500".to_string(),
                ],
                Some(Duration::from_secs(20)),
                Some(4 * 1024 * 1024),
            )
            .await
            .ok()
            .map(|output| output.stdout_text())
            .unwrap_or_default();
        let crash_artifact = if crash.trim().is_empty() {
            None
        } else {
            Some(
                run.write_text_artifact(
                    "logs/crash-buffer.txt",
                    "crash_buffer",
                    Some("Android crash logcat buffer at capture stop"),
                    &crash,
                )?
                .path,
            )
        };
        let _ = std::fs::remove_file(self.state_path(session_id));
        Ok(LogcatStopResult {
            raw_artifact: raw_artifact.path,
            filtered_artifact: filtered_artifact.path,
            crash_artifact,
            raw_bytes: raw.len() as u64,
            filtered_bytes: filtered.len() as u64,
            duration_ms: started.elapsed().as_millis() as u64,
            notes: {
                let mut notes = vec![
                    "logcat was captured continuously until stop; the raw file may be large"
                        .to_string(),
                ];
                if hit_cap {
                    notes.push(format!(
                        "the capture reached its {} byte cap; earlier or later lines may be missing",
                        state.max_bytes
                    ));
                }
                notes
            },
        })
    }
}

fn sanitize_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
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
    fn filters_by_package_and_pid() {
        let raw = "11-15 10:00:00.000  1234  1300 I/ActivityManager: hello com.example\n11-15 10:00:01.000  1234  1300 I/Tag: unrelated\n11-15 10:00:02.000  2222  1300 E/Tag: other\n";
        let filtered = filter_logcat(raw, "com.example", &[2222]);
        assert!(filtered.contains("hello com.example"));
        assert!(filtered.contains("E/Tag: other"));
        assert!(!filtered.contains("unrelated"));
    }

    #[test]
    fn parses_event_crash() {
        let events = "11-15 10:00:00.000  100  200 I am_crash: [1234,10123,com.example,com.example,0,java.lang.RuntimeException,boom,Main.kt,42,123]\n";
        let report = parse_crashes("com.example", events, "");
        assert_eq!(report.events.len(), 1);
        assert_eq!(report.events[0].kind, "java_crash");
    }

    #[test]
    fn parses_anr_from_main_log() {
        let main =
            "11-15 10:00:00.000  100  200 E ActivityManager: ANR in com.example (.MainActivity)\n";
        let report = parse_crashes("com.example", "", main);
        assert!(report.events.iter().any(|event| event.kind == "anr"));
    }
}
