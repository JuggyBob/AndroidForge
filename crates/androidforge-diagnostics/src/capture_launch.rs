//! Bounded repeated-`screencap` sampler for launch first-frame evidence.
//!
//! This module deliberately avoids ffmpeg and MP4 decoders. Each sample is a
//! single `adb exec-out screencap -p` PNG held in memory, decoded through the
//! one PNG decoder in `androidforge_ui::visual`, and optionally persisted into
//! the run's artifact directory. No file is ever created on the device by the
//! sampling path, and the optional UI-dump fallback reads the hierarchy from
//! `/dev/tty` rather than a device temp file.
//!
//! The public surface is split in three layers so it can be tested without a
//! device:
//!
//! * [`LaunchFrameSource`] supplies synthetic or live [`LaunchFrame`] values.
//! * [`analyze_launch_frames`] is pure analysis over those values.
//! * [`assert_first_frame`] turns the result into an explicit
//!   `E_UI_ASSERTION_FAILED` with `actual`/`expected`/`tolerance` context.
//!
//! [`sample_launch_capture`] joins the layers, enforces all bounds and writes
//! artifacts through [`LaunchArtifactSink`]. Cancellation is checked before
//! every sample and around every wait; a dropped ADB future kills its local
//! process tree through `androidforge-process`' `kill_on_drop` behaviour.

use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::run::RunContext;
use androidforge_core::util::{now_rfc3339, sha256_hex};
use androidforge_ui::{UiHierarchy, color, visual};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;

pub const CAPTURE_LAUNCH_SCHEMA_VERSION: u32 = 1;

pub const MIN_FPS: u32 = 1;
pub const MAX_FPS: u32 = 30;
pub const DEFAULT_FPS: u32 = 10;
pub const MIN_DURATION_MS: u64 = 100;
pub const MAX_DURATION_MS: u64 = 30_000;
pub const DEFAULT_DURATION_MS: u64 = 5_000;
pub const MAX_FRAME_TIMEOUT_MS: u64 = 15_000;
pub const DEFAULT_FRAME_TIMEOUT_MS: u64 = 5_000;
pub const ABSOLUTE_MAX_FRAMES: usize = 900;
pub const DEFAULT_MAX_FRAMES: usize = 150;
pub const ABSOLUTE_MAX_FRAME_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_MAX_FRAME_BYTES: u64 = 16 * 1024 * 1024;
pub const ABSOLUTE_MAX_TOTAL_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
pub const DEFAULT_MAX_TOTAL_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
pub const DEFAULT_WHITE_THRESHOLD: u8 = 245;
pub const DEFAULT_BLACK_THRESHOLD: u8 = 10;
pub const DEFAULT_TOLERANCE: u8 = 16;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const UI_DUMP_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Which colour of the first app frame an expected-colour assertion checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorSampleKind {
    /// Pixel at the exact centre of the frame (the default).
    #[default]
    Centre,
    /// Most frequent RGB colour in the frame.
    Dominant,
}

impl ColorSampleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Centre => "centre",
            Self::Dominant => "dominant",
        }
    }
}

/// Typed input for `capture launch`. Every field has a serde default so older
/// callers and partial JSON documents remain readable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct CaptureLaunchOptions {
    /// Requested samples per second, clamped to `MIN_FPS..=MAX_FPS`.
    pub fps: u32,
    /// Requested sample window in milliseconds, clamped to
    /// `MIN_DURATION_MS..=MAX_DURATION_MS`.
    pub duration_ms: u64,
    /// Package whose focus marks the first app frame.
    pub package: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// Fall back to `uiautomator dump /dev/tty` package name when focus does
    /// not match. No device temp file is created.
    pub use_ui_dump_package: bool,
    /// Assert that an app frame was observed; otherwise return
    /// `E_UI_ASSERTION_FAILED`.
    pub expect_first_frame: bool,
    /// Assert that the first app frame is not a blank white/black frame.
    pub no_splash: bool,
    /// Optional expected `#RRGGBB` colour for the first app frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_color: Option<String>,
    pub expected_color_sample: ColorSampleKind,
    pub tolerance: u8,
    pub white_threshold: u8,
    pub black_threshold: u8,
    pub max_frames: usize,
    pub max_frame_bytes: u64,
    pub max_total_artifact_bytes: u64,
    pub frame_timeout_ms: u64,
}

impl Default for CaptureLaunchOptions {
    fn default() -> Self {
        Self {
            fps: DEFAULT_FPS,
            duration_ms: DEFAULT_DURATION_MS,
            package: String::new(),
            activity: None,
            use_ui_dump_package: false,
            expect_first_frame: false,
            no_splash: false,
            expected_color: None,
            expected_color_sample: ColorSampleKind::Centre,
            tolerance: DEFAULT_TOLERANCE,
            white_threshold: DEFAULT_WHITE_THRESHOLD,
            black_threshold: DEFAULT_BLACK_THRESHOLD,
            max_frames: DEFAULT_MAX_FRAMES,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_total_artifact_bytes: DEFAULT_MAX_TOTAL_ARTIFACT_BYTES,
            frame_timeout_ms: DEFAULT_FRAME_TIMEOUT_MS,
        }
    }
}

/// An RGB colour as both hex and numeric channels, ready for JSON evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorSample {
    pub hex: String,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Default for ColorSample {
    fn default() -> Self {
        Self {
            hex: "#000000".to_string(),
            r: 0,
            g: 0,
            b: 0,
        }
    }
}

impl From<color::Rgb> for ColorSample {
    fn from(value: color::Rgb) -> Self {
        Self {
            hex: value.hex(),
            r: value.r,
            g: value.g,
            b: value.b,
        }
    }
}

