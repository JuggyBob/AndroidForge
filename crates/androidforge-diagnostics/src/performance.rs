//! Explicit, reproducible performance evidence.
//!
//! Values are reported only when the device output actually contains them.
//! Missing metrics are recorded as unknown with a reason; AndroidForge never
//! invents a startup time, frame count or CPU percentage. Cold measurement
//! requires an explicit request because it force-stops the application.

use std::time::{Duration, Instant};

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::util::{elapsed_ms, now_rfc3339, now_unix_ms};
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const PERFORMANCE_SCHEMA_VERSION: u32 = 1;

/// A jank percentage is only reported when at least this many frames were
/// rendered. One or two frames are not a sample; reporting 100% jank from a
/// single frame would be actively misleading.
pub const DEFAULT_MIN_FRAMES: u64 = 30;

/// Baseline 60 Hz frame budget. A device-provided refresh rate can replace
/// this in a future, additive change; it is never inferred from jank counts.
const FRAME_DEADLINE_MS: f64 = 16.67;

/// A `dumpsys cpuinfo` window that ended longer ago than this is stale and is
/// not evidence about the run being measured.
const CPU_STALE_WINDOW_MS: u64 = 2_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceCommand {
    pub name: String,
    pub command: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StartupMeasurement {
    pub requested: bool,
    pub cold: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_time_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_time_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub this_time_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JankMeasurement {
    pub reset_before_launch: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_frames: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub janky_frames: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub janky_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentile_50_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentile_90_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentile_95_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentile_99_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missed_vsync: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_reason: Option<String>,
    /// `ok`, `insufficient`, `unavailable` or `unknown`.
    #[serde(default)]
    pub sample_status: String,
    /// Whether this sample is suitable for cross-run comparison.
    #[serde(default)]
    pub comparable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison_reason: Option<String>,
    /// Sample window, in milliseconds relative to the start of collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_start_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_end_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_deadline_ms: Option<f64>,
}

impl Default for JankMeasurement {
    fn default() -> Self {
        Self {
            reset_before_launch: false,
            total_frames: None,
            janky_frames: None,
            janky_percent: None,
            percentile_50_ms: None,
            percentile_90_ms: None,
            percentile_95_ms: None,
            percentile_99_ms: None,
            missed_vsync: None,
            unknown_reason: None,
            sample_status: "unknown".to_string(),
            comparable: false,
            comparison_reason: None,
            window_start_ms: None,
            window_end_ms: None,
            frame_deadline_ms: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryMeasurement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_pss_kb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_rss_kb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_heap_pss_kb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_heap_pss_kb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuMeasurement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_reason: Option<String>,
    /// `ok`, `unavailable`, `stale` or `unknown`.
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Sample window as Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_start: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_end: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_pid: Option<u32>,
}

impl Default for CpuMeasurement {
    fn default() -> Self {
        Self {
            process: None,
            total_percent: None,
            user_percent: None,
            kernel_percent: None,
            unknown_reason: None,
            status: "unknown".to_string(),
            reason: None,
            window_start: None,
            window_end: None,
            sample_pid: None,
        }
    }
}

/// A single metric's usability, surfaced at the top level so agents do not
/// have to interpret individual null fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricQuality {
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Default for MetricQuality {
    fn default() -> Self {
        Self {
            status: "unknown".to_string(),
            reason: None,
        }
    }
}

/// Top-level sample quality verdict for the two numeric sample-based metrics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SampleQuality {
    #[serde(default)]
    pub jank: MetricQuality,
    #[serde(default)]
    pub cpu: MetricQuality,
}

/// Result of checking whether two samples can be compared. Kept here for the
/// cross-run comparison surface; re-exporting is an integration-workstream
/// change, so it is not referenced by the collection path yet.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleComparison {
    #[serde(default)]
    pub comparable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A `dumpsys cpuinfo` sample window in Unix milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuSampleWindow {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Preflight outcome. Waking is only ever an explicit opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenAction {
    Continue,
    Wake,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceReport {
    pub schema_version: u32,
    pub package: String,
    pub serial: String,
    pub collected_at: String,
    pub cold_start_requested: bool,
    pub settle_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_on: Option<bool>,
    pub foreground: Option<bool>,
    pub activity: Option<String>,
    pub startup: StartupMeasurement,
    pub jank: JankMeasurement,
    pub memory: MemoryMeasurement,
    pub cpu: CpuMeasurement,
    #[serde(default)]
    pub sample_quality: SampleQuality,
    pub commands: Vec<PerformanceCommand>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PerformanceOptions<'a> {
    pub package: &'a str,
    pub activity: Option<&'a str>,
    pub cold: bool,
    pub settle: Duration,
    pub timeout: Duration,
    /// Fail fast when the screen is off. Performance samples taken against a
    /// sleeping display are not meaningful.
    pub require_screen_on: bool,
    /// Explicit opt-in only; never wake implicitly.
    pub wake: bool,
    pub min_frames: u64,
    pub frame_window: Option<Duration>,
    pub warmup: Option<Duration>,
}

impl Default for PerformanceOptions<'_> {
    fn default() -> Self {
        Self {
            package: "",
            activity: None,
            cold: false,
            settle: Duration::ZERO,
            timeout: Duration::from_secs(30),
            require_screen_on: true,
            wake: false,
            min_frames: DEFAULT_MIN_FRAMES,
            frame_window: None,
            warmup: None,
        }
    }
}

pub async fn collect_performance(
    adb: &AdbClient,
    serial: &str,
    options: &PerformanceOptions<'_>,
) -> Result<PerformanceReport> {
    let package = options.package;
    let mut warnings = Vec::new();
    let mut commands = Vec::new();
    let run_started = Instant::now();
    let run_start_ms = now_unix_ms();

    // Screen preflight happens before any other device interaction. An
    // asleep display invalidates jank and startup evidence, and waking is
    // never a side effect of measurement unless explicitly requested.
    let mut screen_on = adb.screen_on(serial).await;
    match screen_preflight(screen_on, options.require_screen_on, options.wake) {
        Ok(ScreenAction::Continue) => {}
        Ok(ScreenAction::Wake) => match adb.wake(serial).await {
            Ok(result) => {
                screen_on = result.screen_on.or(screen_on);
                commands.push(PerformanceCommand {
                    name: "wake".to_string(),
                    command: "input keyevent KEYCODE_WAKEUP; wm dismiss-keyguard".to_string(),
                    status: "ok".to_string(),
                    excerpt: Some(format!("screen_on={:?}", screen_on)),
                    reason: None,
                });
                if screen_on == Some(false) {
                    return Err(screen_off_error(serial));
                }
            }
            Err(error) => {
                let message = error.message().to_string();
                return Err(screen_off_error(serial)
                    .with_cause(error)
                    .with_context_string("wake", message));
            }
        },
        Err(error) => return Err(error.device(serial.to_string())),
    }

    let install_path = shell(
        adb,
        serial,
        &["pm".to_string(), "path".to_string(), package.to_string()],
        options.timeout,
    )
    .await?;
    commands.push(command(
        "verify_install",
        &format!("pm path {package}"),
        &install_path,
        1200,
    ));
    if !install_path.text.contains("package:") {
        return Err(ForgeError::new(
            ErrorCode::PackageNotFound,
            format!("package {package} is not installed on {serial}"),
        )
        .with_suggestion("Install the application with `androidforge app install <apk>`.")
        .device(serial.to_string()));
    }

    let component = if let Some(activity) = options.activity {
        Some(normalize_component(package, activity))
    } else {
        let resolved = shell(
            adb,
            serial,
            &[
                "cmd".to_string(),
                "package".to_string(),
                "resolve-activity".to_string(),
                "--brief".to_string(),
                package.to_string(),
            ],
            options.timeout,
        )
        .await
        .ok();
        if let Some(resolved) = &resolved {
            commands.push(command(
                "resolve_activity",
                &format!("cmd package resolve-activity --brief {package}"),
                resolved,
                2000,
            ));
        }
        resolved.and_then(|output| {
            output.text.lines().rev().find_map(|line| {
                let line = line.trim();
                (line.contains('/') && line.contains('.')).then(|| line.to_string())
            })
        })
    };
    if component.is_none() {
        warnings.push(
            "could not resolve a launcher activity; startup timing will be unknown because monkey does not report it"
                .to_string(),
        );
    }

    if options.cold {
        let stopped = shell(
            adb,
            serial,
            &[
                "am".to_string(),
                "force-stop".to_string(),
                package.to_string(),
            ],
            options.timeout,
        )
        .await;
        match &stopped {
            Ok(output) => commands.push(command(
                "force_stop",
                &format!("am force-stop {package}"),
                output,
                1000,
            )),
            Err(error) => {
                warnings.push(format!("cold-start force-stop failed: {}", error.message()));
                commands.push(PerformanceCommand {
                    name: "force_stop".to_string(),
                    command: format!("am force-stop {package}"),
                    status: "failed".to_string(),
                    excerpt: None,
                    reason: Some(error.message().to_string()),
                });
            }
        }
        tokio::time::sleep(Duration::from_millis(750)).await;
    }

    let mut jank = JankMeasurement::default();
    let mut reset_ms: Option<u64> = None;
    match shell(
        adb,
        serial,
        &[
            "dumpsys".to_string(),
            "gfxinfo".to_string(),
            package.to_string(),
            "reset".to_string(),
        ],
        options.timeout,
    )
    .await
    {
        Ok(output) => {
            jank.reset_before_launch = true;
            reset_ms = Some(elapsed_ms(run_started));
            commands.push(command(
                "gfxinfo_reset",
                &format!("dumpsys gfxinfo {package} reset"),
                &output,
                1200,
            ));
        }
        Err(error) => {
            jank.reset_before_launch = false;
            warnings.push(format!(
                "could not reset gfxinfo counters; jank data covers an unknown window: {}",
                error.message()
            ));
            commands.push(PerformanceCommand {
                name: "gfxinfo_reset".to_string(),
                command: format!("dumpsys gfxinfo {package} reset"),
                status: "failed".to_string(),
                excerpt: None,
                reason: Some(error.message().to_string()),
            });
        }
    }

    let launch_argv = if let Some(component) = &component {
        vec![
            "am".to_string(),
            "start".to_string(),
            "-W".to_string(),
            "-n".to_string(),
            component.clone(),
        ]
    } else {
        vec![
            "monkey".to_string(),
            "-p".to_string(),
            package.to_string(),
            "-c".to_string(),
            "android.intent.category.LAUNCHER".to_string(),
            "1".to_string(),
        ]
    };
    let launch = shell(adb, serial, &launch_argv, options.timeout).await?;
    commands.push(command("startup", &launch_argv.join(" "), &launch, 4000));
    let mut startup = parse_am_start(&launch.text);
    startup.requested = true;
    startup.cold = options.cold;
    startup.activity = component.clone();
    if component.is_none() {
        startup.unknown_reason = Some("monkey launch does not report startup timing".to_string());
    }
    if let Some(reason) = &startup.unknown_reason {
        warnings.push(format!("startup timing is unknown: {reason}"));
    }

    if !options.settle.is_zero() {
        tokio::time::sleep(options.settle).await;
    }
    if let Some(warmup) = options.warmup
        && !warmup.is_zero()
    {
        tokio::time::sleep(warmup).await;
    }
    if let Some(frame_window) = options.frame_window
        && !frame_window.is_zero()
    {
        let remaining = frame_window.saturating_sub(run_started.elapsed());
        if !remaining.is_zero() {
            tokio::time::sleep(remaining).await;
        }
    }

    let foreground = adb.current_focus(serial).await;
    let foreground_matches = foreground
        .as_ref()
        .map(|focus| focus.package == package)
        .unwrap_or(false);
    if foreground.is_some() && !foreground_matches {
        warnings.push(format!(
            "the package is not the current focus (focus: {})",
            foreground
                .as_ref()
                .map(|focus| focus.package.as_str())
                .unwrap_or("unknown")
        ));
    }

    match shell(
        adb,
        serial,
        &[
            "dumpsys".to_string(),
            "gfxinfo".to_string(),
            package.to_string(),
        ],
        options.timeout,
    )
    .await
    {
        Ok(output) => {
            let parsed = parse_gfxinfo_with(&output.text, options.min_frames);
            let reset = jank.reset_before_launch;
            jank = parsed;
            jank.reset_before_launch = reset;
            finalize_jank_sample(
                &mut jank,
                reset_ms,
                Some(elapsed_ms(run_started)),
                options.frame_window,
            );
            commands.push(command(
                "gfxinfo",
                &format!("dumpsys gfxinfo {package}"),
                &output,
                2500,
            ));
            if let Some(reason) = &jank.unknown_reason {
                warnings.push(format!("jank metrics are unknown: {reason}"));
            }
            if jank.sample_status == "insufficient" {
                warnings.push(format!(
                    "jank sample is insufficient ({} frame(s)); at least {} are required",
                    jank.total_frames.unwrap_or(0),
                    options.min_frames
                ));
            }
        }
        Err(error) => {
            jank.unknown_reason = Some(error.message().to_string());
            jank.sample_status = "unavailable".to_string();
            finalize_jank_sample(
                &mut jank,
                reset_ms,
                Some(elapsed_ms(run_started)),
                options.frame_window,
            );
            warnings.push(format!("could not read gfxinfo: {}", error.message()));
            commands.push(PerformanceCommand {
                name: "gfxinfo".to_string(),
                command: format!("dumpsys gfxinfo {package}"),
                status: "failed".to_string(),
                excerpt: None,
                reason: Some(error.message().to_string()),
            });
        }
    }

    let memory = match shell(
        adb,
        serial,
        &[
            "dumpsys".to_string(),
            "meminfo".to_string(),
            package.to_string(),
        ],
        options.timeout,
    )
    .await
    {
        Ok(output) => {
            commands.push(command(
                "meminfo",
                &format!("dumpsys meminfo {package}"),
                &output,
                2500,
            ));
            let parsed = parse_meminfo(&output.text);
            if let Some(reason) = &parsed.unknown_reason {
                warnings.push(format!("memory metrics are unknown: {reason}"));
            }
            parsed
        }
        Err(error) => {
            warnings.push(format!("could not read meminfo: {}", error.message()));
            commands.push(PerformanceCommand {
                name: "meminfo".to_string(),
                command: format!("dumpsys meminfo {package}"),
                status: "failed".to_string(),
                excerpt: None,
                reason: Some(error.message().to_string()),
            });
            MemoryMeasurement {
                unknown_reason: Some(error.message().to_string()),
                ..MemoryMeasurement::default()
            }
        }
    };

    let cpuinfo = shell(
        adb,
        serial,
        &["dumpsys".to_string(), "cpuinfo".to_string()],
        options.timeout,
    )
    .await;
    let mut cpuinfo_text: Option<String> = None;
    match &cpuinfo {
        Ok(output) => {
            commands.push(command("cpuinfo", "dumpsys cpuinfo", output, 2500));
            cpuinfo_text = Some(output.text.clone());
        }
        Err(error) => {
            warnings.push(format!("could not read cpuinfo: {}", error.message()));
            commands.push(PerformanceCommand {
                name: "cpuinfo".to_string(),
                command: "dumpsys cpuinfo".to_string(),
                status: "failed".to_string(),
                excerpt: None,
                reason: Some(error.message().to_string()),
            });
        }
    }

    let mut cpu = CpuMeasurement::default();
    let mut proc_error: Option<String> = None;
    // Prefer PID-specific /proc deltas: they are sampled during this run and
    // cannot be confused with another process or a stale system-wide window.
    if let Some(pid) = memory.pid {
        match sample_proc_cpu(adb, serial, pid, options.timeout).await {
            Ok((sample, ticks, first, second)) => {
                cpu = sample;
                commands.push(PerformanceCommand {
                    name: "proc_cpu".to_string(),
                    command: format!("cat /proc/{pid}/stat (two samples)"),
                    status: "ok".to_string(),
                    excerpt: Some(format!(
                        "utime+stime samples {first} -> {second} over a 500 ms window; CLK_TCK={ticks}"
                    )),
                    reason: None,
                });
            }
            Err(error) => proc_error = Some(error.message().to_string()),
        }
    }

    if cpu.status != "ok" {
        let collected_ms = now_unix_ms();
        let window = cpuinfo_text
            .as_deref()
            .and_then(|text| parse_cpuinfo_window(text, collected_ms));
        cpu = if let Some(text) = cpuinfo_text.as_deref() {
            let mut parsed = validate_cpuinfo_sample(
                text,
                package,
                memory.pid,
                window,
                run_start_ms,
                collected_ms,
            );
            if let (Some(error), Some(reason)) = (proc_error.as_deref(), parsed.reason.as_mut()) {
                *reason = format!("{reason}; /proc sampling failed: {error}");
            }
            parsed
        } else {
            CpuMeasurement {
                status: "unavailable".to_string(),
                reason: Some("dumpsys cpuinfo was not readable".to_string()),
                ..CpuMeasurement::default()
            }
        };
        if let Some(reason) = cpu.reason.as_deref().or(cpu.unknown_reason.as_deref()) {
            warnings.push(format!("CPU metrics are {}: {reason}", cpu.status));
        }
    }

    let (sample_quality, quality_warning) = evaluate_sample_quality(&jank, &cpu);
    if let Some(warning) = quality_warning {
        warnings.push(warning);
    }
    Ok(PerformanceReport {
        schema_version: PERFORMANCE_SCHEMA_VERSION,
        package: package.to_string(),
        serial: serial.to_string(),
        collected_at: now_rfc3339(),
        cold_start_requested: options.cold,
        settle_ms: options.settle.as_millis() as u64,
        screen_on,
        foreground: foreground.map(|_| foreground_matches),
        activity: component,
        startup,
        jank,
        memory,
        cpu,
        sample_quality,
        commands,
        warnings,
    })
}

fn evaluate_sample_quality(
    jank: &JankMeasurement,
    cpu: &CpuMeasurement,
) -> (SampleQuality, Option<String>) {
    let quality = SampleQuality {
        jank: MetricQuality {
            status: jank.sample_status.clone(),
            reason: jank
                .comparison_reason
                .clone()
                .or_else(|| jank.unknown_reason.clone()),
        },
        cpu: MetricQuality {
            status: cpu.status.clone(),
            reason: cpu.reason.clone().or_else(|| cpu.unknown_reason.clone()),
        },
    };
    let warning = (quality.jank.status != "ok" && quality.cpu.status != "ok").then(|| {
        format!(
            "all performance samples are unusable (jank: {}, cpu: {}); do not draw conclusions from this report",
            quality.jank.status, quality.cpu.status
        )
    });
    (quality, warning)
}

/// Decide whether measurement may proceed against the current display state.
///
/// This is deliberately a pure function so the screen-off contract can be
/// tested without touching a device. `wake` is an explicit opt-in; the default
/// path never mutates device state.
pub fn screen_preflight(
    screen_on: Option<bool>,
    require_screen_on: bool,
    wake: bool,
) -> Result<ScreenAction> {
    if !require_screen_on {
        return Ok(ScreenAction::Continue);
    }
    if screen_on == Some(false) {
        return if wake {
            Ok(ScreenAction::Wake)
        } else {
            Err(ForgeError::new(
                ErrorCode::ScreenOff,
                "the device screen is off; performance evidence would not be valid",
            )
            .with_suggestion(
                "Wake the screen and retry the measurement (wake is opt-in, never implicit).",
            ))
        };
    }
    Ok(ScreenAction::Continue)
}

fn screen_off_error(serial: &str) -> ForgeError {
    ForgeError::new(
        ErrorCode::ScreenOff,
        format!("the screen is still off on {serial}; performance measurement was not started"),
    )
    .with_suggestion("Wake the device screen manually and retry the measurement.")
    .device(serial.to_string())
}

pub fn parse_am_start(text: &str) -> StartupMeasurement {
    let mut measurement = StartupMeasurement {
        requested: true,
        status: value_after(text, "Status:"),
        total_time_ms: millis_after(text, "TotalTime:"),
        wait_time_ms: millis_after(text, "WaitTime:"),
        this_time_ms: millis_after(text, "ThisTime:"),
        raw_output: Some(excerpt(text, 4000)),
        ..StartupMeasurement::default()
    };
    if measurement.total_time_ms.is_none() {
        measurement.unknown_reason =
            Some("am start output did not include a TotalTime value".to_string());
    }
    measurement
}
pub fn parse_gfxinfo(text: &str) -> JankMeasurement {
    parse_gfxinfo_with(text, DEFAULT_MIN_FRAMES)
}

/// Parse gfxinfo and apply the sample-quality floor. A small sample never
/// yields a jank percentage, even when gfxinfo printed one.
pub fn parse_gfxinfo_with(text: &str, min_frames: u64) -> JankMeasurement {
    let total_frames = capture_u64(r"(?m)^\s*Total frames rendered:\s*(\d+)", text);
    let (janky_frames, janky_percent) = if let Some(caps) =
        Regex::new(r"(?m)^\s*Janky frames:\s*(\d+)\s*\(([\d.]+)%\)")
            .expect("jank regex")
            .captures(text)
    {
        (
            caps.get(1).and_then(|value| value.as_str().parse().ok()),
            caps.get(2).and_then(|value| value.as_str().parse().ok()),
        )
    } else {
        (None, None)
    };
    let mut measurement = JankMeasurement {
        total_frames,
        janky_frames,
        janky_percent,
        percentile_50_ms: percentile(text, 50),
        percentile_90_ms: percentile(text, 90),
        percentile_95_ms: percentile(text, 95),
        percentile_99_ms: percentile(text, 99),
        missed_vsync: capture_u64(r"(?m)^\s*Number Missed Vsync:\s*(\d+)", text),
        ..JankMeasurement::default()
    };
    match measurement.total_frames {
        None => {
            measurement.sample_status = "unavailable".to_string();
            measurement.unknown_reason = Some(if text.contains("No process") {
                "gfxinfo reported no process for the package".to_string()
            } else if text.trim().is_empty() {
                "gfxinfo produced no output".to_string()
            } else {
                "gfxinfo did not report frame statistics".to_string()
            });
            measurement.comparison_reason = measurement.unknown_reason.clone();
        }
        Some(frames) if frames < min_frames => {
            measurement.sample_status = "insufficient".to_string();
            measurement.janky_percent = None;
            measurement.comparison_reason = Some(format!(
                "insufficient sample ({frames} frame(s)); at least {min_frames} are required"
            ));
        }
        _ => measurement.sample_status = "ok".to_string(),
    }
    measurement
}

/// Attach the actual observation window and compute cross-run comparability.
pub fn finalize_jank_sample(
    jank: &mut JankMeasurement,
    window_start_ms: Option<u64>,
    window_end_ms: Option<u64>,
    frame_window: Option<Duration>,
) {
    jank.window_start_ms = match (window_end_ms, frame_window) {
        (Some(end), Some(window)) if !window.is_zero() => {
            Some(end.saturating_sub(window.as_millis() as u64))
        }
        _ => window_start_ms,
    };
    jank.window_end_ms = window_end_ms;
    jank.frame_deadline_ms = Some(FRAME_DEADLINE_MS);
    jank.comparable = jank.sample_status == "ok"
        && jank.janky_percent.is_some()
        && jank.window_start_ms.is_some()
        && jank.window_end_ms.is_some();
    if !jank.comparable && jank.comparison_reason.is_none() {
        jank.comparison_reason = Some(format!(
            "jank sample is not comparable (status: {})",
            jank.sample_status
        ));
    }
}

/// Two jank samples are comparable only when both are usable and describe the
/// same amount of work under the same frame budget and window length.
#[allow(dead_code)]
pub fn compare_jank_samples(first: &JankMeasurement, second: &JankMeasurement) -> SampleComparison {
    let mut comparison = SampleComparison {
        comparable: false,
        reason: None,
    };
    for sample in [first, second] {
        if sample.sample_status != "ok" || !sample.comparable {
            comparison.reason = Some(format!(
                "sample is not comparable (status: {})",
                sample.sample_status
            ));
            return comparison;
        }
    }
    if first.total_frames != second.total_frames {
        comparison.reason = Some(format!(
            "frame counts differ ({} vs {} frame(s))",
            first.total_frames.unwrap_or(0),
            second.total_frames.unwrap_or(0)
        ));
        return comparison;
    }
    if first.frame_deadline_ms != second.frame_deadline_ms {
        comparison.reason = Some("frame deadlines differ".to_string());
        return comparison;
    }
    if let (Some(first_start), Some(first_end), Some(second_start), Some(second_end)) = (
        first.window_start_ms,
        first.window_end_ms,
        second.window_start_ms,
        second.window_end_ms,
    ) {
        let first_window = first_end.saturating_sub(first_start);
        let second_window = second_end.saturating_sub(second_start);
        if first_window.abs_diff(second_window) > 1 {
            comparison.reason = Some(format!(
                "sample windows differ ({first_window}ms vs {second_window}ms)"
            ));
            return comparison;
        }
    }
    comparison.comparable = true;
    comparison
}
pub fn parse_meminfo(text: &str) -> MemoryMeasurement {
    let mut measurement = MemoryMeasurement {
        pid: Regex::new(r"\*\* MEMINFO in pid (\d+)")
            .expect("meminfo pid regex")
            .captures(text)
            .and_then(|captures| captures.get(1))
            .and_then(|value| value.as_str().parse().ok()),
        total_pss_kb: capture_u64(r"TOTAL PSS:\s*(\d+)", text),
        total_rss_kb: capture_u64(r"TOTAL RSS:\s*(\d+)", text),
        java_heap_pss_kb: capture_u64(r"(?m)^\s*Java Heap:\s*(\d+)", text),
        native_heap_pss_kb: capture_u64(r"(?m)^\s*Native Heap:\s*(\d+)", text),
        unknown_reason: None,
    };
    if measurement.total_pss_kb.is_none() && measurement.total_rss_kb.is_none() {
        measurement.unknown_reason = Some(if text.contains("No process") {
            "meminfo reported no process for the package".to_string()
        } else if text.trim().is_empty() {
            "meminfo produced no output".to_string()
        } else {
            "meminfo did not report TOTAL PSS or TOTAL RSS".to_string()
        });
    }
    measurement
}
pub fn parse_cpuinfo(text: &str, package: &str) -> CpuMeasurement {
    parse_cpuinfo_candidate(text, package, None).unwrap_or_else(|| CpuMeasurement {
        status: "unavailable".to_string(),
        reason: Some("the package process was not present in dumpsys cpuinfo".to_string()),
        unknown_reason: Some("the package process was not present in dumpsys cpuinfo".to_string()),
        ..CpuMeasurement::default()
    })
}

/// Convert the relative `CPU usage from Xms to Yms ago` header into an
/// absolute sample window. Without this header a cpuinfo line has no evidence
/// that it covers the run being measured.
pub fn parse_cpuinfo_window(text: &str, collected_ms: u64) -> Option<CpuSampleWindow> {
    let caps = Regex::new(r"(?m)^\s*CPU usage from (\d+)ms to (\d+)ms ago:")
        .expect("cpuinfo window regex")
        .captures(text)?;
    let from = caps.get(1)?.as_str().parse::<u64>().ok()?;
    let to = caps.get(2)?.as_str().parse::<u64>().ok()?;
    if to > from {
        return None;
    }
    Some(CpuSampleWindow {
        start_ms: collected_ms.saturating_sub(from),
        end_ms: collected_ms.saturating_sub(to),
    })
}

/// Accept a cpuinfo line only when its PID matches the measured process, its
/// window overlaps the run, and it is not stale.
pub fn validate_cpuinfo_sample(
    text: &str,
    package: &str,
    expected_pid: Option<u32>,
    window: Option<CpuSampleWindow>,
    run_start_ms: u64,
    collected_ms: u64,
) -> CpuMeasurement {
    let Some(window) = window else {
        let reason = "dumpsys cpuinfo did not report a sample window".to_string();
        return CpuMeasurement {
            status: "unavailable".to_string(),
            reason: Some(reason.clone()),
            unknown_reason: Some(reason),
            ..CpuMeasurement::default()
        };
    };
    if window.end_ms < window.start_ms
        || collected_ms.saturating_sub(window.end_ms) > CPU_STALE_WINDOW_MS
    {
        let reason = format!(
            "dumpsys cpuinfo window is stale (ended {}ms before collection)",
            collected_ms.saturating_sub(window.end_ms)
        );
        return CpuMeasurement {
            status: "stale".to_string(),
            reason: Some(reason.clone()),
            unknown_reason: Some(reason),
            ..CpuMeasurement::default()
        };
    }
    if window.end_ms < run_start_ms || window.start_ms > collected_ms {
        let reason = "dumpsys cpuinfo window does not overlap this run".to_string();
        return CpuMeasurement {
            status: "stale".to_string(),
            reason: Some(reason.clone()),
            unknown_reason: Some(reason),
            ..CpuMeasurement::default()
        };
    }
    let Some(expected_pid) = expected_pid else {
        let reason = "memory PID is unavailable; cpuinfo PID cannot be validated".to_string();
        return CpuMeasurement {
            status: "unavailable".to_string(),
            reason: Some(reason.clone()),
            unknown_reason: Some(reason),
            ..CpuMeasurement::default()
        };
    };
    let Some(mut parsed) = parse_cpuinfo_candidate(text, package, Some(expected_pid)) else {
        let reason = format!("PID {expected_pid} for {package} was not present in dumpsys cpuinfo");
        return CpuMeasurement {
            status: "unavailable".to_string(),
            reason: Some(reason.clone()),
            unknown_reason: Some(reason),
            ..CpuMeasurement::default()
        };
    };
    parsed.window_start = Some(window.start_ms);
    parsed.window_end = Some(window.end_ms);
    parsed
}

fn parse_cpuinfo_candidate(
    text: &str,
    package: &str,
    expected_pid: Option<u32>,
) -> Option<CpuMeasurement> {
    let regex = Regex::new(
        r"(?m)^\s*([\d.]+)%\s+(\d+)/(\S+):\s+([\d.]+)%\s+user\s+\+\s+([\d.]+)%\s+kernel",
    )
    .expect("cpuinfo regex");
    let mut exact = None;
    let mut prefixed: Option<(f64, CpuMeasurement)> = None;
    for caps in regex.captures_iter(text) {
        let Some(process) = caps.get(3).map(|value| value.as_str().to_string()) else {
            continue;
        };
        let Some(pid) = caps
            .get(2)
            .and_then(|value| value.as_str().parse::<u32>().ok())
        else {
            continue;
        };
        if expected_pid.is_some_and(|expected| expected != pid) {
            continue;
        }
        let total = caps
            .get(1)
            .and_then(|value| value.as_str().parse::<f64>().ok());
        let user = caps
            .get(4)
            .and_then(|value| value.as_str().parse::<f64>().ok());
        let kernel = caps
            .get(5)
            .and_then(|value| value.as_str().parse::<f64>().ok());
        let candidate = CpuMeasurement {
            process: Some(process.clone()),
            total_percent: total,
            user_percent: user,
            kernel_percent: kernel,
            unknown_reason: None,
            status: "ok".to_string(),
            reason: None,
            sample_pid: Some(pid),
            ..CpuMeasurement::default()
        };
        if process == package {
            exact = Some(candidate);
            break;
        }
        if process.starts_with(&format!("{package}:")) {
            let rank = total.unwrap_or(0.0);
            if prefixed.as_ref().is_none_or(|(best, _)| rank > *best) {
                prefixed = Some((rank, candidate));
            }
        }
    }
    exact.or_else(|| prefixed.map(|(_, candidate)| candidate))
}

async fn sample_proc_cpu(
    adb: &AdbClient,
    serial: &str,
    pid: u32,
    timeout: Duration,
) -> Result<(CpuMeasurement, f64, u64, u64)> {
    let ticks = shell(
        adb,
        serial,
        &["getconf".to_string(), "CLK_TCK".to_string()],
        timeout,
    )
    .await
    .ok()
    .and_then(|output| output.text.trim().parse::<f64>().ok())
    .filter(|ticks| *ticks > 0.0)
    .unwrap_or(100.0);
    let first = proc_cpu_ticks(adb, serial, pid, timeout).await?;
    let window_start = now_unix_ms();
    let started = Instant::now();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let second = proc_cpu_ticks(adb, serial, pid, timeout).await?;
    let window_end = now_unix_ms();
    let elapsed = started.elapsed().as_secs_f64().max(0.001);
    let user = (second.0.saturating_sub(first.0)) as f64 / ticks / elapsed * 100.0;
    let kernel = (second.1.saturating_sub(first.1)) as f64 / ticks / elapsed * 100.0;
    Ok((
        CpuMeasurement {
            process: Some(format!("pid {pid}")),
            total_percent: Some(user + kernel),
            user_percent: Some(user),
            kernel_percent: Some(kernel),
            unknown_reason: None,
            status: "ok".to_string(),
            reason: None,
            window_start: Some(window_start),
            window_end: Some(window_end),
            sample_pid: Some(pid),
        },
        ticks,
        first.0.saturating_add(first.1),
        second.0.saturating_add(second.1),
    ))
}

async fn proc_cpu_ticks(
    adb: &AdbClient,
    serial: &str,
    pid: u32,
    timeout: Duration,
) -> Result<(u64, u64)> {
    let output = shell(
        adb,
        serial,
        &["cat".to_string(), format!("/proc/{pid}/stat")],
        timeout,
    )
    .await?;
    parse_proc_stat(&output.text).ok_or_else(|| {
        ForgeError::new(
            ErrorCode::CommandFailed,
            format!("could not parse /proc/{pid}/stat"),
        )
        .with_context_string("output", excerpt(&output.text, 1200))
        .device(serial.to_string())
    })
}

fn parse_proc_stat(text: &str) -> Option<(u64, u64)> {
    let rest = text.rsplit_once(')')?.1.trim();
    let fields = rest.split_whitespace().collect::<Vec<_>>();
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    Some((utime, stime))
}
fn normalize_component(package: &str, activity: &str) -> String {
    if activity.contains('/') {
        activity.to_string()
    } else {
        format!("{package}/{activity}")
    }
}

struct ShellOutput {
    text: String,
}

async fn shell(
    adb: &AdbClient,
    serial: &str,
    args: &[String],
    timeout: Duration,
) -> Result<ShellOutput> {
    let output = adb
        .shell_args(serial, args, Some(timeout), Some(4 * 1024 * 1024))
        .await?;
    Ok(ShellOutput {
        text: output.combined_text(),
    })
}

fn command(
    name: &str,
    command_text: &str,
    output: &ShellOutput,
    excerpt_limit: usize,
) -> PerformanceCommand {
    PerformanceCommand {
        name: name.to_string(),
        command: command_text.to_string(),
        status: "ok".to_string(),
        excerpt: Some(excerpt(&output.text, excerpt_limit)),
        reason: None,
    }
}

fn value_after(text: &str, prefix: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix(prefix)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn millis_after(text: &str, prefix: &str) -> Option<u64> {
    value_after(text, prefix)?
        .trim_end_matches("ms")
        .trim()
        .parse()
        .ok()
}

fn percentile(text: &str, percentile: u32) -> Option<f64> {
    let pattern = format!(
        r"(?m)^\s*{}(?:st|nd|rd|th) percentile:\s*([\d.]+)\s*ms",
        percentile
    );
    Regex::new(&pattern)
        .ok()?
        .captures(text)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

fn capture_u64(pattern: &str, text: &str) -> Option<u64> {
    Regex::new(pattern)
        .ok()?
        .captures(text)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

fn excerpt(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let mut result = trimmed.chars().take(limit).collect::<String>();
    result.push_str("\n[truncated]");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const AM_START: &str = "\
Starting: Intent { cmp=com.example/.MainActivity }
Status: ok
LaunchState: COLD
Activity: com.example/.MainActivity
TotalTime: 842
WaitTime: 855
ThisTime: 410
";

    const GFXINFO: &str = "\
Applications Graphics Acceleration Info:
Uptime: 123456 Realtime: 123456
** Graphics info for pid 1234 [com.example] **
Total frames rendered: 120
Janky frames: 9 (7.50%)
50th percentile: 8ms
90th percentile: 16ms
95th percentile: 24ms
99th percentile: 40ms
Number Missed Vsync: 3
";

    const MEMINFO: &str = "\
** MEMINFO in pid 1234 [com.example] **
                   Pss  Private  Private  SwapPss      Rss
         Java Heap:   12345    10000        0        0    20000
       Native Heap:   23456    20000        0        0    30000
           TOTAL:  456789   400000        0        0   500000
 TOTAL PSS:   456789            TOTAL RSS:   500000       TOTAL SWAP PSS:    123
";

    const CPUINFO: &str = "\
Load: 1.2 / 1.1 / 1.0
CPU usage from 1000ms to 0ms ago:
  12.5% 1234/com.example: 8.0% user + 4.5% kernel + 0.0% iowait
   2.0% 5678/com.example:remote: 1.0% user + 1.0% kernel
   1.0% 999/system_server: 0.5% user + 0.5% kernel
";

    const GFXINFO_ONE_FRAME: &str = "\
Applications Graphics Acceleration Info:
Uptime: 123456 Realtime: 123456
** Graphics info for pid 1234 [com.example] **
Total frames rendered: 1
Janky frames: 1 (100.00%)
50th percentile: 16ms
90th percentile: 16ms
Number Missed Vsync: 1
";

    const GFXINFO_SIXTY_FRAMES: &str = "\
Applications Graphics Acceleration Info:
Uptime: 123456 Realtime: 123456
** Graphics info for pid 1234 [com.example] **
Total frames rendered: 60
Janky frames: 6 (10.00%)
50th percentile: 8ms
90th percentile: 16ms
Number Missed Vsync: 2
";

    const CPUINFO_STALE: &str = "\
Load: 1.2 / 1.1 / 1.0
CPU usage from 9000ms to 8000ms ago:
  12.5% 1234/com.example: 8.0% user + 4.5% kernel + 0.0% iowait
";

    #[test]
    fn parses_am_start_timings() {
        let startup = parse_am_start(AM_START);
        assert_eq!(startup.status.as_deref(), Some("ok"));
        assert_eq!(startup.total_time_ms, Some(842));
        assert_eq!(startup.wait_time_ms, Some(855));
        assert_eq!(startup.this_time_ms, Some(410));
        assert!(startup.unknown_reason.is_none());
    }

    #[test]
    fn missing_startup_time_is_explicitly_unknown() {
        let startup = parse_am_start("Status: ok\n");
        assert_eq!(startup.total_time_ms, None);
        assert!(
            startup
                .unknown_reason
                .as_deref()
                .unwrap()
                .contains("TotalTime")
        );
    }

    #[test]
    fn parses_gfxinfo_frames_and_percentiles() {
        let jank = parse_gfxinfo(GFXINFO);
        assert_eq!(jank.total_frames, Some(120));
        assert_eq!(jank.janky_frames, Some(9));
        assert_eq!(jank.janky_percent, Some(7.5));
        assert_eq!(jank.percentile_90_ms, Some(16.0));
        assert_eq!(jank.percentile_99_ms, Some(40.0));
        assert_eq!(jank.missed_vsync, Some(3));
        assert!(jank.unknown_reason.is_none());
    }

    #[test]
    fn empty_gfxinfo_is_unknown() {
        let jank = parse_gfxinfo("No process found for com.example\n");
        assert_eq!(jank.total_frames, None);
        assert!(jank.unknown_reason.is_some());
    }

    #[test]
    fn parses_meminfo_totals_and_heaps() {
        let memory = parse_meminfo(MEMINFO);
        assert_eq!(memory.pid, Some(1234));
        assert_eq!(memory.total_pss_kb, Some(456789));
        assert_eq!(memory.total_rss_kb, Some(500000));
        assert_eq!(memory.java_heap_pss_kb, Some(12345));
        assert_eq!(memory.native_heap_pss_kb, Some(23456));
        assert!(memory.unknown_reason.is_none());
    }

    #[test]
    fn parses_exact_and_remote_cpu_processes() {
        let cpu = parse_cpuinfo(CPUINFO, "com.example");
        assert_eq!(cpu.process.as_deref(), Some("com.example"));
        assert_eq!(cpu.total_percent, Some(12.5));
        assert_eq!(cpu.user_percent, Some(8.0));
        assert_eq!(cpu.kernel_percent, Some(4.5));
    }

    #[test]
    fn parses_proc_stat_cpu_ticks() {
        let line =
            "1234 (com.example app) S 1 1234 0 0 -1 4194560 100 0 0 0 42 17 0 0 20 0 12 0 100 0 0";
        assert_eq!(parse_proc_stat(line), Some((42, 17)));
        assert_eq!(parse_proc_stat("not a stat line"), None);
    }

    #[test]
    fn unknown_cpu_process_is_not_guessed() {
        let cpu = parse_cpuinfo(CPUINFO, "com.other");
        assert_eq!(cpu.total_percent, None);
        assert!(cpu.unknown_reason.is_some());
    }

    // Fixture 1: one frame must never be reported as 100% jank.
    #[test]
    fn one_frame_gfxinfo_is_insufficient_and_percent_is_null() {
        let jank = parse_gfxinfo_with(GFXINFO_ONE_FRAME, DEFAULT_MIN_FRAMES);
        assert_eq!(jank.total_frames, Some(1));
        assert_eq!(jank.janky_frames, Some(1));
        assert_eq!(jank.sample_status, "insufficient");
        assert_eq!(jank.janky_percent, None);
        assert!(!jank.comparable);
        assert_eq!(
            jank.comparison_reason.as_deref(),
            Some("insufficient sample (1 frame(s)); at least 30 are required")
        );
    }

    // Fixture 2a: a valid window with a different PID is not our sample.
    #[test]
    fn cpuinfo_with_absent_pid_is_unavailable_and_percent_is_null() {
        let collected_ms = 5_000;
        let window = parse_cpuinfo_window(CPUINFO, collected_ms).expect("window");
        let cpu = validate_cpuinfo_sample(
            CPUINFO,
            "com.example",
            Some(9999),
            Some(window),
            0,
            collected_ms,
        );
        assert_eq!(cpu.status, "unavailable");
        assert_eq!(cpu.total_percent, None);
        assert!(cpu.reason.as_deref().unwrap().contains("9999"));
    }

    // Fixture 2b: a window that ended long before collection is stale.
    #[test]
    fn stale_cpuinfo_window_is_rejected_and_percent_is_null() {
        let collected_ms = 10_000;
        let window = parse_cpuinfo_window(CPUINFO_STALE, collected_ms).expect("window");
        let cpu = validate_cpuinfo_sample(
            CPUINFO_STALE,
            "com.example",
            Some(1234),
            Some(window),
            0,
            collected_ms,
        );
        assert_eq!(cpu.status, "stale");
        assert_eq!(cpu.total_percent, None);
        assert!(cpu.reason.as_deref().unwrap().contains("stale"));
    }

    // Fixture 3: screen-off is a hard, unit-level preflight failure; no device
    // access is needed to verify the contract.
    #[test]
    fn screen_off_requires_explicit_wake_and_returns_screen_off() {
        let error = screen_preflight(Some(false), true, false).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ScreenOff);
        assert_eq!(error.exit_code(), 4);
        assert!(
            error
                .body()
                .suggestions
                .iter()
                .any(|suggestion| suggestion.to_ascii_lowercase().contains("wake"))
        );
        assert_eq!(
            screen_preflight(Some(false), true, true).unwrap(),
            ScreenAction::Wake
        );
        assert_eq!(
            screen_preflight(Some(true), true, false).unwrap(),
            ScreenAction::Continue
        );
    }

    // Fixture 4: different frame counts describe different workloads.
    #[test]
    fn runs_with_different_frame_counts_are_not_comparable() {
        let mut first = parse_gfxinfo_with(GFXINFO, DEFAULT_MIN_FRAMES);
        let mut second = parse_gfxinfo_with(GFXINFO_SIXTY_FRAMES, DEFAULT_MIN_FRAMES);
        finalize_jank_sample(&mut first, Some(100), Some(1_100), None);
        finalize_jank_sample(&mut second, Some(100), Some(1_100), None);
        let comparison = compare_jank_samples(&first, &second);
        assert!(!comparison.comparable);
        assert!(
            comparison
                .reason
                .as_deref()
                .unwrap()
                .contains("frame counts differ")
        );
        assert!(compare_jank_samples(&first, &first).comparable);
    }

    #[test]
    fn defaults_require_screen_on_and_thirty_frames_without_waking() {
        let options = PerformanceOptions::default();
        assert!(options.require_screen_on);
        assert!(!options.wake);
        assert_eq!(options.min_frames, DEFAULT_MIN_FRAMES);
        assert!(options.frame_window.is_none());
        assert!(options.warmup.is_none());
    }

    #[test]
    fn all_unusable_samples_raise_a_quality_warning() {
        let jank = parse_gfxinfo("No process found for com.example\n");
        let cpu = validate_cpuinfo_sample(CPUINFO, "com.example", Some(9999), None, 0, 1_000);
        let (quality, warning) = evaluate_sample_quality(&jank, &cpu);
        assert_eq!(quality.jank.status, "unavailable");
        assert_eq!(quality.cpu.status, "unavailable");
        assert!(
            warning
                .unwrap()
                .contains("all performance samples are unusable")
        );
    }
}
