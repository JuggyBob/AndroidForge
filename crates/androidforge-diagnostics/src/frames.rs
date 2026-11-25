//! Frame-timing evidence from `dumpsys gfxinfo`.
//!
//! This is an additive, focused parser for the `diagnostics frames` data layer.
//! It does not change `diagnostics performance`: the existing jank parser and
//! its output stay exactly as they are. Numeric values are `None` when the
//! device did not print them; missing metrics are never reported as zero.
//!
//! The gfxinfo dialect varies by Android version. Counters such as
//! `Janky frames legacy`, `Number Slow UI thread` and
//! `Number Frame deadline missed` are captured when present, while older
//! builds simply leave them unknown.

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::performance::DEFAULT_MIN_FRAMES;

/// Schema version for frame reports. Additive fields keep this at 1.
pub const FRAMES_SCHEMA_VERSION: u32 = 1;

/// Maximum number of histogram buckets retained from device output.
const MAX_HISTOGRAM_BUCKETS: usize = 128;

/// Bound on the verbatim header excerpt stored as raw window metadata.
const RAW_HEADER_MAX_CHARS: usize = 500;

/// Small tolerance so floating point arithmetic does not turn equal values
/// into regressions.
const DELTA_EPSILON: f64 = 1e-9;

fn default_frames_schema_version() -> u32 {
    FRAMES_SCHEMA_VERSION
}

/// One `Histogram:` row, for example `16-32ms: 10 frames (8.33%)`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FramesHistogramBucket {
    pub start_ms: f64,
    pub end_ms: f64,
    pub count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
}

/// Verbatim window metadata printed by gfxinfo before the frame counters.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FramesWindow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uptime_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realtime_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats_since_ns: Option<u64>,
    /// Bounded, verbatim header lines. Kept so a future parser can recover
    /// fields this version does not recognise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_excerpt: Option<String>,
}

/// Parsed frame sample. Every metric is optional because gfxinfo prints
/// different counters on different Android versions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FramesSample {
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "frames")]
    pub total_frames: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "janky_frames"
    )]
    pub janky: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub janky_percent: Option<f64>,
    /// Modern `Janky frames legacy` counter, when the device prints it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "janky_legacy_frames"
    )]
    pub janky_legacy: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub janky_legacy_percent: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "percentile_50_ms"
    )]
    pub p50_ms: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "percentile_90_ms"
    )]
    pub p90_ms: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "percentile_95_ms"
    )]
    pub p95_ms: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "percentile_99_ms"
    )]
    pub p99_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missed_vsync: Option<u64>,
    /// `Number Slow UI thread`, when present.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "slow_ui_frames"
    )]
    pub slow_ui: Option<u64>,
    /// `Number Frame deadline missed`, when present.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "deadline_missed_frames"
    )]
    pub deadline_missed: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub histogram: Vec<FramesHistogramBucket>,
    #[serde(default)]
    pub window: FramesWindow,
    /// `ok`, `insufficient`, `unavailable` or `unknown`.
    #[serde(default)]
    pub sample_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_reason: Option<String>,
}

impl FramesSample {
    /// Reported jank percentage, falling back to a computed one only for a
    /// sample that clears the same quality floor as `diagnostics performance`.
    pub fn jank_percent(&self) -> Option<f64> {
        if let Some(percent) = self.janky_percent {
            return Some(percent);
        }
        match (self.janky, self.total_frames) {
            (Some(janky), Some(total)) if total >= DEFAULT_MIN_FRAMES => {
                Some(janky as f64 * 100.0 / total as f64)
            }
            _ => None,
        }
    }

    fn status(&self) -> &str {
        if self.sample_status.is_empty() {
            "unknown"
        } else {
            &self.sample_status
        }
    }
}

/// Verdict for a single baseline metric comparison.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FramesDeltaStatus {
    Pass,
    Regression,
    #[default]
    Unknown,
}