impl ColorSample {
    pub fn rgb(&self) -> color::Rgb {
        color::Rgb::new(self.r, self.g, self.b)
    }
}

/// One sample supplied by a [`LaunchFrameSource`].
///
/// `elapsed_ms` is relative to sampler start so synthetic sources can provide
/// deterministic timestamps. `focus_package` comes from
/// `adb.current_focus().package`; `ui_package` is only populated when the UI
/// dump fallback is enabled.
#[derive(Debug, Clone)]
pub struct LaunchFrame {
    pub png: Vec<u8>,
    pub elapsed_ms: u64,
    pub focus_package: Option<String>,
    pub ui_package: Option<String>,
}

impl LaunchFrame {
    /// Whether this frame belongs to `package` by focus or UI package.
    pub fn belongs_to(&self, package: &str) -> bool {
        self.focus_package.as_deref() == Some(package)
            || self.ui_package.as_deref() == Some(package)
    }
}

/// Terminal state of a bounded launch sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchCaptureStatus {
    FirstFrameFound,
    Timeout,
    Cancelled,
    FrameLimit,
    ArtifactLimit,
    SourceExhausted,
}

impl LaunchCaptureStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FirstFrameFound => "first_frame_found",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::FrameLimit => "frame_limit",
            Self::ArtifactLimit => "artifact_limit",
            Self::SourceExhausted => "source_exhausted",
        }
    }

    /// Whether this terminal state implies no app frame was observed.
    pub fn has_no_frame(self) -> bool {
        !matches!(self, Self::FirstFrameFound)
    }
}

/// A persisted frame and its evidence metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchFrameArtifact {
    pub index: usize,
    pub frame_index: usize,
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub elapsed_ms: u64,
    pub is_first_app_frame: bool,
}

/// Typed result for `capture launch`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchCaptureResult {
    pub schema_version: u32,
    pub package: String,
    pub serial: String,
    pub collected_at: String,
    pub status: LaunchCaptureStatus,
    pub fps: u32,
    pub requested_duration_ms: u64,
    pub sample_window_ms: u64,
    pub sampled_duration_ms: u64,
    pub frames_captured: usize,
    pub frames_analyzed: usize,
    pub dropped_frames: usize,
    pub white_threshold: u8,
    pub black_threshold: u8,
    pub white_frames: usize,
    pub black_frames: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_app_frame_ms: Option<u64>,
    /// Run-relative path of the first app frame PNG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_frame_png: Option<String>,
    /// Centre pixel of the first app frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_frame_rgb: Option<ColorSample>,
    /// Most frequent RGB colour of the first app frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dominant_color: Option<ColorSample>,
    pub artifacts: Vec<LaunchFrameArtifact>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_frame_reason: Option<String>,
}

impl Default for LaunchCaptureResult {
    fn default() -> Self {
        Self {
            schema_version: CAPTURE_LAUNCH_SCHEMA_VERSION,
            package: String::new(),
            serial: String::new(),
            collected_at: String::new(),
            status: LaunchCaptureStatus::Timeout,
            fps: DEFAULT_FPS,
            requested_duration_ms: DEFAULT_DURATION_MS,
            sample_window_ms: DEFAULT_DURATION_MS,
            sampled_duration_ms: 0,
            frames_captured: 0,
            frames_analyzed: 0,
            dropped_frames: 0,
            white_threshold: DEFAULT_WHITE_THRESHOLD,
            black_threshold: DEFAULT_BLACK_THRESHOLD,
            white_frames: 0,
            black_frames: 0,
            first_app_frame_ms: None,
            first_frame_png: None,
            first_frame_rgb: None,
            dominant_color: None,
            artifacts: Vec::new(),
            warnings: Vec::new(),
            no_frame_reason: None,
        }
    }
}

/// Explicit assertion primitive backing `--expect-first-frame` and
/// `--no-splash`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct FirstFrameExpectation {
    pub expect_first_frame: bool,
    pub no_splash: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_color: Option<String>,
    pub expected_color_sample: ColorSampleKind,
    pub tolerance: u8,
}

impl Default for FirstFrameExpectation {
    fn default() -> Self {
        Self {
            expect_first_frame: false,
            no_splash: false,
            expected_color: None,
            expected_color_sample: ColorSampleKind::Centre,
            tolerance: DEFAULT_TOLERANCE,
        }
    }
}

impl From<&CaptureLaunchOptions> for FirstFrameExpectation {
    fn from(options: &CaptureLaunchOptions) -> Self {
        Self {
            expect_first_frame: options.expect_first_frame,
            no_splash: options.no_splash,
            expected_color: options.expected_color.clone(),
            expected_color_sample: options.expected_color_sample,
            tolerance: options.tolerance,
        }
    }
}

/// Successful assertion evidence. Failures are returned as `ForgeError`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FirstFrameAssertion {
    pub passed: bool,
    pub expect_first_frame: bool,
    pub no_splash: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_color_sample: Option<ColorSampleKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_color: Option<ColorSample>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_app_frame_ms: Option<u64>,
    pub tolerance: u8,
}

impl Default for FirstFrameAssertion {
    fn default() -> Self {
        Self {
            passed: false,
            expect_first_frame: false,
            no_splash: false,
            expected_color: None,
            expected_color_sample: None,
            checked_color: None,
            first_app_frame_ms: None,
            tolerance: DEFAULT_TOLERANCE,
        }
    }
}

/// Cooperative cancellation shared by the sampler and its frame source.
#[derive(Debug, Clone)]
pub struct LaunchCancellation {
    /// Persistent flag: `watch::send` fails when nobody is subscribed yet, so
    /// the authoritative state must not live only in the channel.
    flag: Arc<AtomicBool>,
    sender: watch::Sender<bool>,
}

