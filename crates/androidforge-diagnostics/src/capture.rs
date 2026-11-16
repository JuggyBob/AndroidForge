use std::path::PathBuf;
use std::time::{Duration, Instant};

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::ArtifactRef;
use androidforge_core::paths::ForgePaths;
use androidforge_core::run::RunContext;
use androidforge_core::util::{
    ensure_dir, now_rfc3339, read_json, sha256_hex, short_id, slugify, write_atomic,
    write_json_atomic,
};
use androidforge_process::{DetachedSpec, kill_tree, process_exists, shell_quote, spawn_detached};
use chrono::Utc;
use serde::{Deserialize, Serialize};

const MAX_RECORD_SECONDS: u64 = 180;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotResult {
    pub artifact: ArtifactRef,
    pub local_path: String,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
    pub captured_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_size: Option<(u32, u32)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameResult {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub sha256: String,
    pub captured_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_size: Option<(u32, u32)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

pub async fn capture_screenshot(
    adb: &AdbClient,
    serial: &str,
    run: &mut RunContext,
    label: Option<&str>,
    timeout: Duration,
) -> Result<ScreenshotResult> {
    let output = adb
        .exec_out_args(
            serial,
            &["screencap".to_string(), "-p".to_string()],
            Some(timeout),
            Some(64 * 1024 * 1024),
        )
        .await?;
    if output.stdout_truncated {
        return Err(ForgeError::new(
            ErrorCode::CommandFailed,
            "screenshot exceeded the 64 MiB capture limit",
        )
        .device(serial.to_string()));
    }
    let bytes = output.stdout;
    if bytes.len() < 24 || &bytes[..8] != PNG_SIGNATURE {
        return Err(ForgeError::new(
            ErrorCode::CommandFailed,
            "screencap did not return a valid PNG",
        )
        .with_context_string("bytes", bytes.len().to_string())
        .with_suggestion("Check whether the device display is available and awake.")
        .with_suggestion("Use `androidforge adb --serial <serial> exec-out screencap -p > file` only with binary-safe tooling.")
        .device(serial.to_string()));
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S-%3f").to_string();
    let label = slugify(label.unwrap_or("screenshot"), 40);
    let relative = format!("screenshots/{timestamp}_{label}.png");
    let artifact =
        run.write_binary_artifact(&relative, "screenshot", Some(label.as_str()), &bytes)?;
    let mut notes = Vec::new();
    let display_size = adb.display_size(serial).await;
    if let Some((display_width, display_height)) = display_size
        && (display_width != width || display_height != height)
    {
        notes.push(format!(
                "screenshot dimensions {width}x{height} differ from display size {display_width}x{display_height}; do not scale tap coordinates from this image"
            ));
    }
    let metadata = serde_json::json!({
        "schema_version": 1,
        "captured_at": now_rfc3339(),
        "serial": serial,
        "width": width,
        "height": height,
        "display_size": display_size.map(|(w, h)| format!("{w}x{h}")),
        "sha256": artifact.sha256.clone(),
        "path": relative,
        "notes": notes,
    });
    let metadata_relative = format!("{relative}.json");
    let _ = run.write_text_artifact(
        &metadata_relative,
        "screenshot_metadata",
        Some("Display, timestamp and checksum metadata"),
        &serde_json::to_string_pretty(&metadata).unwrap_or_default(),
    );
    let sha256 = artifact.sha256.clone().unwrap_or_default();
    let local_path = run.dir.join(&artifact.path).to_string_lossy().into_owned();
    Ok(ScreenshotResult {
        artifact,
        local_path,
        width,
        height,
        sha256,
        captured_at: now_rfc3339(),
        display_size,
        notes,
    })
}

/// Capture a single preview frame directly to a local PNG without creating a
/// run, artifact manifest or report. Intended for the desktop inspector.
pub async fn capture_frame(
    adb: &AdbClient,
    serial: &str,
    output_path: &std::path::Path,
    timeout: Duration,
) -> Result<FrameResult> {
    let output = adb
        .exec_out_args(
            serial,
            &["screencap".to_string(), "-p".to_string()],
            Some(timeout),
            Some(64 * 1024 * 1024),
        )
        .await?;
    if output.stdout_truncated {
        return Err(ForgeError::new(
            ErrorCode::CommandFailed,
            "preview frame exceeded the 64 MiB capture limit",
        )
        .device(serial.to_string()));
    }
    let bytes = output.stdout;
    if bytes.len() < 24 || &bytes[..8] != PNG_SIGNATURE {
        return Err(ForgeError::new(
            ErrorCode::CommandFailed,
            "screencap did not return a valid PNG",
        )
        .with_context_string("bytes", bytes.len().to_string())
        .with_suggestion("Check whether the device display is available and awake.")
        .device(serial.to_string()));
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        ensure_dir(parent)?;
    }
    write_atomic(output_path, &bytes)?;
    let mut notes = Vec::new();
    let display_size = adb.display_size(serial).await;
    if let Some((display_width, display_height)) = display_size
        && (display_width != width || display_height != height)
    {
        notes.push(format!(
            "preview dimensions {width}x{height} differ from display size {display_width}x{display_height}"
        ));
    }
    Ok(FrameResult {
        path: output_path.to_string_lossy().into_owned(),
        width,
        height,
        bytes: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
        captured_at: now_rfc3339(),
        display_size,
        notes,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingState {
    pub schema_version: u32,
    pub session_id: String,
    pub serial: String,
    pub run_id: String,
    pub run_dir: String,
    pub remote_path: String,
    pub remote_pid_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_pid: Option<u32>,
    pub local_pid: u32,
    pub local_relative_path: String,
    pub started_at: String,
    pub requested_time_limit_secs: u64,
    pub time_limit_secs: u64,
    pub bit_rate: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    pub status: String,
    /// True when AndroidForge created an implicit run just for this recording.
    /// A recording started inside a caller-owned run must not finalize it.
    #[serde(default)]
    pub finalize_on_stop: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording: Option<RecordingState>,
    pub local_process_alive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_size_bytes: Option<u64>,
    pub age_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingOutcome {
    pub artifact: ArtifactRef,
    pub local_path: String,
    pub size_bytes: u64,
    pub duration_ms: u64,
    pub remote_path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Recorder {
    pub adb: AdbClient,
    pub paths: ForgePaths,
}

impl Recorder {
    pub fn new(adb: AdbClient, paths: ForgePaths) -> Self {
        Self { adb, paths }
    }

    pub fn state_path(&self, session_id: &str) -> PathBuf {
        self.paths
            .state_dir
            .join("recordings")
            .join(format!("{}.json", sanitize_key(session_id)))
    }

    pub fn load_state(&self, session_id: &str) -> Result<Option<RecordingState>> {
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
        run: &RunContext,
        requested_time_limit_secs: u64,
        bit_rate: u64,
        size: Option<String>,
        finalize_on_stop: bool,
        trace: bool,
    ) -> Result<RecordingState> {
        if let Some(existing) = self.load_state(session_id)? {
            let alive = process_exists(existing.local_pid);
            return Err(ForgeError::new(
                ErrorCode::DeviceBusy,
                format!(
                    "a recording is already {} for session {session_id}",
                    if alive { "active" } else { "awaiting stop/finalization" }
                ),
            )
            .with_suggestion("Run `androidforge capture record status` and `androidforge capture record stop` first.")
            .session(session_id.to_string())
            .device(serial.to_string()));
        }
        let requested = requested_time_limit_secs.max(1);
        let time_limit = requested.min(MAX_RECORD_SECONDS);
        let mut notes = Vec::new();
        if requested > MAX_RECORD_SECONDS {
            notes.push(format!(
                "Android's standard screenrecord limit is {MAX_RECORD_SECONDS}s; recording will stop at {time_limit}s. Start another recording for additional footage."
            ));
        }
        if let Some(size) = &size {
            let valid = size
                .split_once('x')
                .map(|(width, height)| {
                    width.parse::<u32>().is_ok() && height.parse::<u32>().is_ok()
                })
                .unwrap_or(false);
            if !valid {
                return Err(ForgeError::invalid_argument(
                    "--size must use WIDTHxHEIGHT, for example 720x1280",
                ));
            }
        }
        let id = short_id();
        let remote_path = format!("/data/local/tmp/androidforge-record-{id}.mp4");
        let remote_pid_path = format!("/data/local/tmp/androidforge-record-{id}.pid");
        let local_relative = format!(
            "recordings/{}_{}.mp4",
            Utc::now().format("%Y%m%d-%H%M%S"),
            sanitize_key(session_id)
        );
        let log_path = run.dir.join("logs").join(format!("recording-{id}.log"));
        let mut screenrecord = format!("--time-limit {} --bit-rate {}", time_limit, bit_rate);
        if let Some(size) = &size {
            screenrecord.push_str(&format!(" --size {size}"));
        }
        let script = format!(
            "echo $$ > {}; exec screenrecord {} {}",
            shell_quote(&remote_pid_path),
            screenrecord,
            shell_quote(&remote_path)
        );
        let spec = DetachedSpec::new(self.adb.adb.to_string_lossy().into_owned())
            .args([
                "-s".to_string(),
                serial.to_string(),
                "shell".to_string(),
                script,
            ])
            .stdout_path(log_path.clone())
            .stderr_path(log_path)
            .hide_window(true)
            .trace(trace)
            .env("ANDROIDFORGE_SESSION", session_id.to_string());
        let child = spawn_detached(&spec)?;
        tokio::time::sleep(Duration::from_millis(600)).await;
        if !process_exists(child.pid) {
            let log =
                std::fs::read_to_string(run.dir.join("logs").join(format!("recording-{id}.log")))
                    .unwrap_or_default();
            return Err(ForgeError::new(
                ErrorCode::UnsupportedOperation,
                "screenrecord exited immediately",
            )
            .with_context_string("output", truncate(log.trim(), 4000))
            .with_suggestion("This device may not support screen recording, or the requested size/bitrate is invalid.")
            .with_suggestion("Verify with `androidforge adb shell screenrecord --help`.")
            .device(serial.to_string()));
        }
        let remote_pid = self
            .adb
            .shell_args(
                serial,
                &["cat".to_string(), remote_pid_path.clone()],
                Some(Duration::from_secs(5)),
                None,
            )
            .await
            .ok()
            .and_then(|output| output.stdout_text().trim().parse::<u32>().ok());
        if remote_pid.is_none() {
            notes.push(
                "could not identify the remote screenrecord PID; stop may be limited by the device time limit"
                    .to_string(),
            );
        }
        let state = RecordingState {
            schema_version: 1,
            session_id: session_id.to_string(),
            serial: serial.to_string(),
            run_id: run.run_id.clone(),
            run_dir: run.dir.to_string_lossy().into_owned(),
            remote_path,
            remote_pid_path,
            remote_pid,
            local_pid: child.pid,
            local_relative_path: local_relative,
            started_at: now_rfc3339(),
            requested_time_limit_secs: requested,
            time_limit_secs: time_limit,
            bit_rate,
            size,
            status: "recording".to_string(),
            finalize_on_stop,
            notes: notes.clone(),
        };
        let path = self.state_path(session_id);
        if let Some(parent) = path.parent() {
            ensure_dir(parent)?;
        }
        write_json_atomic(&path, &state)?;
        Ok(state)
    }

    pub async fn status(&self, session_id: &str, serial: &str) -> Result<RecordingStatus> {
        let Some(state) = self.load_state(session_id)? else {
            return Ok(RecordingStatus {
                recording: None,
                local_process_alive: false,
                remote_size_bytes: None,
                age_ms: 0,
            });
        };
        let local_process_alive = process_exists(state.local_pid);
        let remote_size_bytes = self
            .adb
            .shell_args(
                serial,
                &[
                    "stat".to_string(),
                    "-c".to_string(),
                    "%s".to_string(),
                    state.remote_path.clone(),
                ],
                Some(Duration::from_secs(8)),
                None,
            )
            .await
            .ok()
            .and_then(|output| output.stdout_text().trim().parse::<u64>().ok());
        let age_ms = chrono::DateTime::parse_from_rfc3339(&state.started_at)
            .map(|started| {
                chrono::Utc::now()
                    .signed_duration_since(started.with_timezone(&chrono::Utc))
                    .num_milliseconds()
                    .max(0) as u64
            })
            .unwrap_or(0);
        Ok(RecordingStatus {
            recording: Some(state),
            local_process_alive,
            remote_size_bytes,
            age_ms,
        })
    }

    pub async fn stop(
        &self,
        session_id: &str,
        run: &mut RunContext,
        trace: bool,
    ) -> Result<RecordingOutcome> {
        let Some(state) = self.load_state(session_id)? else {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                format!("no recording is active for session {session_id}"),
            )
            .with_suggestion("Run `androidforge capture record start` first.")
            .session(session_id.to_string()));
        };
        if state.run_id != run.run_id {
            return Err(ForgeError::new(
                ErrorCode::SessionOwnershipMismatch,
                format!(
                    "recording belongs to run {}, not the active run {}",
                    state.run_id, run.run_id
                ),
            )
            .with_suggestion("Open the recording's run or stop it from the original run context.")
            .session(session_id.to_string()));
        }
        let mut notes = state.notes.clone();
        if let Some(remote_pid) = state.remote_pid {
            let _ = self
                .adb
                .shell_args(
                    &state.serial,
                    &["kill".to_string(), "-2".to_string(), remote_pid.to_string()],
                    Some(Duration::from_secs(10)),
                    None,
                )
                .await;
        } else {
            notes.push(
                "remote recording PID was unavailable; the recording may continue until its device-side time limit"
                    .to_string(),
            );
            kill_tree(state.local_pid);
        }
        let started = Instant::now();
        while process_exists(state.local_pid) && started.elapsed() < Duration::from_secs(12) {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if process_exists(state.local_pid) {
            kill_tree(state.local_pid);
            notes.push("local adb screenrecord process had to be terminated".to_string());
        }
        let local_path = run.dir.join(&state.local_relative_path);
        if let Some(parent) = local_path.parent() {
            ensure_dir(parent)?;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        self.adb
            .pull(&state.serial, &state.remote_path, &local_path)
            .await
            .map_err(|error| {
                ForgeError::new(
                    ErrorCode::CommandFailed,
                    "could not pull the finished recording from the device",
                )
                .with_cause(error.message())
                .with_context_string("remote_path", state.remote_path.clone())
                .with_suggestion(
                    "The recording may still exist on the device; inspect it with raw ADB.",
                )
                .device(state.serial.clone())
            })?;
        let size = std::fs::metadata(&local_path)
            .map_err(|error| {
                ForgeError::new(
                    ErrorCode::ArtifactWriteFailed,
                    format!("recording was pulled but cannot be inspected: {error}"),
                )
            })?
            .len();
        if size == 0 {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                "the device produced an empty recording file",
            )
            .with_context_string("remote_path", state.remote_path.clone())
            .with_suggestion(
                "Check whether screenrecord is supported and whether the screen was active.",
            )
            .device(state.serial.clone()));
        }
        let _ = self
            .adb
            .shell_args(
                &state.serial,
                &[
                    "rm".to_string(),
                    "-f".to_string(),
                    state.remote_path.clone(),
                ],
                Some(Duration::from_secs(10)),
                None,
            )
            .await;
        let _ = self
            .adb
            .shell_args(
                &state.serial,
                &[
                    "rm".to_string(),
                    "-f".to_string(),
                    state.remote_pid_path.clone(),
                ],
                Some(Duration::from_secs(10)),
                None,
            )
            .await;
        let artifact = run.write_binary_artifact(
            &state.local_relative_path,
            "recording",
            Some("Android screen recording"),
            &std::fs::read(&local_path).unwrap_or_default(),
        )?;
        let metadata = serde_json::json!({
            "schema_version": 1,
            "session_id": state.session_id.clone(),
            "serial": state.serial.clone(),
            "started_at": state.started_at.clone(),
            "stopped_at": now_rfc3339(),
            "requested_time_limit_secs": state.requested_time_limit_secs,
            "time_limit_secs": state.time_limit_secs,
            "bit_rate": state.bit_rate,
            "size": state.size.clone(),
            "remote_path": state.remote_path.clone(),
            "sha256": artifact.sha256.clone(),
            "notes": notes,
        });
        let _ = run.write_text_artifact(
            &format!("{}.json", state.local_relative_path),
            "recording_metadata",
            Some("Recording parameters and checksum"),
            &serde_json::to_string_pretty(&metadata).unwrap_or_default(),
        );
        let _ = std::fs::remove_file(self.state_path(session_id));
        let _ = trace;
        Ok(RecordingOutcome {
            artifact,
            local_path: local_path.to_string_lossy().into_owned(),
            size_bytes: size,
            duration_ms: started.elapsed().as_millis() as u64,
            remote_path: state.remote_path,
            notes,
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
    fn png_dimensions_are_big_endian() {
        let bytes = [0u8, 0, 0, 42, 0, 0, 1, 44];
        assert_eq!(
            u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            42
        );
    }

    #[test]
    fn recording_keys_are_safe() {
        assert_eq!(sanitize_key("ses/../../x"), "ses_______x");
    }
}