/// A structured delta for one metric. `delta` is `current - baseline`:
/// positive is slower/jankier, negative is an improvement.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FramesMetricDelta {
    pub metric: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<f64>,
    pub status: FramesDeltaStatus,
}

/// Allowed increases before a metric is a regression. Defaults are strict:
/// any increase counts until a CLI flag supplies explicit tolerances.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FramesThresholds {
    /// Allowed increase in jank percentage points.
    pub jank_percent_pp: f64,
    /// Allowed increase in the 99th percentile frame time, in ms.
    pub p99_ms: f64,
    /// Allowed increase in the longest frame, in ms.
    pub max_ms: f64,
}

impl Default for FramesThresholds {
    fn default() -> Self {
        Self {
            jank_percent_pp: 0.0,
            p99_ms: 0.0,
            max_ms: 0.0,
        }
    }
}

/// Result of comparing a baseline frame report with a current one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FramesComparison {
    pub comparable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deltas: Vec<FramesMetricDelta>,
    /// True only when at least one comparable metric regressed.
    pub regression: bool,
}

/// Complete frame report written by the `diagnostics frames` command.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FramesReport {
    #[serde(default = "default_frames_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub package: String,
    #[serde(default)]
    pub serial: String,
    #[serde(default)]
    pub collected_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_dir: Option<String>,
    #[serde(default)]
    pub reset_before_collection: bool,
    #[serde(default)]
    pub sample: FramesSample,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<FramesComparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Parse gfxinfo output with the standard quality floor (see
/// [`DEFAULT_MIN_FRAMES`]). A small sample never yields a jank percentage.
pub fn parse_frames_gfxinfo(text: &str) -> FramesSample {
    parse_frames_gfxinfo_with(text, DEFAULT_MIN_FRAMES)
}

/// Parse gfxinfo output and apply an explicit sample-quality floor.
pub fn parse_frames_gfxinfo_with(text: &str, min_frames: u64) -> FramesSample {
    let total_frames = capture_u64(r"(?im)^\s*Total frames rendered:\s*(\d+)", text);
    let (janky, janky_percent) = jank_counter(text, "Janky frames");
    let (janky_legacy, janky_legacy_percent) = jank_counter(text, "Janky frames legacy");
    let (missed_vsync, slow_ui, deadline_missed) = (
        capture_u64(r"(?im)^\s*Number Missed Vsync:\s*(\d+)", text),
        capture_u64(r"(?im)^\s*Number Slow UI thread:\s*(\d+)", text),
        capture_u64(r"(?im)^\s*Number Frame deadline missed:\s*(\d+)", text),
    );
    let hist = parse_frames_histogram(text);
    let mut sample = FramesSample {
        total_frames,
        janky,
        janky_percent,
        janky_legacy,
        janky_legacy_percent,
        p50_ms: frame_percentile(text, 50),
        p90_ms: frame_percentile(text, 90),
        p95_ms: frame_percentile(text, 95),
        p99_ms: frame_percentile(text, 99),
        max_ms: frame_max_ms(text),
        missed_vsync,
        slow_ui,
        deadline_missed,
        histogram: hist,
        window: parse_frames_window(text),
        ..FramesSample::default()
    };
    match sample.total_frames {
        None => {
            sample.sample_status = "unavailable".to_string();
            sample.unknown_reason = Some(if text.contains("No process") {
                "gfxinfo reported no process for the package".to_string()
            } else if text.trim().is_empty() {
                "gfxinfo produced no output".to_string()
            } else {
                "gfxinfo did not report frame statistics".to_string()
            });
        }
        Some(frames) if frames < min_frames => {
            sample.sample_status = "insufficient".to_string();
            sample.janky_percent = None;
            sample.unknown_reason = Some(format!(
                "insufficient sample ({frames} frame(s)); at least {min_frames} are required"
            ));
        }
        _ => sample.sample_status = "ok".to_string(),
    }
    sample
}

/// Parse just the histogram rows, if the device printed any.
pub fn parse_frames_histogram(text: &str) -> Vec<FramesHistogramBucket> {
    let Ok(pattern) = Regex::new(
        r"^[ \t]*(\d+(?:\.\d+)?)[ \t]*-[ \t]*(\d+(?:\.\d+)?)[ \t]*ms[ \t]*:[ \t]*(\d+)[ \t]*(?:frames?)?[ \t]*(?:[(,][ \t]*(\d+(?:\.\d+)?)[ \t]*%[)\]]?)?",
    ) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| pattern.captures(line))
        .take(MAX_HISTOGRAM_BUCKETS)
        .filter_map(|caps| {
            Some(FramesHistogramBucket {
                start_ms: caps.get(1)?.as_str().parse().ok()?,
                end_ms: caps.get(2)?.as_str().parse().ok()?,
                count: caps.get(3)?.as_str().parse().ok()?,
                percent: caps
                    .get(4)
                    .and_then(|value| value.as_str().parse::<f64>().ok()),
            })
        })
        .collect()
}