impl Default for LaunchCancellation {
    fn default() -> Self {
        Self::new()
    }
}

impl LaunchCancellation {
    pub fn new() -> Self {
        let (sender, _) = watch::channel(false);
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            sender,
        }
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        let _ = self.sender.send(true);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Resolves once cancellation has been requested.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let mut receiver = self.sender.subscribe();
        while !*receiver.borrow() {
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }
}

/// Supplies frames to [`sample_launch_capture`]. Async so live ADB calls and
/// synthetic sequences share the same sampling loop.
#[allow(async_fn_in_trait)]
pub trait LaunchFrameSource {
    async fn next_frame(
        &mut self,
        timeout: Duration,
        cancellation: &LaunchCancellation,
    ) -> Result<Option<LaunchFrame>>;
}

/// Sink for bounded frame artifacts. `RunContext` implements it directly;
/// tests can record writes without touching a real run directory.
pub trait LaunchArtifactSink {
    fn write_frame(&mut self, index: usize, png: &[u8]) -> Result<String>;
}

impl LaunchArtifactSink for RunContext {
    fn write_frame(&mut self, index: usize, png: &[u8]) -> Result<String> {
        let relative = format!("capture-launch/frame-{index:04}.png");
        let artifact = self.write_binary_artifact(
            &relative,
            "launch_frame",
            Some("Bounded launch sample frame"),
            png,
        )?;
        Ok(artifact.path)
    }
}

/// Live frame source using only `adb exec-out screencap -p` and read-only ADB
/// queries. It creates no device temp files and starts no long-lived remote
/// process.
pub struct AdbLaunchFrameSource<'a> {
    adb: &'a AdbClient,
    serial: String,
    package: String,
    use_ui_dump_package: bool,
    /// Set on the first sample so `elapsed_ms` is relative to actual capture.
    started: Option<tokio::time::Instant>,
}

impl<'a> AdbLaunchFrameSource<'a> {
    pub fn new(
        adb: &'a AdbClient,
        serial: impl Into<String>,
        package: impl Into<String>,
        use_ui_dump_package: bool,
    ) -> Self {
        Self {
            adb,
            serial: serial.into(),
            package: package.into(),
            use_ui_dump_package,
            started: None,
        }
    }
}

impl LaunchFrameSource for AdbLaunchFrameSource<'_> {
    async fn next_frame(
        &mut self,
        timeout: Duration,
        cancellation: &LaunchCancellation,
    ) -> Result<Option<LaunchFrame>> {
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        let output = self
            .adb
            .exec_out_args(
                &self.serial,
                &["screencap".to_string(), "-p".to_string()],
                Some(timeout),
                Some(ABSOLUTE_MAX_FRAME_BYTES as usize),
            )
            .await?;
        if output.stdout_truncated {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                "screencap PNG exceeded the sampler's byte limit",
            )
            .device(self.serial.clone()));
        }
        let png = output.stdout;
        if png.len() < 24 || &png[..8] != PNG_SIGNATURE {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                "screencap did not return a valid PNG",
            )
            .with_context_string("bytes", png.len().to_string())
            .device(self.serial.clone()));
        }
        let focus_package =
            match tokio::time::timeout(timeout, self.adb.current_focus(&self.serial)).await {
                Ok(Some(focus)) => Some(focus.package),
                Ok(None) | Err(_) => None,
            };
        let wants_ui_dump =
            self.use_ui_dump_package && focus_package.as_deref() != Some(self.package.as_str());
        let ui_package = if wants_ui_dump {
            ui_dump_package(self.adb, &self.serial, timeout).await
        } else {
            None
        };
        let started = self.started.get_or_insert_with(tokio::time::Instant::now);
        Ok(Some(LaunchFrame {
            png,
            elapsed_ms: duration_ms(started.elapsed()),
            focus_package,
            ui_package,
        }))
    }
}

/// Read the hierarchy package without writing a device temp file. Failures are
/// deliberately soft: focus remains the authoritative signal.
async fn ui_dump_package(adb: &AdbClient, serial: &str, timeout: Duration) -> Option<String> {
    let output = tokio::time::timeout(
        timeout,
        adb.exec_out_args(
            serial,
            &[
                "uiautomator".to_string(),
                "dump".to_string(),
                "/dev/tty".to_string(),
            ],
            Some(timeout),
            Some(UI_DUMP_MAX_BYTES),
        ),
    )
    .await
    .ok()?
    .ok()?;
    if output.stdout_truncated {
        return None;
    }
    let xml = output.stdout_text();
    if !xml.contains("<hierarchy") {
        return None;
    }
    UiHierarchy::parse(&xml).ok()?.package_name
}

/// Pure first-frame and colour analysis over synthetic or live frames.
#[derive(Debug, Clone)]
pub struct FrameAnalysisInput<'a> {
    pub frames: &'a [LaunchFrame],
    pub package: &'a str,
    pub white_threshold: u8,
    pub black_threshold: u8,
}

/// Outcome of [`analyze_launch_frames`].
#[derive(Debug, Clone, Default)]
pub struct LaunchAnalysis {
    pub first_app_frame_index: Option<usize>,
    pub first_app_frame_ms: Option<u64>,
    pub white_frames: usize,
    pub black_frames: usize,
    pub first_frame_rgb: Option<ColorSample>,
    pub dominant_color: Option<ColorSample>,
}

