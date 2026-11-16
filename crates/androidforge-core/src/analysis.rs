use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

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

pub fn compare_reports(current: &RunReport, baseline: &RunReport) -> RunComparison {
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
}
