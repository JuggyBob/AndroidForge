use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::BuildIdentity;
use crate::run::{RunReport, RunStatus, StepStatus};
use crate::util::now_rfc3339;

pub const ANALYSIS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FieldChange {
    pub field: String,
    pub current: String,
    pub baseline: String,
    pub changed: bool,
}

impl FieldChange {
    fn new(field: &str, current: impl Into<String>, baseline: impl Into<String>) -> Self {
        let current = current.into();
        let baseline = baseline.into();
        Self {
            field: field.to_string(),
            changed: current != baseline,
            current,
            baseline,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepComparison {
    pub name: String,
    pub command: String,
    pub current_status: Option<String>,
    pub baseline_status: Option<String>,
    pub current_duration_ms: Option<u64>,
    pub baseline_duration_ms: Option<u64>,
    pub duration_delta_ms: Option<i64>,
    pub current_failure: Option<String>,
    pub baseline_failure: Option<String>,
    pub change: String,
    pub current_artifacts: usize,
    pub baseline_artifacts: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunComparison {
    pub schema_version: u32,
    pub generated_at: String,
    pub current_run_id: String,
    pub baseline_run_id: String,
    pub comparable: bool,
    pub notes: Vec<String>,
    pub build_changes: Vec<FieldChange>,
    pub environment_changes: Vec<FieldChange>,
    pub step_changes: Vec<StepComparison>,
    pub new_failures: Vec<String>,
    pub resolved_failures: Vec<String>,
    pub artifacts_added: Vec<String>,
    pub artifacts_removed: Vec<String>,
    pub current_status: RunStatus,
    pub baseline_status: RunStatus,
    pub current_duration_ms: u64,
    pub baseline_duration_ms: u64,
    pub duration_delta_ms: i64,
    /// Performance evidence extracted from step details and run artifacts.
    /// Empty for older reports and for runs that are not comparable.
    #[serde(default, skip_serializing_if = "PerformanceComparison::is_empty")]
    pub performance: PerformanceComparison,
    /// Thresholds used for the budget evaluation, when any were supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<PerformanceBudget>,
    /// Every threshold that could be evaluated (including passing checks).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub budget_checks: Vec<BudgetViolation>,
    /// The subset of `budget_checks` that failed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub budget_violations: Vec<BudgetViolation>,
    /// `true` when every evaluated budget check passed, `None` when no budget
    /// was configured or the runs were not comparable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_passed: Option<bool>,
}

/// Report-oriented alias for the canonical `RunComparison` name.
pub type ReportComparison = RunComparison;

/// One metric observed in both runs. `delta` is `current - baseline` and is
/// absent when a value is missing from either side. `source` is the flattened
/// step-detail or artifact path the value was read from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PerformanceMetricComparison {
    #[serde(default)]
    pub baseline: Option<f64>,
    #[serde(default)]
    pub current: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl PerformanceMetricComparison {
    fn new(baseline: Option<f64>, current: Option<f64>, source: Option<String>) -> Self {
        Self {
            baseline,
            current,
            delta: baseline.zip(current).map(|(base, now)| now - base),
            source,
        }
    }
}

/// Additive performance comparison. All fields are optional so reports that
/// predate performance evidence keep their original shape.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PerformanceComparison {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_ms: Option<PerformanceMetricComparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jank_percent: Option<PerformanceMetricComparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub janky_frames: Option<PerformanceMetricComparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_frames: Option<PerformanceMetricComparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_frame_ms: Option<PerformanceMetricComparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apk_size_bytes: Option<PerformanceMetricComparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests_total: Option<PerformanceMetricComparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests_failed: Option<PerformanceMetricComparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests_skipped: Option<PerformanceMetricComparison>,
    /// Additional numeric metrics found in step details, keyed by flattened
    /// path. Recognised metrics above are not duplicated here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metrics: BTreeMap<String, PerformanceMetricComparison>,
}

impl PerformanceComparison {
    pub fn is_empty(&self) -> bool {
        self.startup_ms.is_none()
            && self.jank_percent.is_none()
            && self.janky_frames.is_none()
            && self.total_frames.is_none()
            && self.max_frame_ms.is_none()
            && self.apk_size_bytes.is_none()
            && self.tests_total.is_none()
            && self.tests_failed.is_none()
            && self.tests_skipped.is_none()
            && self.metrics.is_empty()
    }
}

/// Thresholds for report performance budgets. `*_ms`, `*_percent`,
/// `*_bytes`, `*_frames` and `*_tests` fields are absolute ceilings;
/// `*_delta_*` fields cap the positive change from the baseline. Named
/// `metrics` and `metric_deltas` apply to flattened step-detail metrics.
/// An absent threshold is not evaluated, and a missing metric never produces
/// a failed check.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PerformanceBudget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jank_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub janky_frames: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_frame_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apk_size_bytes: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_tests: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_delta_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jank_percent_delta: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_frame_delta_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apk_size_delta_bytes: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_tests_delta: Option<f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metrics: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metric_deltas: BTreeMap<String, f64>,
}

impl PerformanceBudget {
    pub fn is_empty(&self) -> bool {
        self.startup_ms.is_none()
            && self.jank_percent.is_none()
            && self.janky_frames.is_none()
            && self.max_frame_ms.is_none()
            && self.apk_size_bytes.is_none()
            && self.failed_tests.is_none()
            && self.startup_delta_ms.is_none()
            && self.jank_percent_delta.is_none()
            && self.max_frame_delta_ms.is_none()
            && self.apk_size_delta_bytes.is_none()
            && self.failed_tests_delta.is_none()
            && self.metrics.is_empty()
            && self.metric_deltas.is_empty()
    }

    /// Evaluate every configured threshold against observed metrics. Checks
    /// whose metric is unavailable are omitted rather than failed.
    pub fn evaluate(&self, performance: &PerformanceComparison) -> Vec<BudgetViolation> {
        let mut checks = Vec::new();
        for (metric, limit, observed) in [
            ("startup_ms", self.startup_ms, &performance.startup_ms),
            ("jank_percent", self.jank_percent, &performance.jank_percent),
            ("janky_frames", self.janky_frames, &performance.janky_frames),
            ("max_frame_ms", self.max_frame_ms, &performance.max_frame_ms),
            (
                "apk_size_bytes",
                self.apk_size_bytes,
                &performance.apk_size_bytes,
            ),
            ("failed_tests", self.failed_tests, &performance.tests_failed),
        ] {
            if let Some(limit) = limit
                && let Some(observed) = observed
                && let Some(check) = BudgetViolation::absolute(metric, limit, observed)
            {
                checks.push(check);
            }
        }
        for (metric, limit, observed) in [
            (
                "startup_delta_ms",
                self.startup_delta_ms,
                &performance.startup_ms,
            ),
            (
                "jank_percent_delta",
                self.jank_percent_delta,
                &performance.jank_percent,
            ),
            (
                "max_frame_delta_ms",
                self.max_frame_delta_ms,
                &performance.max_frame_ms,
            ),
            (
                "apk_size_delta_bytes",
                self.apk_size_delta_bytes,
                &performance.apk_size_bytes,
            ),
            (
                "failed_tests_delta",
                self.failed_tests_delta,
                &performance.tests_failed,
            ),
        ] {
            if let Some(limit) = limit
                && let Some(observed) = observed
                && let Some(check) = BudgetViolation::change(metric, limit, observed)
            {
                checks.push(check);
            }
        }
        for (metric, limit) in &self.metrics {
            if let Some(observed) = performance.metrics.get(metric)
                && let Some(check) = BudgetViolation::absolute(metric, *limit, observed)
            {
                checks.push(check);
            }
        }
        for (metric, limit) in &self.metric_deltas {
            if let Some(observed) = performance.metrics.get(metric)
                && let Some(check) = BudgetViolation::change(metric, *limit, observed)
            {
                checks.push(check);
            }
        }
        checks.sort_by(|left, right| left.metric.cmp(&right.metric));
        checks
    }
}

/// Result of one budget threshold evaluation. `passed` is `true` when the
/// observed value stayed within `expected`; `basis` distinguishes an absolute
/// ceiling from a maximum allowed change relative to the baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetViolation {
    pub metric: String,
    pub expected: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<f64>,
    pub passed: bool,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
}

impl BudgetViolation {
    fn absolute(metric: &str, limit: f64, observed: &PerformanceMetricComparison) -> Option<Self> {
        let actual = observed.current?;
        let passed = actual <= limit;
        let outcome = if passed { "within" } else { "exceeded" };
        Some(Self {
            metric: metric.to_string(),
            expected: limit,
            actual: Some(actual),
            delta: observed.delta,
            passed,
            reason: format!("{metric} {actual} {outcome} the {limit} budget"),
            basis: Some("absolute".to_string()),
        })
    }