/// Decode each PNG through `androidforge_ui` and compute first-frame evidence.
/// White/black classification uses the frame's dominant (most frequent) RGB
/// colour so a small icon on a blank background does not hide a blank frame.
pub fn analyze_launch_frames(input: FrameAnalysisInput<'_>) -> Result<LaunchAnalysis> {
    if input.package.trim().is_empty() {
        return Err(ForgeError::invalid_argument(
            "launch capture requires a non-empty package",
        ));
    }
    let mut analysis = LaunchAnalysis::default();
    for (index, frame) in input.frames.iter().enumerate() {
        let image =
            visual::decode_reader(Cursor::new(&frame.png), &format!("launch frame {index}"))?;
        let rgb = color::sample_rgb(&image, image.width / 2, image.height / 2)?;
        let dominant = color::majority_color(&image, color::Region::full(&image))?;
        if is_white(dominant, input.white_threshold) {
            analysis.white_frames += 1;
        }
        if is_black(dominant, input.black_threshold) {
            analysis.black_frames += 1;
        }
        if analysis.first_app_frame_index.is_none() && frame.belongs_to(input.package) {
            analysis.first_app_frame_index = Some(index);
            analysis.first_app_frame_ms = Some(frame.elapsed_ms);
            analysis.first_frame_rgb = Some(ColorSample::from(rgb));
            analysis.dominant_color = Some(ColorSample::from(dominant));
        }
    }
    Ok(analysis)
}

fn is_white(rgb: color::Rgb, threshold: u8) -> bool {
    rgb.r >= threshold && rgb.g >= threshold && rgb.b >= threshold
}

fn is_black(rgb: color::Rgb, threshold: u8) -> bool {
    rgb.r <= threshold && rgb.g <= threshold && rgb.b <= threshold
}

/// Assert the first-frame expectations. On failure the returned error is
/// always `E_UI_ASSERTION_FAILED` and carries `actual`, `expected` and
/// `tolerance` in its structured context.
pub fn assert_first_frame(
    result: &LaunchCaptureResult,
    expectation: &FirstFrameExpectation,
) -> Result<FirstFrameAssertion> {
    let mut assertion = FirstFrameAssertion {
        expect_first_frame: expectation.expect_first_frame,
        no_splash: expectation.no_splash,
        expected_color_sample: expectation
            .expected_color
            .as_ref()
            .map(|_| expectation.expected_color_sample),
        tolerance: expectation.tolerance,
        first_app_frame_ms: result.first_app_frame_ms,
        ..FirstFrameAssertion::default()
    };

    if expectation.expect_first_frame && result.first_app_frame_ms.is_none() {
        return Err(assertion_error(
            format!(
                "no first app frame for {} was observed within {} ms",
                result.package, result.sample_window_ms
            ),
            json!("none"),
            json!(result.package),
            json!(0),
        )
        .with_context("status", json!(result.status.as_str()))
        .with_context("no_frame_reason", json!(result.no_frame_reason))
        .device(result.serial.as_str()));
    }

    if expectation.no_splash {
        let Some(sample) = result.dominant_color.as_ref() else {
            return Err(assertion_error(
                "cannot check --no-splash without a decoded first app frame",
                json!("none"),
                json!("non-blank first frame"),
                json!(0),
            )
            .device(result.serial.as_str()));
        };
        let rgb = sample.rgb();
        let blank = if is_white(rgb, result.white_threshold) {
            Some("white")
        } else if is_black(rgb, result.black_threshold) {
            Some("black")
        } else {
            None
        };
        if let Some(blank) = blank {
            return Err(assertion_error(
                format!(
                    "first app frame for {} is a blank {} splash frame ({})",
                    result.package, blank, sample.hex
                ),
                json!(sample.hex),
                json!("non-blank first frame"),
                json!(0),
            )
            .with_context("classification", json!(blank))
            .with_context("first_app_frame_ms", json!(result.first_app_frame_ms))
            .device(result.serial.as_str()));
        }
    }

    if let Some(expected_hex) = &expectation.expected_color {
        let expected = color::parse_hex(expected_hex)?;
        let (sample, kind) = match expectation.expected_color_sample {
            ColorSampleKind::Centre => (result.first_frame_rgb.as_ref(), "centre"),
            ColorSampleKind::Dominant => (result.dominant_color.as_ref(), "dominant"),
        };
        let Some(sample) = sample else {
            return Err(assertion_error(
                "cannot check expected colour without a decoded first app frame",
                json!("none"),
                json!(expected.hex()),
                json!(expectation.tolerance),
            )
            .device(result.serial.as_str()));
        };
        let comparison = color::compare(expected, sample.rgb(), expectation.tolerance);
        assertion.expected_color = Some(expected.hex());
        assertion.checked_color = Some(sample.clone());
        if !comparison.passed {
            return Err(assertion_error(
                format!(
                    "first frame {kind} colour {} did not match expected {} (delta {}, tolerance {})",
                    sample.hex, expected.hex(), comparison.max_channel_delta,
                    expectation.tolerance
                ),
                json!(sample.hex),
                json!(expected.hex()),
                json!(expectation.tolerance),
            )
            .with_context("max_channel_delta", json!(comparison.max_channel_delta))
            .with_context("sample", json!(kind))
            .with_context("first_app_frame_ms", json!(result.first_app_frame_ms))
            .device(result.serial.as_str()));
        }
    }

    assertion.passed = true;
    Ok(assertion)
}

fn assertion_error(
    message: impl Into<String>,
    actual: Value,
    expected: Value,
    tolerance: Value,
) -> ForgeError {
    ForgeError::new(ErrorCode::UiAssertionFailed, message)
        .with_context("actual", actual)
        .with_context("expected", expected)
        .with_context("tolerance", tolerance)
}

#[derive(Debug)]
struct EffectiveLimits {
    fps: u32,
    duration: Duration,
    frame_timeout: Duration,
    max_frames: usize,
    max_frame_bytes: u64,
    max_total_bytes: u64,
    warnings: Vec<String>,
}