/// Parse the verbatim window/header metadata printed by gfxinfo.
pub fn parse_frames_window(text: &str) -> FramesWindow {
    let identity = Regex::new(r"(?i)Graphics info for pid\s+(\d+)\s+\[([^\]]+)\]")
        .ok()
        .and_then(|pattern| pattern.captures(text));
    let raw_excerpt = text
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            line.starts_with("Uptime:")
                || line.starts_with("Realtime:")
                || line.contains("Graphics info for pid")
                || line.starts_with("Stats since:")
        })
        .collect::<Vec<_>>()
        .join("\n");
    FramesWindow {
        pid: identity
            .as_ref()
            .and_then(|caps| caps.get(1))
            .and_then(|value| value.as_str().parse().ok()),
        package: identity
            .as_ref()
            .and_then(|caps| caps.get(2))
            .map(|value| value.as_str().to_string()),
        uptime_ms: capture_u64(r"(?i)\bUptime:\s*(\d+)", text),
        realtime_ms: capture_u64(r"(?i)\bRealtime:\s*(\d+)", text),
        stats_since_ns: capture_u64(r"(?i)\bStats since:\s*(\d+)\s*ns", text),
        raw_excerpt: if raw_excerpt.is_empty() {
            None
        } else if raw_excerpt.chars().count() > RAW_HEADER_MAX_CHARS {
            Some(format!(
                "{}[truncated]",
                raw_excerpt
                    .chars()
                    .take(RAW_HEADER_MAX_CHARS)
                    .collect::<String>()
            ))
        } else {
            Some(raw_excerpt)
        },
    }
}

/// Build a complete report from gfxinfo text without touching a device.
pub fn frames_report_from_gfxinfo(
    text: &str,
    package: &str,
    serial: &str,
    collected_at: &str,
    min_frames: u64,
) -> FramesReport {
    let sample = parse_frames_gfxinfo_with(text, min_frames);
    let warnings = sample
        .unknown_reason
        .clone()
        .into_iter()
        .collect::<Vec<_>>();
    FramesReport {
        schema_version: FRAMES_SCHEMA_VERSION,
        package: package.to_string(),
        serial: serial.to_string(),
        collected_at: collected_at.to_string(),
        sample,
        warnings,
        ..FramesReport::default()
    }
}

/// Compare two frame reports on jank percentage, p99 and maximum frame time.
/// Thresholds are explicit; callers that have no CLI flags should use the
/// strict defaults via [`compare_frames_reports_default`].
pub fn compare_frames_reports(
    baseline: &FramesReport,
    current: &FramesReport,
    thresholds: FramesThresholds,
) -> FramesComparison {
    if let Some(reason) = comparability_reason(baseline, current) {
        return FramesComparison {
            comparable: false,
            reason: Some(reason),
            deltas: Vec::new(),
            regression: false,
        };
    }
    let deltas = vec![
        metric_delta(
            "jank_percent",
            baseline.sample.jank_percent(),
            current.sample.jank_percent(),
            thresholds.jank_percent_pp,
        ),
        metric_delta(
            "p99_ms",
            baseline.sample.p99_ms,
            current.sample.p99_ms,
            thresholds.p99_ms,
        ),
        metric_delta(
            "max_ms",
            baseline.sample.max_ms,
            current.sample.max_ms,
            thresholds.max_ms,
        ),
    ];
    let regression = deltas
        .iter()
        .any(|delta| delta.status == FramesDeltaStatus::Regression);
    FramesComparison {
        comparable: true,
        reason: None,
        deltas,
        regression,
    }
}