    fn change(metric: &str, limit: f64, observed: &PerformanceMetricComparison) -> Option<Self> {
        let delta = observed.delta?;
        let passed = delta <= limit;
        let outcome = if passed { "within" } else { "exceeded" };
        Some(Self {
            metric: metric.to_string(),
            expected: limit,
            actual: observed.current,
            delta: Some(delta),
            passed,
            reason: format!("{metric} change {delta:+} {outcome} the {limit:+} budget"),
            basis: Some("delta".to_string()),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakyObservation {
    pub run_id: String,
    pub started_at: String,
    pub run_status: RunStatus,
    pub step_status: StepStatus,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakyStep {
    pub key: String,
    pub command: String,
    pub step_name: String,
    pub executions: usize,
    pub successes: usize,
    pub failures: usize,
    pub skipped: usize,
    pub sufficient_evidence: bool,
    pub classification: String,
    pub heuristic: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observations: Vec<FlakyObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakyReport {
    pub schema_version: u32,
    pub generated_at: String,
    pub project: String,
    pub runs_analyzed: usize,
    pub min_runs: usize,
    pub steps: Vec<FlakyStep>,
    pub heuristic: String,
}

fn opt(value: Option<&str>) -> String {
    value.unwrap_or("-").to_string()
}

fn build_changes(current: &BuildIdentity, baseline: &BuildIdentity) -> Vec<FieldChange> {
    [
        FieldChange::new(
            "package",
            opt(current.package.as_deref()),
            opt(baseline.package.as_deref()),
        ),
        FieldChange::new(
            "version_name",
            opt(current.version_name.as_deref()),
            opt(baseline.version_name.as_deref()),
        ),
        FieldChange::new(
            "version_code",
            current
                .version_code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            baseline
                .version_code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ),
        FieldChange::new(
            "apk_sha256",
            opt(current.apk_sha256.as_deref()),
            opt(baseline.apk_sha256.as_deref()),
        ),
        FieldChange::new(
            "installed_apk_sha256",
            opt(current.installed_apk_sha256.as_deref()),
            opt(baseline.installed_apk_sha256.as_deref()),
        ),
        FieldChange::new(
            "cert_sha256",
            opt(current.cert_sha256.as_deref()),
            opt(baseline.cert_sha256.as_deref()),
        ),
        FieldChange::new(
            "verified",
            current.verified.to_string(),
            baseline.verified.to_string(),
        ),
    ]
    .into_iter()
    .filter(|change| change.changed)
    .collect()
}

fn environment_changes(current: &RunReport, baseline: &RunReport) -> Vec<FieldChange> {
    let current_device = current.device.as_ref();
    let baseline_device = baseline.device.as_ref();
    [
        FieldChange::new(
            "device_serial",
            opt(current.device_serial.as_deref()),
            opt(baseline.device_serial.as_deref()),
        ),
        FieldChange::new(
            "device_model",
            opt(current_device.and_then(|device| device.model.as_deref())),
            opt(baseline_device.and_then(|device| device.model.as_deref())),
        ),
        FieldChange::new(
            "android_version",
            opt(current_device.and_then(|device| device.android_version.as_deref())),
            opt(baseline_device.and_then(|device| device.android_version.as_deref())),
        ),
        FieldChange::new(
            "api_level",
            current_device
                .and_then(|device| device.api_level)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            baseline_device
                .and_then(|device| device.api_level)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ),
        FieldChange::new(
            "device_product",
            opt(current_device.and_then(|device| device.product.as_deref())),
            opt(baseline_device.and_then(|device| device.product.as_deref())),
        ),
    ]
    .into_iter()
    .filter(|change| change.changed)
    .collect()
}

fn step_map(report: &RunReport) -> BTreeMap<(String, String), &crate::run::RunStep> {
    report
        .steps
        .iter()
        .map(|step| ((step.name.clone(), step.command.clone()), step))
        .collect()
}

fn failure_label(step: &crate::run::RunStep) -> Option<String> {
    step.failure
        .as_ref()
        .map(|failure| format!("{}: {}", failure.code.as_str(), failure.message))
}

const MAX_METRIC_DEPTH: usize = 4;
const MAX_METRICS_PER_STEP: usize = 32;
const MAX_METRICS_TOTAL: usize = 128;
const MAX_NAMED_COMPARISONS: usize = 64;

fn key_ends_with(key: &str, suffix: &str) -> bool {
    key == suffix || key.strip_suffix(&format!(".{suffix}")).is_some()
}

fn lookup_metric(metrics: &BTreeMap<String, f64>, suffixes: &[&str]) -> Option<(f64, String)> {
    for suffix in suffixes {
        let mut matches = metrics
            .iter()
            .filter(|(key, _)| key_ends_with(key, suffix))
            .collect::<Vec<_>>();
        matches.sort_by_key(|entry| std::cmp::Reverse(entry.0.len()));
        if let Some((key, value)) = matches.first() {
            return Some((**value, (*key).clone()));
        }
    }
    None
}

fn sum_metrics(
    metrics: &BTreeMap<String, f64>,
    prefix: &str,
    suffixes: &[&str],
) -> Option<(f64, String)> {
    let mut total = 0.0;
    let mut count = 0;
    for (key, value) in metrics {
        if key.starts_with(prefix) && suffixes.iter().any(|suffix| key_ends_with(key, suffix)) {
            total += value;
            count += 1;
        }
    }
    (count > 0).then_some((total, format!("{prefix}sum")))
}

fn step_metric_prefix(step: &crate::run::RunStep) -> String {
    if let Some(suite) = step.name.strip_prefix("junit:") {
        return format!("junit.{suite}");
    }
    let command = step
        .command
        .split_whitespace()
        .next()
        .unwrap_or(&step.command);
    if command.is_empty() {
        "step".to_string()
    } else {
        command.to_string()
    }
}

fn flatten_metrics(
    value: &Value,
    prefix: &str,
    depth: usize,
    blocked: &[String],
    out: &mut BTreeMap<String, f64>,
) {
    if out.len() >= MAX_METRICS_TOTAL {
        return;
    }
    let Value::Object(map) = value else {
        return;
    };
    for (key, child) in map {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if blocked
            .iter()
            .any(|blocked| path == *blocked || path.starts_with(&format!("{blocked}.")))
        {
            continue;
        }
        match child {
            Value::Object(_) if depth + 1 < MAX_METRIC_DEPTH => {
                flatten_metrics(child, &path, depth + 1, blocked, out);
            }
            Value::Number(number) => {
                if let Some(number) = number.as_f64().filter(|number| number.is_finite()) {
                    out.insert(path, number);
                }
            }
            _ => {}
        }
        if out.len() >= MAX_METRICS_TOTAL {
            break;
        }
    }
}

fn collect_step_metrics(step: &crate::run::RunStep, out: &mut BTreeMap<String, f64>) {
    let prefix = step_metric_prefix(step);
    let mut blocked = Vec::new();
    if let Some(jank) = step.details.get("jank")
        && (jank.get("comparable") == Some(&Value::Bool(false))
            || jank
                .get("sample_status")
                .and_then(Value::as_str)
                .is_some_and(|status| status != "ok"))
    {
        blocked.push(format!("{prefix}.jank"));
    }
    if let Some(sample) = step.details.get("sample")
        && sample
            .get("sample_status")
            .and_then(Value::as_str)
            .is_some_and(|status| status != "ok")
    {
        blocked.push(format!("{prefix}.sample"));
    }
    let mut local = BTreeMap::new();
    flatten_metrics(&step.details, &prefix, 0, &blocked, &mut local);
    for (key, value) in local.into_iter().take(MAX_METRICS_PER_STEP) {
        if out.len() >= MAX_METRICS_TOTAL {
            break;
        }
        out.insert(key, value);
    }
}

fn collect_metrics(report: &RunReport) -> BTreeMap<String, f64> {
    let mut metrics = BTreeMap::new();
    for step in &report.steps {
        collect_step_metrics(step, &mut metrics);
    }
    let apk_size = report
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind.to_ascii_lowercase().contains("apk"))
        .filter_map(|artifact| artifact.size_bytes.map(|size| size as f64))
        .max_by(|left, right| left.total_cmp(right));
    if let Some(size) = apk_size {
        metrics.insert("artifacts.apk.size_bytes".to_string(), size);
    }
    metrics
}

fn metric_comparison(
    baseline: &BTreeMap<String, f64>,
    current: &BTreeMap<String, f64>,
    suffixes: &[&str],
) -> Option<PerformanceMetricComparison> {
    let baseline_value = lookup_metric(baseline, suffixes);
    let current_value = lookup_metric(current, suffixes);
    if baseline_value.is_none() && current_value.is_none() {
        return None;
    }
    let source = current_value
        .as_ref()
        .or(baseline_value.as_ref())
        .map(|(_, source)| source.clone());
    Some(PerformanceMetricComparison::new(
        baseline_value.map(|(value, _)| value),
        current_value.map(|(value, _)| value),
        source,
    ))
}

fn summed_metric_comparison(
    baseline: &BTreeMap<String, f64>,
    current: &BTreeMap<String, f64>,
    suffixes: &[&str],
) -> Option<PerformanceMetricComparison> {
    let baseline_value =
        sum_metrics(baseline, "junit.", suffixes).or_else(|| lookup_metric(baseline, suffixes));
    let current_value =
        sum_metrics(current, "junit.", suffixes).or_else(|| lookup_metric(current, suffixes));
    if baseline_value.is_none() && current_value.is_none() {
        return None;
    }
    let source = current_value
        .as_ref()
        .or(baseline_value.as_ref())
        .map(|(_, source)| source.clone());
    Some(PerformanceMetricComparison::new(
        baseline_value.map(|(value, _)| value),
        current_value.map(|(value, _)| value),
        source,
    ))
}

fn compare_performance(current: &RunReport, baseline: &RunReport) -> PerformanceComparison {
    let baseline_metrics = collect_metrics(baseline);
    let current_metrics = collect_metrics(current);
    let mut comparison = PerformanceComparison {
        startup_ms: metric_comparison(
            &baseline_metrics,
            &current_metrics,
            &["startup.total_time_ms", "startup_ms", "total_time_ms"],
        ),
        jank_percent: metric_comparison(
            &baseline_metrics,
            &current_metrics,
            &[
                "jank.janky_percent",
                "sample.janky_percent",
                "janky_percent",
            ],
        ),
        janky_frames: metric_comparison(
            &baseline_metrics,
            &current_metrics,
            &["jank.janky_frames", "sample.janky", "janky_frames"],
        ),
        total_frames: metric_comparison(
            &baseline_metrics,
            &current_metrics,
            &["jank.total_frames", "sample.total_frames", "total_frames"],
        ),
        max_frame_ms: metric_comparison(
            &baseline_metrics,
            &current_metrics,
            &[
                "sample.max_ms",
                "jank.percentile_99_ms",
                "max_frame_ms",
                "max_ms",
            ],
        ),
        apk_size_bytes: metric_comparison(
            &baseline_metrics,
            &current_metrics,
            &[
                "artifacts.apk.size_bytes",
                "apk_size_bytes",
                "apk.size_bytes",
                "selected.size_bytes",
            ],
        ),
        tests_total: summed_metric_comparison(
            &baseline_metrics,
            &current_metrics,
            &["tests", "test_count"],
        ),
        tests_failed: summed_metric_comparison(
            &baseline_metrics,
            &current_metrics,
            &["failures", "errors", "failed_tests", "tests_failed"],
        ),
        tests_skipped: summed_metric_comparison(
            &baseline_metrics,
            &current_metrics,
            &["skipped", "skipped_tests"],
        ),
        metrics: BTreeMap::new(),
    };

    let recognised: BTreeSet<String> = [
        &comparison.startup_ms,
        &comparison.jank_percent,
        &comparison.janky_frames,
        &comparison.total_frames,
        &comparison.max_frame_ms,
        &comparison.apk_size_bytes,
        &comparison.tests_total,
        &comparison.tests_failed,
        &comparison.tests_skipped,
    ]
    .into_iter()
    .filter_map(|metric| metric.as_ref().and_then(|metric| metric.source.clone()))
    .collect();

    for key in current_metrics
        .keys()
        .chain(baseline_metrics.keys())
        .collect::<BTreeSet<_>>()
    {
        if recognised.contains(key) || comparison.metrics.len() >= MAX_NAMED_COMPARISONS {
            continue;
        }
        comparison.metrics.insert(
            key.clone(),
            PerformanceMetricComparison::new(
                baseline_metrics.get(key).copied(),
                current_metrics.get(key).copied(),
                Some(key.clone()),
            ),
        );
    }
    comparison
}

/// Compare two run reports without a performance budget. Existing comparison
/// fields are unchanged; performance deltas are reported but no thresholds
/// are evaluated.
pub fn compare_reports(current: &RunReport, baseline: &RunReport) -> RunComparison {
    compare_with_budget(current, baseline, &PerformanceBudget::default())
}

/// Compare two run reports and evaluate the supplied performance budget.
/// Budget checks are skipped entirely when the runs are not comparable.
pub fn compare_with_budget(
    current: &RunReport,
    baseline: &RunReport,
    budget: &PerformanceBudget,
) -> RunComparison {
    let mut comparison = compare_report_fields(current, baseline);
    if comparison.comparable {
        comparison.performance = compare_performance(current, baseline);
        if !budget.is_empty() {
            let checks = budget.evaluate(&comparison.performance);
            comparison.budget = Some(budget.clone());
            comparison.budget_passed = Some(checks.iter().all(|check| check.passed));
            comparison.budget_violations = checks
                .iter()
                .filter(|check| !check.passed)
                .cloned()
                .collect();
            comparison.budget_checks = checks;
        }
    }
    comparison
}

fn compare_report_fields(current: &RunReport, baseline: &RunReport) -> RunComparison {
    let mut notes = Vec::new();
    let comparable = current.project == baseline.project && current.command == baseline.command;
    if current.project != baseline.project {
        notes.push(format!(
            "Project differs: current '{}', baseline '{}'.",
            current.project, baseline.project
        ));
    }
    if current.command != baseline.command {
        notes.push(format!(
            "Scenario differs: current '{}', baseline '{}'.",
            current.command, baseline.command
        ));
    }

    let build_changes = build_changes(&current.build, &baseline.build);
    if !build_changes.is_empty() {
        notes.push(
            "Build identity differs; a step regression may be caused by the build rather than the test."
                .to_string(),
        );
    }
    let environment_changes = environment_changes(current, baseline);
    if !environment_changes.is_empty() {
        notes.push(
            "Device or Android environment differs; treat step changes as environment-sensitive."
                .to_string(),
        );
    }

    let current_steps = step_map(current);
    let baseline_steps = step_map(baseline);
    let mut keys = BTreeSet::new();
    keys.extend(current_steps.keys().cloned());
    keys.extend(baseline_steps.keys().cloned());

    let mut step_changes = Vec::new();
    let mut new_failures = Vec::new();
    let mut resolved_failures = Vec::new();
    for key in keys {
        let current_step = current_steps.get(&key);
        let baseline_step = baseline_steps.get(&key);
        let current_failed = current_step.is_some_and(|step| step.status == StepStatus::Failed);
        let baseline_failed = baseline_step.is_some_and(|step| step.status == StepStatus::Failed);
        let change = match (current_step, baseline_step) {
            (Some(current), Some(_)) if current_failed && !baseline_failed => {
                new_failures.push(current.name.clone());
                "regressed"
            }
            (Some(_), Some(baseline)) if !current_failed && baseline_failed => {
                resolved_failures.push(baseline.name.clone());
                "fixed"
            }
            (Some(current), Some(_)) if current.status == StepStatus::Failed => "failed_both",
            (Some(_), Some(_)) => "unchanged",
            (Some(_), None) => "added",
            (None, Some(_)) => "removed",
            (None, None) => "unknown",
        };
        let current_duration = current_step.map(|step| step.duration_ms);
        let baseline_duration = baseline_step.map(|step| step.duration_ms);
        let duration_delta = current_duration
            .zip(baseline_duration)
            .map(|(current, baseline)| current as i64 - baseline as i64);
        let name = current_step
            .map(|step| step.name.clone())
            .or_else(|| baseline_step.map(|step| step.name.clone()))
            .unwrap_or_default();
        let command = current_step
            .map(|step| step.command.clone())
            .or_else(|| baseline_step.map(|step| step.command.clone()))
            .unwrap_or_default();
        step_changes.push(StepComparison {
            name,
            command,
            current_status: current_step.map(|step| format!("{:?}", step.status).to_lowercase()),
            baseline_status: baseline_step.map(|step| format!("{:?}", step.status).to_lowercase()),
            current_duration_ms: current_duration,
            baseline_duration_ms: baseline_duration,
            duration_delta_ms: duration_delta,
            current_failure: current_step.and_then(|step| failure_label(step)),
            baseline_failure: baseline_step.and_then(|step| failure_label(step)),
            change: change.to_string(),
            current_artifacts: current_step.map(|step| step.artifacts.len()).unwrap_or(0),
            baseline_artifacts: baseline_step.map(|step| step.artifacts.len()).unwrap_or(0),
        });
    }

    let current_artifacts: BTreeSet<_> = current
        .artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect();
    let baseline_artifacts: BTreeSet<_> = baseline
        .artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect();
    let artifacts_added = current_artifacts
        .difference(&baseline_artifacts)
        .cloned()
        .collect();
    let artifacts_removed = baseline_artifacts
        .difference(&current_artifacts)
        .cloned()
        .collect();

    RunComparison {
        schema_version: ANALYSIS_SCHEMA_VERSION,
        generated_at: now_rfc3339(),
        current_run_id: current.run_id.clone(),
        baseline_run_id: baseline.run_id.clone(),
        comparable,
        notes,
        build_changes,
        environment_changes,
        step_changes,
        new_failures,
        resolved_failures,
        artifacts_added,
        artifacts_removed,
        current_status: current.status,
        baseline_status: baseline.status,
        current_duration_ms: current.duration_ms,
        baseline_duration_ms: baseline.duration_ms,
        duration_delta_ms: current.duration_ms as i64 - baseline.duration_ms as i64,
        performance: PerformanceComparison::default(),
        budget: None,
        budget_checks: Vec::new(),
        budget_violations: Vec::new(),
        budget_passed: None,
    }
}

pub fn analyze_flaky(runs: &[RunReport], min_runs: usize) -> FlakyReport {
    let min_runs = min_runs.max(2);
    let mut groups: BTreeMap<String, Vec<FlakyObservation>> = BTreeMap::new();
    for run in runs {
        if !run.status.is_terminal()
            || matches!(
                run.status,
                RunStatus::Cancelled
                    | RunStatus::Interrupted
                    | RunStatus::TimedOut
                    | RunStatus::Abandoned
            )
        {
            continue;
        }
        for step in &run.steps {
            let key = format!("{}\u{1f}{}", run.command, step.name);
            groups.entry(key).or_default().push(FlakyObservation {
                run_id: run.run_id.clone(),
                started_at: run.started_at.clone(),
                run_status: run.status,
                step_status: step.status,
                duration_ms: step.duration_ms,
                failure_code: step
                    .failure
                    .as_ref()
                    .map(|failure| failure.code.as_str().to_string()),
                failure_message: step.failure.as_ref().map(|failure| failure.message.clone()),
            });
        }
    }

    let heuristic = format!(
        "A step is classified flaky when it has at least one success and one failure across at least {min_runs} compatible terminal runs. Cancelled, interrupted, timed-out and abandoned runs are excluded."
    );
    let mut steps = Vec::new();
    for (key, mut observations) in groups {
        observations.sort_by(|left, right| right.started_at.cmp(&left.started_at));
        let executions = observations.len();
        let successes = observations
            .iter()
            .filter(|observation| observation.step_status == StepStatus::Success)
            .count();
        let failures = observations
            .iter()
            .filter(|observation| observation.step_status == StepStatus::Failed)
            .count();
        let skipped = observations
            .iter()
            .filter(|observation| observation.step_status == StepStatus::Skipped)
            .count();
        let sufficient_evidence = executions >= min_runs;
        let classification = if !sufficient_evidence {
            "insufficient_evidence"
        } else if failures == 0 {
            "stable_success"
        } else if successes == 0 {
            "stable_failure"
        } else {
            "flaky"
        };
        let (command, step_name) = key
            .split_once('\u{1f}')
            .map(|(command, step)| (command.to_string(), step.to_string()))
            .unwrap_or_else(|| (String::new(), key.clone()));
        observations.truncate(10);
        steps.push(FlakyStep {
            key,
            command,
            step_name,
            executions,
            successes,
            failures,
            skipped,
            sufficient_evidence,
            classification: classification.to_string(),
            heuristic: heuristic.clone(),
            observations,
        });
    }
    steps.sort_by(|left, right| {
        let left_flaky = left.classification == "flaky";
        let right_flaky = right.classification == "flaky";
        right_flaky
            .cmp(&left_flaky)
            .then_with(|| right.failures.cmp(&left.failures))
            .then_with(|| right.executions.cmp(&left.executions))
            .then_with(|| left.step_name.cmp(&right.step_name))
    });

    let project = runs
        .first()
        .map(|run| run.project.clone())
        .unwrap_or_default();
    FlakyReport {
        schema_version: ANALYSIS_SCHEMA_VERSION,
        generated_at: now_rfc3339(),
        project,
        runs_analyzed: runs.len(),
        min_runs,
        steps,
        heuristic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ArtifactRef, Device};
    use crate::run::{RunReport, RunStep};

    fn step(name: &str, status: StepStatus) -> RunStep {
        RunStep {
            id: name.to_string(),
            name: name.to_string(),
            command: format!("cmd {name}"),
            status,
            exit_code: Some(if status == StepStatus::Failed { 1 } else { 0 }),
            started_at: "2026-01-01T00:00:00Z".to_string(),
            ended_at: Some("2026-01-01T00:00:01Z".to_string()),
            duration_ms: 1000,
            failure: (status == StepStatus::Failed).then(|| crate::run::FailureInfo {
                code: crate::error::ErrorCode::CommandFailed,
                message: "boom".to_string(),
                failed_operation: name.to_string(),
                recoverable: true,
                suggestions: vec![],
                context: BTreeMap::new(),
                artifacts: vec![],
            }),
            artifacts: vec![],
            details: serde_json::json!({}),
            retries: 0,
        }
    }

    fn run(id: &str, status: RunStatus, step_status: StepStatus) -> RunReport {
        let mut report = RunReport::new(id, "conformance.run", "App", "C:\\App");
        report.status = status;
        let mut device = Device::basic("emulator-5554", crate::model::DeviceState::Device);
        device.model = Some("Pixel".to_string());
        device.android_version = Some("16".to_string());
        report.device = Some(device);
        report.device_serial = Some("emulator-5554".to_string());
        report.steps.push(step("launch", step_status));
        report
    }

    #[test]
    fn comparison_finds_regressions_and_build_changes() {
        let baseline = run("baseline", RunStatus::Passed, StepStatus::Success);
        let mut current = run("current", RunStatus::Failed, StepStatus::Failed);
        current.build.version_name = Some("2.0".to_string());
        current.artifacts.push(ArtifactRef {
            kind: "screenshot".to_string(),
            path: "screenshots/fail.png".to_string(),
            sha256: None,
            size_bytes: None,
            description: None,
        });
        let comparison = compare_reports(&current, &baseline);
        assert!(comparison.comparable);
        assert_eq!(comparison.new_failures, vec!["launch"]);
        assert_eq!(
            comparison
                .step_changes
                .iter()
                .find(|step| step.name == "launch")
                .unwrap()
                .change,
            "regressed"
        );
        assert!(!comparison.build_changes.is_empty());
        assert_eq!(comparison.artifacts_added.len(), 1);
    }

    #[test]
    fn flaky_requires_both_outcomes_and_enough_runs() {
        let runs = vec![
            run("one", RunStatus::Passed, StepStatus::Success),
            run("two", RunStatus::Failed, StepStatus::Failed),
            run("three", RunStatus::Passed, StepStatus::Success),
        ];
        let report = analyze_flaky(&runs, 3);
        let launch = report
            .steps
            .iter()
            .find(|step| step.step_name == "launch")
            .unwrap();
        assert_eq!(launch.classification, "flaky");
        assert_eq!(launch.executions, 3);
        assert_eq!(launch.successes, 2);
        assert_eq!(launch.failures, 1);
    }

    #[test]
    fn one_failure_is_not_flaky() {
        let runs = vec![run("one", RunStatus::Failed, StepStatus::Failed)];
        let report = analyze_flaky(&runs, 3);
        assert_eq!(report.steps[0].classification, "insufficient_evidence");
    }

    fn detailed_step(name: &str, command: &str, details: Value) -> RunStep {
        let mut step = step(name, StepStatus::Success);
        step.name = name.to_string();
        step.command = command.to_string();
        step.details = details;
        step
    }

    fn performance_report(
        id: &str,
        startup_ms: u64,
        janky_percent: f64,
        apk_bytes: u64,
        tests: u64,
        failures: u64,
    ) -> RunReport {
        let mut report = run(id, RunStatus::Passed, StepStatus::Success);
        report.steps.clear();
        report.steps.push(detailed_step(
            "diagnostics performance",
            "diagnostics.performance",
            serde_json::json!({
                "startup": {"total_time_ms": startup_ms},
                "jank": {
                    "reset_before_launch": true,
                    "total_frames": 100,
                    "janky_frames": janky_percent as u64,
                    "janky_percent": janky_percent,
                    "percentile_99_ms": janky_percent * 4.0,
                    "comparable": true,
                    "sample_status": "ok"
                },
                "memory": {"total_pss_kb": startup_ms * 2}
            }),
        ));
        report.steps.push(detailed_step(
            "junit:Smoke",
            "report import junit smoke.xml",
            serde_json::json!({
                "tests": tests,
                "failures": failures,
                "errors": 0,
                "skipped": 1,
            }),
        ));
        report.artifacts.push(ArtifactRef {
            kind: "apk".to_string(),
            path: "app.apk".to_string(),
            sha256: None,
            size_bytes: Some(apk_bytes),
            description: None,
        });
        report
    }

    #[test]
    fn performance_comparison_reports_known_and_named_deltas() {
        let baseline = performance_report("baseline", 500, 5.0, 1_000_000, 100, 1);
        let current = performance_report("current", 620, 7.5, 1_100_000, 120, 2);
        let comparison = compare_reports(&current, &baseline);
        let performance = &comparison.performance;

        assert_eq!(performance.startup_ms.as_ref().unwrap().delta, Some(120.0));
        assert_eq!(performance.jank_percent.as_ref().unwrap().delta, Some(2.5));
        assert_eq!(performance.janky_frames.as_ref().unwrap().delta, Some(2.0));
        assert_eq!(performance.total_frames.as_ref().unwrap().delta, Some(0.0));
        assert_eq!(performance.max_frame_ms.as_ref().unwrap().delta, Some(10.0));
        assert_eq!(
            performance.apk_size_bytes.as_ref().unwrap().delta,
            Some(100_000.0)
        );
        assert_eq!(performance.tests_total.as_ref().unwrap().delta, Some(20.0));
        assert_eq!(performance.tests_failed.as_ref().unwrap().delta, Some(1.0));
        assert_eq!(performance.tests_skipped.as_ref().unwrap().delta, Some(0.0));
        assert_eq!(
            performance
                .metrics
                .get("diagnostics.performance.memory.total_pss_kb")
                .unwrap()
                .delta,
            Some(240.0)
        );
        assert!(comparison.budget.is_none());
        assert!(comparison.budget_checks.is_empty());
        assert!(comparison.budget_violations.is_empty());
        assert_eq!(comparison.budget_passed, None);
    }

    #[test]
    fn budget_passes_when_metrics_stay_within_thresholds() {
        let baseline = performance_report("baseline", 500, 5.0, 1_000_000, 100, 1);
        let current = performance_report("current", 560, 6.0, 1_050_000, 110, 1);
        let budget = PerformanceBudget {
            startup_ms: Some(600.0),
            jank_percent: Some(10.0),
            apk_size_bytes: Some(1_100_000.0),
            failed_tests: Some(2.0),
            startup_delta_ms: Some(100.0),
            max_frame_delta_ms: Some(10.0),
            metrics: BTreeMap::from([(
                "diagnostics.performance.memory.total_pss_kb".to_string(),
                2_000.0,
            )]),
            metric_deltas: BTreeMap::from([(
                "diagnostics.performance.memory.total_pss_kb".to_string(),
                200.0,
            )]),
            ..PerformanceBudget::default()
        };

        let comparison = compare_with_budget(&current, &baseline, &budget);
        assert_eq!(comparison.budget_passed, Some(true));
        assert!(comparison.budget_violations.is_empty());
        assert_eq!(comparison.budget_checks.len(), 8);
        assert!(comparison.budget_checks.iter().all(|check| check.passed));
        assert!(comparison.budget_checks.iter().any(|check| {
            check.metric == "startup_ms"
                && check.actual == Some(560.0)
                && check.delta == Some(60.0)
                && check.basis.as_deref() == Some("absolute")
        }));
        assert!(comparison.budget_checks.iter().any(|check| {
            check.metric == "startup_delta_ms"
                && check.expected == 100.0
                && check.basis.as_deref() == Some("delta")
        }));
        assert!(comparison.budget_checks.iter().any(|check| {
            check.metric == "diagnostics.performance.memory.total_pss_kb" && check.passed
        }));
        assert_eq!(comparison.budget.as_ref(), Some(&budget));
    }

    #[test]
    fn budget_reports_failed_thresholds_as_violations() {
        let baseline = performance_report("baseline", 500, 5.0, 1_000_000, 100, 1);
        let current = performance_report("current", 700, 12.0, 1_400_000, 110, 3);
        let budget = PerformanceBudget {
            startup_ms: Some(600.0),
            jank_percent: Some(10.0),
            apk_size_delta_bytes: Some(200_000.0),
            failed_tests_delta: Some(1.0),
            ..PerformanceBudget::default()
        };

        let comparison = compare_with_budget(&current, &baseline, &budget);
        assert_eq!(comparison.budget_passed, Some(false));
        assert_eq!(comparison.budget_checks.len(), 4);
        assert_eq!(comparison.budget_violations.len(), 4);
        let startup = comparison
            .budget_violations
            .iter()
            .find(|check| check.metric == "startup_ms")
            .unwrap();
        assert_eq!(startup.expected, 600.0);
        assert_eq!(startup.actual, Some(700.0));
        assert_eq!(startup.delta, Some(200.0));
        assert!(!startup.passed);
        assert!(startup.reason.contains("exceeded"));
        let apk = comparison
            .budget_violations
            .iter()
            .find(|check| check.metric == "apk_size_delta_bytes")
            .unwrap();
        assert_eq!(apk.delta, Some(400_000.0));
        assert_eq!(apk.expected, 200_000.0);
        assert_eq!(apk.basis.as_deref(), Some("delta"));
    }

    #[test]
    fn budget_skips_thresholds_for_missing_metrics() {
        let baseline = run("baseline", RunStatus::Passed, StepStatus::Success);
        let mut current = run("current", RunStatus::Passed, StepStatus::Success);
        current.steps.clear();
        current.steps.push(detailed_step(
            "diagnostics performance",
            "diagnostics.performance",
            serde_json::json!({"startup": {"total_time_ms": 900}}),
        ));
        let budget = PerformanceBudget {
            startup_ms: Some(500.0),
            jank_percent: Some(10.0),
            apk_size_bytes: Some(1_000_000.0),
            failed_tests: Some(0.0),
            startup_delta_ms: Some(100.0),
            ..PerformanceBudget::default()
        };

        let comparison = compare_with_budget(&current, &baseline, &budget);
        assert!(comparison.performance.startup_ms.is_some());
        assert!(comparison.performance.jank_percent.is_none());
        assert!(comparison.performance.apk_size_bytes.is_none());
        assert!(comparison.performance.tests_failed.is_none());
        assert_eq!(comparison.budget_checks.len(), 1);
        assert_eq!(comparison.budget_checks[0].metric, "startup_ms");
        assert_eq!(comparison.budget_checks[0].actual, Some(900.0));
        assert_eq!(comparison.budget_checks[0].delta, None);
        assert_eq!(comparison.budget_violations.len(), 1);
        assert_eq!(comparison.budget_passed, Some(false));

        let empty = compare_reports(&current, &baseline);
        assert!(empty.budget_checks.is_empty());
        assert!(empty.budget_violations.is_empty());
    }

    #[test]
    fn non_comparable_runs_skip_performance_and_budget() {
        let baseline = performance_report("baseline", 500, 5.0, 1_000_000, 100, 1);
        let budget = PerformanceBudget {
            startup_ms: Some(100.0),
            ..PerformanceBudget::default()
        };

        let mut project_mismatch = performance_report("current", 700, 12.0, 1_400_000, 110, 3);
        project_mismatch.project = "Other".to_string();
        let comparison = compare_with_budget(&project_mismatch, &baseline, &budget);
        assert!(!comparison.comparable);
        assert!(comparison.performance.is_empty());
        assert!(comparison.budget.is_none());
        assert!(comparison.budget_checks.is_empty());
        assert!(comparison.budget_violations.is_empty());
        assert_eq!(comparison.budget_passed, None);

        let mut command_mismatch = performance_report("current", 700, 12.0, 1_400_000, 110, 3);
        command_mismatch.command = "other.run".to_string();
        let comparison = compare_with_budget(&command_mismatch, &baseline, &budget);
        assert!(!comparison.comparable);
        assert!(comparison.performance.is_empty());
        assert!(comparison.budget_violations.is_empty());
    }

    #[test]
    fn legacy_comparison_json_defaults_budget_fields() {
        let baseline = performance_report("baseline", 500, 5.0, 1_000_000, 100, 1);
        let current = performance_report("current", 700, 12.0, 1_100_000, 110, 3);
        let comparison = compare_reports(&current, &baseline);
        let mut value = serde_json::to_value(&comparison).unwrap();
        let object = value.as_object_mut().unwrap();
        for key in [
            "performance",
            "budget",
            "budget_checks",
            "budget_violations",
            "budget_passed",
        ] {
            object.remove(key);
        }
        let legacy: RunComparison = serde_json::from_value(value).unwrap();
        assert!(legacy.performance.is_empty());
        assert!(legacy.budget.is_none());
        assert!(legacy.budget_checks.is_empty());
        assert!(legacy.budget_violations.is_empty());
        assert_eq!(legacy.budget_passed, None);
        assert_eq!(legacy.duration_delta_ms, comparison.duration_delta_ms);
    }

    #[test]
    fn comparison_payloads_validate_against_analysis_schema() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("schema")
            .join("analysis.schema.json");
        let bytes = std::fs::read(&path).unwrap_or_else(|error| {
            panic!("read {}: {error}", path.display());
        });
        let schema: Value = serde_json::from_slice(&bytes).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let baseline = performance_report("baseline", 500, 5.0, 1_000_000, 100, 1);
        let current = performance_report("current", 700, 12.0, 1_400_000, 110, 3);
        let budget = PerformanceBudget {
            startup_ms: Some(600.0),
            metric_deltas: BTreeMap::from([(
                "diagnostics.performance.memory.total_pss_kb".to_string(),
                100.0,
            )]),
            ..PerformanceBudget::default()
        };

        for comparison in [
            compare_reports(&current, &baseline),
            compare_with_budget(&current, &baseline, &budget),
        ] {
            let value = serde_json::to_value(&comparison).unwrap();
            validator.validate(&value).unwrap_or_else(|error| {
                panic!("schema rejected comparison: {error}\n{value:#}");
            });
        }
        let flaky = analyze_flaky(
            &[
                run("one", RunStatus::Passed, StepStatus::Success),
                run("two", RunStatus::Failed, StepStatus::Failed),
            ],
            2,
        );
        let value = serde_json::to_value(&flaky).unwrap();
        validator
            .validate(&value)
            .unwrap_or_else(|error| panic!("schema rejected flaky report: {error}\n{value:#}"));
    }
}