fn effective_limits(options: &CaptureLaunchOptions) -> EffectiveLimits {
    let mut warnings = Vec::new();
    let fps = options.fps.clamp(MIN_FPS, MAX_FPS);
    if fps != options.fps {
        warnings.push(format!(
            "requested fps {} was clamped to {fps} (allowed {MIN_FPS}..={MAX_FPS})",
            options.fps
        ));
    }
    let duration_ms = options.duration_ms.clamp(MIN_DURATION_MS, MAX_DURATION_MS);
    if duration_ms != options.duration_ms {
        warnings.push(format!(
            "requested duration {} ms was clamped to {duration_ms} ms (allowed {MIN_DURATION_MS}..={MAX_DURATION_MS})",
            options.duration_ms
        ));
    }
    let frame_timeout_ms = options.frame_timeout_ms.clamp(1, MAX_FRAME_TIMEOUT_MS);
    if frame_timeout_ms != options.frame_timeout_ms {
        warnings.push(format!(
            "requested frame timeout {} ms was clamped to {frame_timeout_ms} ms",
            options.frame_timeout_ms
        ));
    }
    let max_frames = options.max_frames.clamp(1, ABSOLUTE_MAX_FRAMES);
    if max_frames != options.max_frames {
        warnings.push(format!(
            "requested max frames {} was clamped to {max_frames}",
            options.max_frames
        ));
    }
    let max_frame_bytes = options.max_frame_bytes.clamp(1, ABSOLUTE_MAX_FRAME_BYTES);
    if max_frame_bytes != options.max_frame_bytes {
        warnings.push(format!(
            "requested max frame bytes {} was clamped to {max_frame_bytes}",
            options.max_frame_bytes
        ));
    }
    let max_total_bytes = options
        .max_total_artifact_bytes
        .clamp(max_frame_bytes, ABSOLUTE_MAX_TOTAL_ARTIFACT_BYTES);
    if max_total_bytes != options.max_total_artifact_bytes {
        warnings.push(format!(
            "requested max total artifact bytes {} was clamped to {max_total_bytes}",
            options.max_total_artifact_bytes
        ));
    }
    EffectiveLimits {
        fps,
        duration: Duration::from_millis(duration_ms),
        frame_timeout: Duration::from_millis(frame_timeout_ms),
        max_frames,
        max_frame_bytes,
        max_total_bytes,
        warnings,
    }
}

/// Run a bounded sample. Frames are paced at the requested fps; the loop ends
/// on first-frame detection, window expiry, cancellation, or an artifact
/// bound. Frames rejected by a bound are counted in `dropped_frames` and never
/// reach the sink.
pub async fn sample_launch_capture<S: LaunchFrameSource>(
    source: &mut S,
    mut sink: Option<&mut dyn LaunchArtifactSink>,
    serial: &str,
    options: &CaptureLaunchOptions,
    cancellation: &LaunchCancellation,
) -> Result<LaunchCaptureResult> {
    if options.package.trim().is_empty() {
        return Err(ForgeError::invalid_argument(
            "launch capture requires a non-empty package",
        ));
    }
    let limits = effective_limits(options);
    let started = tokio::time::Instant::now();
    let mut warnings = limits.warnings.clone();
    let mut frames: Vec<LaunchFrame> = Vec::new();
    let mut artifacts: Vec<LaunchFrameArtifact> = Vec::new();
    let mut total_bytes = 0u64;
    let mut attempts = 0usize;
    let mut dropped_frames = 0usize;
    let mut status: LaunchCaptureStatus;

    loop {
        if attempts >= limits.max_frames {
            status = LaunchCaptureStatus::FrameLimit;
            break;
        }
        if cancellation.is_cancelled() {
            status = LaunchCaptureStatus::Cancelled;
            break;
        }
        let elapsed = started.elapsed();
        if elapsed >= limits.duration {
            status = LaunchCaptureStatus::Timeout;
            break;
        }
        let next_due = Duration::from_millis((attempts as u64) * 1000 / u64::from(limits.fps));
        if next_due > elapsed {
            let wait = (next_due - elapsed).min(limits.duration - elapsed);
            tokio::select! {
                _ = cancellation.cancelled() => {
                    status = LaunchCaptureStatus::Cancelled;
                    break;
                }
                _ = tokio::time::sleep(wait) => {}
            }
            continue;
        }
        let frame_timeout = limits
            .frame_timeout
            .min(limits.duration - started.elapsed());
        let next = tokio::select! {
            _ = cancellation.cancelled() => {
                status = LaunchCaptureStatus::Cancelled;
                break;
            }
            result = source.next_frame(frame_timeout, cancellation) => result,
        };
        let frame = match next {
            Ok(Some(frame)) => frame,
            Ok(None) => {
                status = if started.elapsed() >= limits.duration {
                    LaunchCaptureStatus::Timeout
                } else {
                    LaunchCaptureStatus::SourceExhausted
                };
                break;
            }
            Err(error) if error.code() == ErrorCode::CommandTimeout => {
                warnings.push("a screencap sample timed out; ending the sample window".to_string());
                status = LaunchCaptureStatus::Timeout;
                break;
            }
            Err(error) => return Err(error.device(serial.to_string())),
        };
        attempts += 1;
        let bytes = frame.png.len() as u64;
        if bytes > limits.max_frame_bytes {
            dropped_frames += 1;
            let warning = format!(
                "frame {attempts} was {bytes} bytes and exceeded the per-frame limit of {} bytes",
                limits.max_frame_bytes
            );
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
            continue;
        }
        match total_bytes.checked_add(bytes) {
            Some(total) if total <= limits.max_total_bytes => total_bytes = total,
            _ => {
                dropped_frames += 1;
                warnings.push(format!(
                    "artifact byte budget of {} bytes was reached after {} stored frames",
                    limits.max_total_bytes,
                    frames.len()
                ));
                status = LaunchCaptureStatus::ArtifactLimit;
                break;
            }
        }
        let frame_index = frames.len();
        let path = if let Some(sink) = sink.as_mut() {
            Some(sink.write_frame(attempts - 1, &frame.png)?)
        } else {
            None
        };
        if let Some(path) = path {
            artifacts.push(LaunchFrameArtifact {
                index: attempts - 1,
                frame_index,
                path,
                sha256: sha256_hex(&frame.png),
                bytes,
                elapsed_ms: frame.elapsed_ms,
                is_first_app_frame: frame.belongs_to(&options.package),
            });
        }
        frames.push(frame);
    }

    let analysis = analyze_launch_frames(FrameAnalysisInput {
        frames: &frames,
        package: &options.package,
        white_threshold: options.white_threshold,
        black_threshold: options.black_threshold,
    })?;
    if let Some(index) = analysis.first_app_frame_index {
        status = LaunchCaptureStatus::FirstFrameFound;
        if let Some(artifact) = artifacts
            .iter_mut()
            .find(|artifact| artifact.frame_index == index)
        {
            artifact.is_first_app_frame = true;
        }
    }
    let no_frame_reason = if status.has_no_frame() {
        Some(match status {
            LaunchCaptureStatus::Cancelled => {
                "sampling was cancelled before an app frame appeared".to_string()
            }
            LaunchCaptureStatus::ArtifactLimit => {
                "artifact bounds stopped sampling before an app frame appeared".to_string()
            }
            LaunchCaptureStatus::FrameLimit => {
                "frame count bound was reached before an app frame appeared".to_string()
            }
            LaunchCaptureStatus::SourceExhausted => {
                "frame source was exhausted before an app frame appeared".to_string()
            }
            LaunchCaptureStatus::Timeout | LaunchCaptureStatus::FirstFrameFound => format!(
                "no frame with focus or UI package {} was observed within {} ms",
                options.package,
                limits.duration.as_millis()
            ),
        })
    } else {
        None
    };

    let first_frame_png = analysis.first_app_frame_index.and_then(|index| {
        artifacts
            .iter()
            .find(|artifact| artifact.frame_index == index)
            .map(|artifact| artifact.path.clone())
    });

    Ok(LaunchCaptureResult {
        schema_version: CAPTURE_LAUNCH_SCHEMA_VERSION,
        package: options.package.clone(),
        serial: serial.to_string(),
        collected_at: now_rfc3339(),
        status,
        fps: limits.fps,
        requested_duration_ms: options.duration_ms,
        sample_window_ms: duration_ms(limits.duration),
        sampled_duration_ms: duration_ms(started.elapsed()),
        frames_captured: frames.len(),
        frames_analyzed: frames.len(),
        dropped_frames,
        white_threshold: options.white_threshold,
        black_threshold: options.black_threshold,
        white_frames: analysis.white_frames,
        black_frames: analysis.black_frames,
        first_app_frame_ms: analysis.first_app_frame_ms,
        first_frame_png,
        first_frame_rgb: analysis.first_frame_rgb,
        dominant_color: analysis.dominant_color,
        artifacts,
        warnings,
        no_frame_reason,
    })
}