/// Strict comparison: any increase in jank %, p99 or max is a regression.
pub fn compare_frames_reports_default(
    baseline: &FramesReport,
    current: &FramesReport,
) -> FramesComparison {
    compare_frames_reports(baseline, current, FramesThresholds::default())
}

fn comparability_reason(baseline: &FramesReport, current: &FramesReport) -> Option<String> {
    for (label, report) in [("baseline", baseline), ("current", current)] {
        let status = report.sample.status();
        if status != "ok" {
            return Some(format!(
                "{label} sample is not comparable (status: {status})"
            ));
        }
    }
    match (baseline.sample.total_frames, current.sample.total_frames) {
        (Some(baseline_frames), Some(current_frames)) if baseline_frames == current_frames => {}
        (Some(baseline_frames), Some(current_frames)) => {
            return Some(format!(
                "frame counts differ ({baseline_frames} vs {current_frames} frame(s))"
            ));
        }
        _ => return Some("frame count is unknown".to_string()),
    }
    None
}

fn metric_delta(
    metric: &str,
    baseline: Option<f64>,
    current: Option<f64>,
    allowed_increase: f64,
) -> FramesMetricDelta {
    match (baseline, current) {
        (Some(baseline), Some(current)) if baseline.is_finite() && current.is_finite() => {
            let delta = current - baseline;
            let status = if delta > allowed_increase + DELTA_EPSILON {
                FramesDeltaStatus::Regression
            } else {
                FramesDeltaStatus::Pass
            };
            FramesMetricDelta {
                metric: metric.to_string(),
                baseline: Some(baseline),
                current: Some(current),
                delta: Some(delta),
                status,
            }
        }
        _ => FramesMetricDelta {
            metric: metric.to_string(),
            baseline,
            current,
            delta: None,
            status: FramesDeltaStatus::Unknown,
        },
    }
}

fn jank_counter(text: &str, label: &str) -> (Option<u64>, Option<f64>) {
    let Ok(pattern) = Regex::new(&format!(
        r"(?im)^\s*{label}:\s*(\d+)\s*(?:\(\s*(\d+(?:\.\d+)?)\s*%\s*\))?"
    )) else {
        return (None, None);
    };
    let Some(caps) = pattern.captures(text) else {
        return (None, None);
    };
    (
        caps.get(1).and_then(|value| value.as_str().parse().ok()),
        caps.get(2).and_then(|value| value.as_str().parse().ok()),
    )
}

fn frame_percentile(text: &str, percentile: u32) -> Option<f64> {
    let pattern =
        Regex::new(r"(?im)^\s*(\d{1,3})(?:st|nd|rd|th)?\s+percentile:\s*(\d+(?:\.\d+)?)\s*ms")
            .ok()?;
    pattern.captures_iter(text).find_map(|caps| {
        let found = caps.get(1)?.as_str().parse::<u32>().ok()?;
        if found == percentile {
            caps.get(2)?.as_str().parse().ok()
        } else {
            None
        }
    })
}

fn frame_max_ms(text: &str) -> Option<f64> {
    for pattern in [
        r"(?im)^\s*Max(?:imum)?(?:\s+frame(?:\s+time)?)?:\s*(\d+(?:\.\d+)?)\s*ms",
        r"(?im)^\s*Longest\s+frame(?:\s+time)?:\s*(\d+(?:\.\d+)?)\s*ms",
        r"(?im)^\s*100(?:th)?\s+percentile:\s*(\d+(?:\.\d+)?)\s*ms",
    ] {
        if let Some(value) = capture_f64(pattern, text) {
            return Some(value);
        }
    }
    None
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

fn capture_f64(pattern: &str, text: &str) -> Option<f64> {
    Regex::new(pattern)
        .ok()?
        .captures(text)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const GFXINFO_TYPICAL: &str = include_str!("../tests/fixtures/frames/gfxinfo_typical.txt");
    const GFXINFO_ONE_FRAME: &str = include_str!("../tests/fixtures/frames/gfxinfo_one_frame.txt");
    const GFXINFO_MISSING: &str = include_str!("../tests/fixtures/frames/gfxinfo_missing.txt");
    const GFXINFO_LEGACY: &str = include_str!("../tests/fixtures/frames/gfxinfo_legacy.txt");
    const GFXINFO_HISTOGRAM: &str = include_str!("../tests/fixtures/frames/gfxinfo_histogram.txt");

    #[test]
    fn parses_typical_gfxinfo() {
        let sample = parse_frames_gfxinfo(GFXINFO_TYPICAL);
        assert_eq!(sample.sample_status, "ok");
        assert_eq!(sample.total_frames, Some(120));
        assert_eq!(sample.janky, Some(9));
        assert_eq!(sample.janky_percent, Some(7.5));
        assert_eq!(sample.janky_legacy, Some(5));
        assert_eq!(sample.janky_legacy_percent, Some(4.17));
        assert_eq!(sample.p50_ms, Some(8.0));
        assert_eq!(sample.p90_ms, Some(16.0));
        assert_eq!(sample.p95_ms, Some(24.0));
        assert_eq!(sample.p99_ms, Some(40.0));
        assert_eq!(sample.max_ms, Some(72.0));
        assert_eq!(sample.missed_vsync, Some(3));
        assert_eq!(sample.slow_ui, Some(2));
        assert_eq!(sample.deadline_missed, Some(1));
        assert_eq!(sample.window.pid, Some(1234));
        assert_eq!(sample.window.package.as_deref(), Some("com.example"));
        assert_eq!(sample.window.uptime_ms, Some(123456));
        assert_eq!(sample.window.realtime_ms, Some(123456));
        assert_eq!(sample.window.stats_since_ns, Some(3411175000000));
        assert!(
            sample
                .window
                .raw_excerpt
                .as_deref()
                .is_some_and(|raw| raw.contains("Graphics info for pid 1234"))
        );
    }

    #[test]
    fn one_frame_sample_is_insufficient_without_jank_percent() {
        let sample = parse_frames_gfxinfo(GFXINFO_ONE_FRAME);
        assert_eq!(sample.sample_status, "insufficient");
        assert_eq!(sample.total_frames, Some(1));
        assert_eq!(sample.janky, Some(1));
        assert_eq!(sample.janky_percent, None);
        assert_eq!(sample.jank_percent(), None);
        assert_eq!(sample.p50_ms, Some(16.0));
        assert!(
            sample
                .unknown_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("1 frame"))
        );
    }

    #[test]
    fn missing_gfxinfo_fields_are_unknown_not_zero() {
        let sample = parse_frames_gfxinfo(GFXINFO_MISSING);
        assert_eq!(sample.sample_status, "unavailable");
        assert_eq!(sample.total_frames, None);
        assert_eq!(sample.janky, None);
        assert_eq!(sample.janky_percent, None);
        assert_eq!(sample.janky_legacy, None);
        assert_eq!(sample.p99_ms, None);
        assert_eq!(sample.max_ms, None);
        assert_eq!(sample.missed_vsync, None);
        assert_eq!(sample.slow_ui, None);
        assert_eq!(sample.deadline_missed, None);
        assert!(sample.histogram.is_empty());
        assert_eq!(
            sample.unknown_reason.as_deref(),
            Some("gfxinfo reported no process for the package")
        );
    }

    #[test]
    fn empty_gfxinfo_reports_no_output() {
        let sample = parse_frames_gfxinfo("  \n");
        assert_eq!(sample.sample_status, "unavailable");
        assert_eq!(
            sample.unknown_reason.as_deref(),
            Some("gfxinfo produced no output")
        );
    }

    #[test]
    fn captures_legacy_and_slow_ui_counters_without_new_dialect_fields() {
        let sample = parse_frames_gfxinfo(GFXINFO_LEGACY);
        assert_eq!(sample.sample_status, "ok");
        assert_eq!(sample.total_frames, Some(60));
        assert_eq!(sample.janky, Some(6));
        assert_eq!(sample.janky_legacy, Some(4));
        assert_eq!(sample.janky_legacy_percent, Some(6.67));
        assert_eq!(sample.slow_ui, Some(2));
        assert_eq!(sample.deadline_missed, Some(0));
        // Older builds do not print p99 or a max frame time.
        assert_eq!(sample.p99_ms, None);
        assert_eq!(sample.max_ms, None);
        assert_eq!(sample.jank_percent(), Some(10.0));
    }

    #[test]
    fn parses_histogram_rows_with_and_without_percent() {
        let buckets = parse_frames_histogram(GFXINFO_HISTOGRAM);
        assert_eq!(buckets.len(), 4);
        assert_eq!(buckets[0].start_ms, 0.0);
        assert_eq!(buckets[0].end_ms, 16.0);
        assert_eq!(buckets[0].count, 100);
        assert_eq!(buckets[0].percent, Some(83.33));
        assert_eq!(buckets[1].count, 10);
        assert_eq!(buckets[1].percent, None);
        assert_eq!(buckets[2].count, 5);
        assert_eq!(buckets[2].percent, Some(4.0));
        assert_eq!(buckets[3].start_ms, 100.0);
        assert_eq!(buckets[3].end_ms, 1000.0);
        assert_eq!(buckets[3].count, 1);
        let sample = parse_frames_gfxinfo(GFXINFO_HISTOGRAM);
        assert_eq!(sample.histogram.len(), 4);
    }

    #[test]
    fn report_from_gfxinfo_is_serialisable_with_expected_shape() {
        let report = frames_report_from_gfxinfo(
            GFXINFO_TYPICAL,
            "com.example",
            "emulator-5554",
            "2026-01-01T00:00:00.000Z",
            DEFAULT_MIN_FRAMES,
        );
        assert_eq!(report.schema_version, FRAMES_SCHEMA_VERSION);
        assert!(!report.reset_before_collection);
        assert!(report.warnings.is_empty());
        let value = serde_json::to_value(&report).expect("serialise frames report");
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["sample"]["janky"], 9);
        assert_eq!(value["sample"]["janky_legacy"], 5);
        assert_eq!(value["sample"]["p99_ms"], 40.0);
        assert_eq!(value["sample"]["max_ms"], 72.0);
        assert_eq!(value["sample"]["slow_ui"], 2);
        assert_eq!(value["sample"]["deadline_missed"], 1);
        assert!(value["sample"]["histogram"].is_array());
        assert!(value["sample"]["window"]["raw_excerpt"].is_string());
    }

    fn baseline_report() -> FramesReport {
        frames_report_from_gfxinfo(
            GFXINFO_TYPICAL,
            "com.example",
            "emulator-5554",
            "2026-01-01T00:00:00.000Z",
            DEFAULT_MIN_FRAMES,
        )
    }

    fn current_report(janky: &str, p99: &str, max: &str) -> FramesReport {
        let text = GFXINFO_TYPICAL
            .replace("Janky frames: 9 (7.50%)", janky)
            .replace("99th percentile: 40ms", p99)
            .replace("Max: 72ms", max);
        frames_report_from_gfxinfo(
            &text,
            "com.example",
            "emulator-5554",
            "2026-01-02T00:00:00.000Z",
            DEFAULT_MIN_FRAMES,
        )
    }

    #[test]
    fn baseline_identical_sample_passes() {
        let baseline = baseline_report();
        let current = baseline.clone();
        let comparison = compare_frames_reports_default(&baseline, &current);
        assert!(comparison.comparable, "{comparison:?}");
        assert!(!comparison.regression);
        assert_eq!(comparison.deltas.len(), 3);
        for delta in &comparison.deltas {
            assert_eq!(delta.status, FramesDeltaStatus::Pass, "{delta:?}");
            assert_eq!(delta.delta, Some(0.0));
        }
    }

    #[test]
    fn baseline_improvement_passes() {
        let baseline = baseline_report();
        let current = current_report(
            "Janky frames: 3 (2.50%)",
            "99th percentile: 20ms",
            "Max: 40ms",
        );
        let comparison = compare_frames_reports_default(&baseline, &current);
        assert!(comparison.comparable);
        assert!(!comparison.regression);
        assert!(
            comparison
                .deltas
                .iter()
                .all(|delta| delta.delta.is_some_and(|value| value < 0.0))
        );
    }

    #[test]
    fn baseline_jank_regression_is_structured() {
        let baseline = baseline_report();
        let current = current_report(
            "Janky frames: 18 (15.00%)",
            "99th percentile: 40ms",
            "Max: 72ms",
        );
        let comparison = compare_frames_reports_default(&baseline, &current);
        assert!(comparison.comparable);
        assert!(comparison.regression);
        let jank = comparison
            .deltas
            .iter()
            .find(|delta| delta.metric == "jank_percent")
            .expect("jank delta");
        assert_eq!(jank.status, FramesDeltaStatus::Regression);
        assert_eq!(jank.baseline, Some(7.5));
        assert_eq!(jank.current, Some(15.0));
        assert!((jank.delta.expect("jank delta value") - 7.5).abs() < 1e-9);
    }

    #[test]
    fn baseline_p99_and_max_regression_with_explicit_thresholds() {
        let baseline = baseline_report();
        let current = current_report(
            "Janky frames: 9 (7.50%)",
            "99th percentile: 48ms",
            "Max: 80ms",
        );
        let strict = compare_frames_reports_default(&baseline, &current);
        assert!(strict.regression);
        let tolerant = compare_frames_reports(
            &baseline,
            &current,
            FramesThresholds {
                jank_percent_pp: 1.0,
                p99_ms: 10.0,
                max_ms: 10.0,
            },
        );
        assert!(tolerant.comparable);
        assert!(!tolerant.regression, "{tolerant:?}");
    }

    #[test]
    fn baseline_missing_metric_is_unknown_not_regression() {
        let baseline = baseline_report();
        let mut current = baseline.clone();
        current.sample.p99_ms = None;
        current.sample.max_ms = None;
        let comparison = compare_frames_reports_default(&baseline, &current);
        assert!(comparison.comparable);
        assert!(!comparison.regression);
        assert_eq!(comparison.deltas[1].status, FramesDeltaStatus::Unknown);
        assert_eq!(comparison.deltas[2].status, FramesDeltaStatus::Unknown);
        assert_eq!(comparison.deltas[1].delta, None);
    }

    #[test]
    fn baseline_different_frame_counts_are_not_comparable() {
        let baseline = baseline_report();
        let mut current = baseline.clone();
        current.sample.total_frames = Some(60);
        let comparison = compare_frames_reports_default(&baseline, &current);
        assert!(!comparison.comparable);
        assert!(!comparison.regression);
        assert!(
            comparison
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("frame counts differ"))
        );
        assert!(comparison.deltas.is_empty());
    }

    #[test]
    fn legacy_report_deserialises_with_defaults() {
        let value: Value = serde_json::json!({"package": "com.example"});
        let report: FramesReport = serde_json::from_value(value).expect("legacy report");
        assert_eq!(report.schema_version, FRAMES_SCHEMA_VERSION);
        assert_eq!(report.sample.sample_status, "");
        assert!(report.sample.histogram.is_empty());
    }
}