fn duration_ms(value: Duration) -> u64 {
    value.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    fn solid_png(rgb: [u8; 3]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            // 64x64 keeps fixtures above the 1 KiB per-frame floor while the
            // deterministic single colour makes dominant colour unambiguous.
            let mut encoder = png::Encoder::new(&mut bytes, 64, 64);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("png header");
            let pixels = (0..64 * 64).flat_map(|_| rgb).collect::<Vec<_>>();
            writer.write_image_data(&pixels).expect("png pixels");
        }
        bytes
    }

    fn frame(rgb: [u8; 3], elapsed_ms: u64, focus: Option<&str>, ui: Option<&str>) -> LaunchFrame {
        LaunchFrame {
            png: solid_png(rgb),
            elapsed_ms,
            focus_package: focus.map(str::to_string),
            ui_package: ui.map(str::to_string),
        }
    }

    #[derive(Default)]
    struct FakeSource {
        frames: VecDeque<LaunchFrame>,
        cancel_on_first: bool,
    }

    impl LaunchFrameSource for FakeSource {
        async fn next_frame(
            &mut self,
            _timeout: Duration,
            cancellation: &LaunchCancellation,
        ) -> Result<Option<LaunchFrame>> {
            if self.cancel_on_first {
                cancellation.cancel();
            }
            Ok(self.frames.pop_front())
        }
    }

    /// A source that produces no frames and consumes the remaining window,
    /// mirroring a device that never shows the target app.
    struct TimeoutSource;

    impl LaunchFrameSource for TimeoutSource {
        async fn next_frame(
            &mut self,
            timeout: Duration,
            _cancellation: &LaunchCancellation,
        ) -> Result<Option<LaunchFrame>> {
            tokio::time::sleep(timeout).await;
            Ok(None)
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        writes: Vec<(usize, Vec<u8>, String)>,
    }

    impl LaunchArtifactSink for RecordingSink {
        fn write_frame(&mut self, index: usize, png: &[u8]) -> Result<String> {
            let path = format!("capture-launch/frame-{index:04}.png");
            self.writes.push((index, png.to_vec(), path.clone()));
            Ok(path)
        }
    }

    fn options(package: &str) -> CaptureLaunchOptions {
        CaptureLaunchOptions {
            fps: 30,
            duration_ms: 300,
            package: package.to_string(),
            ..CaptureLaunchOptions::default()
        }
    }

    async fn run(
        frames: Vec<LaunchFrame>,
        options: &CaptureLaunchOptions,
    ) -> (LaunchCaptureResult, RecordingSink) {
        run_with(frames, options, false).await
    }

    async fn run_with(
        frames: Vec<LaunchFrame>,
        options: &CaptureLaunchOptions,
        cancel_on_first: bool,
    ) -> (LaunchCaptureResult, RecordingSink) {
        let mut source = FakeSource {
            frames: frames.into(),
            cancel_on_first,
        };
        let mut sink = RecordingSink::default();
        let cancellation = LaunchCancellation::new();
        let result = sample_launch_capture(
            &mut source,
            Some(&mut sink),
            "emulator-5554",
            options,
            &cancellation,
        )
        .await
        .expect("sample");
        (result, sink)
    }

    #[tokio::test]
    async fn first_app_frame_is_detected_by_focus_package() {
        let mut opts = options("com.example.app");
        opts.duration_ms = 200;
        let (result, sink) = run(
            vec![
                frame([255, 255, 255], 0, Some("com.android.launcher"), None),
                frame([255, 0, 0], 100, Some("com.example.app"), None),
                frame([0, 0, 255], 150, Some("com.example.app"), None),
            ],
            &opts,
        )
        .await;
        assert_eq!(result.status, LaunchCaptureStatus::FirstFrameFound);
        assert_eq!(result.first_app_frame_ms, Some(100));
        assert_eq!(result.first_frame_rgb.unwrap().hex, "#FF0000");
        assert_eq!(result.dominant_color.unwrap().hex, "#FF0000");
        assert_eq!(result.white_frames, 1);
        assert_eq!(result.black_frames, 0);
        assert_eq!(result.frames_captured, 3);
        assert_eq!(
            result.first_frame_png.as_deref(),
            Some("capture-launch/frame-0001.png")
        );
        assert_eq!(sink.writes.len(), 3);
        assert!(sink.writes[1].2.ends_with("0001.png"));
    }

    #[tokio::test]
    async fn ui_dump_package_can_supply_first_frame_detection() {
        let mut opts = options("com.example.app");
        opts.duration_ms = 200;
        opts.use_ui_dump_package = true;
        let (result, _) = run(
            vec![
                frame([0, 0, 0], 0, Some("com.android.launcher"), None),
                frame(
                    [10, 20, 30],
                    80,
                    Some("com.android.launcher"),
                    Some("com.example.app"),
                ),
            ],
            &opts,
        )
        .await;
        assert_eq!(result.first_app_frame_ms, Some(80));
        assert_eq!(result.first_frame_rgb.unwrap().hex, "#0A141E");
    }

    #[tokio::test]
    async fn no_frame_window_ends_in_timeout_with_a_reason() {
        let mut opts = options("com.example.app");
        opts.duration_ms = 100;
        let mut timeout_source = TimeoutSource;
        let cancellation = LaunchCancellation::new();
        let result = sample_launch_capture(
            &mut timeout_source,
            None::<&mut dyn LaunchArtifactSink>,
            "emulator-5554",
            &opts,
            &cancellation,
        )
        .await
        .expect("sample");
        assert_eq!(result.status, LaunchCaptureStatus::Timeout);
        assert_eq!(result.first_app_frame_ms, None);
        assert_eq!(result.frames_captured, 0);
        assert!(result.artifacts.is_empty());
        assert!(
            result
                .no_frame_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("com.example.app"))
        );

        let expectation = FirstFrameExpectation {
            expect_first_frame: true,
            ..FirstFrameExpectation::default()
        };
        let error = assert_first_frame(&result, &expectation).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UiAssertionFailed);
        assert_eq!(error.body().context["actual"], json!("none"));
        assert_eq!(error.body().context["expected"], json!("com.example.app"));
        assert_eq!(error.body().context["tolerance"], json!(0));
    }

    #[tokio::test]
    async fn white_and_black_frames_are_counted_by_dominant_colour() {
        let mut opts = options("com.example.app");
        opts.duration_ms = 200;
        let (result, _) = run(
            vec![
                frame([255, 255, 255], 0, Some("com.example.app"), None),
                frame([0, 0, 0], 50, Some("com.example.app"), None),
                frame([255, 0, 0], 100, Some("com.example.app"), None),
            ],
            &opts,
        )
        .await;
        assert_eq!(result.white_frames, 1);
        assert_eq!(result.black_frames, 1);
        assert_eq!(result.first_app_frame_ms, Some(0));
        assert_eq!(result.dominant_color.unwrap().hex, "#FFFFFF");
    }

    async fn result_for_colour(rgb: [u8; 3]) -> LaunchCaptureResult {
        let mut opts = options("com.example.app");
        opts.duration_ms = 100;
        run(vec![frame(rgb, 10, Some("com.example.app"), None)], &opts)
            .await
            .0
    }

    #[tokio::test]
    async fn expected_colour_passes_within_tolerance_and_fails_outside_it() {
        let result = result_for_colour([255, 0, 0]).await;
        let exact = FirstFrameExpectation {
            expected_color: Some("#FF0000".to_string()),
            tolerance: 0,
            ..FirstFrameExpectation::default()
        };
        let assertion = assert_first_frame(&result, &exact).expect("exact match");
        assert!(assertion.passed);
        assert_eq!(assertion.checked_color.unwrap().hex, "#FF0000");

        let tolerant = FirstFrameExpectation {
            expected_color: Some("#F01010".to_string()),
            tolerance: 16,
            ..FirstFrameExpectation::default()
        };
        assert!(assert_first_frame(&result, &tolerant).is_ok());

        let failed = FirstFrameExpectation {
            expected_color: Some("#00FF00".to_string()),
            tolerance: 0,
            ..FirstFrameExpectation::default()
        };
        let error = assert_first_frame(&result, &failed).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UiAssertionFailed);
        assert_eq!(error.body().context["actual"], json!("#FF0000"));
        assert_eq!(error.body().context["expected"], json!("#00FF00"));
        assert_eq!(error.body().context["tolerance"], json!(0));
        assert_eq!(error.body().context["max_channel_delta"], json!(255));
    }

    #[tokio::test]
    async fn no_splash_rejects_a_blank_first_frame() {
        let result = result_for_colour([255, 255, 255]).await;
        let expectation = FirstFrameExpectation {
            no_splash: true,
            ..FirstFrameExpectation::default()
        };
        let error = assert_first_frame(&result, &expectation).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UiAssertionFailed);
        assert_eq!(error.body().context["classification"], json!("white"));
        assert_eq!(error.body().context["actual"], json!("#FFFFFF"));

        let coloured = result_for_colour([20, 90, 160]).await;
        assert!(assert_first_frame(&coloured, &expectation).is_ok());
    }

    #[tokio::test]
    async fn artifact_bounds_limit_stored_frames_and_bytes() {
        let mut opts = options("com.example.app");
        opts.duration_ms = 300;
        let one = solid_png([1, 2, 3]);
        let size = one.len() as u64;
        opts.max_frames = 6;
        opts.max_frame_bytes = size + 10;
        opts.max_total_artifact_bytes = size * 2 + 5;
        let frames = (0..6)
            .map(|index| LaunchFrame {
                png: solid_png([index as u8, 2, 3]),
                elapsed_ms: index * 10,
                focus_package: Some("com.android.launcher".to_string()),
                ui_package: None,
            })
            .collect();
        let (result, sink) = run(frames, &opts).await;
        assert_eq!(result.status, LaunchCaptureStatus::ArtifactLimit);
        assert!(result.artifacts.len() <= opts.max_frames);
        assert!(sink.writes.len() <= opts.max_frames);
        let total = result
            .artifacts
            .iter()
            .map(|artifact| artifact.bytes)
            .sum::<u64>();
        assert!(total <= opts.max_total_artifact_bytes);
        assert_eq!(result.dropped_frames, 1);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("budget"))
        );
    }

    #[tokio::test]
    async fn frame_count_bound_stops_sampling() {
        let mut opts = options("com.example.app");
        opts.duration_ms = 500;
        opts.max_frames = 2;
        let frames = (0..6)
            .map(|_| frame([1, 2, 3], 0, Some("com.android.launcher"), None))
            .collect();
        let (result, sink) = run(frames, &opts).await;
        assert_eq!(result.status, LaunchCaptureStatus::FrameLimit);
        assert_eq!(result.artifacts.len(), 2);
        assert_eq!(sink.writes.len(), 2);
    }

    #[tokio::test]
    async fn oversize_frames_are_dropped_before_reaching_the_sink() {
        let mut opts = options("com.example.app");
        opts.duration_ms = 200;
        let size = solid_png([4, 5, 6]).len() as u64;
        opts.max_frame_bytes = size - 1;
        let frames = (0..3)
            .map(|_| frame([4, 5, 6], 0, Some("com.example.app"), None))
            .collect();
        let (result, sink) = run(frames, &opts).await;
        assert_eq!(result.frames_captured, 0);
        assert_eq!(sink.writes.len(), 0);
        assert!(result.dropped_frames >= 1);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("per-frame limit"))
        );
    }

    #[tokio::test]
    async fn cancellation_stops_the_sampling_loop() {
        let mut opts = options("com.example.app");
        opts.duration_ms = 500;
        let (result, _) = run_with(
            vec![frame([1, 2, 3], 0, Some("com.android.launcher"), None)],
            &opts,
            true,
        )
        .await;
        assert_eq!(result.status, LaunchCaptureStatus::Cancelled);
        assert_eq!(result.frames_captured, 1);
        assert!(
            result
                .no_frame_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("cancelled"))
        );
    }

    #[test]
    fn options_and_expectation_defaults_round_trip_from_empty_json() {
        let options: CaptureLaunchOptions = serde_json::from_str("{}").expect("options");
        assert_eq!(options.fps, DEFAULT_FPS);
        assert_eq!(options.duration_ms, DEFAULT_DURATION_MS);
        assert_eq!(options.package, "");
        assert_eq!(options.expected_color_sample, ColorSampleKind::Centre);
        let json = serde_json::to_value(&options).expect("serialize");
        assert_eq!(json["fps"], json!(DEFAULT_FPS));

        let expectation: FirstFrameExpectation = serde_json::from_str("{}").expect("expectation");
        assert!(!expectation.expect_first_frame);
        assert!(!expectation.no_splash);
        assert_eq!(expectation.tolerance, DEFAULT_TOLERANCE);
    }

    #[test]
    fn limits_clamp_out_of_range_requests() {
        let options = CaptureLaunchOptions {
            fps: 500,
            duration_ms: 500_000,
            frame_timeout_ms: 100_000,
            max_frames: 100_000,
            max_frame_bytes: u64::MAX,
            max_total_artifact_bytes: u64::MAX,
            ..CaptureLaunchOptions::default()
        };
        let limits = effective_limits(&options);
        assert_eq!(limits.fps, MAX_FPS);
        assert_eq!(limits.duration, Duration::from_millis(MAX_DURATION_MS));
        assert_eq!(
            limits.frame_timeout,
            Duration::from_millis(MAX_FRAME_TIMEOUT_MS)
        );
        assert_eq!(limits.max_frames, ABSOLUTE_MAX_FRAMES);
        assert_eq!(limits.max_frame_bytes, ABSOLUTE_MAX_FRAME_BYTES);
        assert_eq!(limits.max_total_bytes, ABSOLUTE_MAX_TOTAL_ARTIFACT_BYTES);
        assert!(limits.warnings.len() >= 6);
    }
}
