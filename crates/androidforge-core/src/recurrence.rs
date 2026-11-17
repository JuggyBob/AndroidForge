//! Local failure recurrence memory.
//!
//! Recurrence is derived from the reports that already exist in the project
//! artifact store. There is no second database: every query reads the same
//! `report.json` files used by `report show` and `report explain`. This keeps
//! historical data authoritative and avoids a migration that could lose runs.
//!
//! Comparability is intentionally conservative. A passing run only proves that
//! a failure was resolved when:
//!
//! 1. it belongs to the same project scope and scenario namespace,
//! 2. it ran after the failure,
//! 3. it contains the same failing step and that step succeeded, and
//! 4. the package identity is either identical or unavailable in one report.
//!
//! If those conditions cannot be established, the failure is reported as
//! `unknown` instead of being declared fixed.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::diagnosis::{DiagnosisCategory, DiagnosticExplanation, explain_report};
use crate::run::{RunReport, RunStatus, RunStep, StepStatus};

pub const RECURRENCE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceStatus {
    New,
    Recurring,
    Regressed,
    Unknown,
}

impl RecurrenceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Recurring => "recurring",
            Self::Regressed => "regressed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecurrenceOccurrence {
    pub run_id: String,
    pub started_at: String,
    pub run_status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apk_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<RecurrenceEvidence>,
    /// A covered success occurred between this occurrence and the next one.
    #[serde(default)]
    pub followed_by_covered_success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecurrenceEvidence {
    pub run_id: String,
    pub source: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FingerprintRecurrence {
    pub fingerprint: String,
    pub fingerprint_version: u32,
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub project_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    pub first_seen: String,
    pub last_seen: String,
    pub occurrence_count: usize,
    pub run_ids: Vec<String>,
    pub status: RecurrenceStatus,
    /// The last occurrence is followed by a comparable passing run.
    pub currently_resolved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by_run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub app_versions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub occurrences: Vec<RecurrenceOccurrence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncertainty: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecurrenceHistory {
    pub schema_version: u32,
    pub scanned_runs: usize,
    pub failed_runs: usize,
    pub unreadable_runs: usize,
    pub fingerprints: Vec<FingerprintRecurrence>,
}

#[derive(Debug, Clone, Default)]
pub struct RecurrenceFilter {
    pub project_root: Option<String>,
    pub package: Option<String>,
    pub fingerprint: Option<String>,
    pub device: Option<String>,
    pub app_version: Option<String>,
    pub since_rfc3339: Option<String>,
    pub status: Option<RecurrenceStatus>,
}

struct ObservedFailure {
    explanation: DiagnosticExplanation,
    occurrence: RecurrenceOccurrence,
    comparable: bool,
    failed_step_name: Option<String>,
    scenario: String,
    project_root: String,
    package: Option<String>,
}

/// Build recurrence memory from in-memory reports. Reports are ordered by
/// `started_at` and then `run_id`, so the result is deterministic even when the
/// caller's input order changes.
pub fn build_history(reports: &[RunReport], unreadable_runs: usize) -> RecurrenceHistory {
    let mut ordered = reports
        .iter()
        .enumerate()
        .filter(|(_, report)| report.status.is_terminal() && report.status != RunStatus::Passed)
        .collect::<Vec<_>>();
    ordered.sort_by(|(left_index, left), (right_index, right)| {
        left.started_at
            .cmp(&right.started_at)
            .then_with(|| left.run_id.cmp(&right.run_id))
            .then_with(|| left_index.cmp(right_index))
    });

    let mut groups: BTreeMap<String, Vec<ObservedFailure>> = BTreeMap::new();
    for (_, report) in &ordered {
        let explanation = explain_report(report);
        let failed_step = failing_step(report);
        let comparable = scenario_is_comparable(report, failed_step.as_ref())
            && explanation.diagnosis.category != DiagnosisCategory::Unknown;
        let failed_step_name = failed_step.as_ref().map(|step| step.name.clone());
        let occurrence = occurrence_from(report, &explanation, failed_step);
        groups
            .entry(format!(
                "{}|{}",
                canonical_scope(&report.project_root),
                explanation.fingerprint
            ))
            .or_default()
            .push(ObservedFailure {
                scenario: report.command.clone(),
                project_root: report.project_root.clone(),
                package: explanation.package.clone(),
                explanation,
                occurrence,
                comparable,
                failed_step_name,
            });
    }

    let passing = reports
        .iter()
        .filter(|report| report.status == RunStatus::Passed)
        .collect::<Vec<_>>();

    let mut fingerprints = groups
        .into_values()
        .map(|failures| {
            let fingerprint = failures[0].explanation.fingerprint.clone();
            assemble(fingerprint, failures, &passing)
        })
        .collect::<Vec<_>>();
    fingerprints.sort_by(|left, right| {
        right
            .last_seen
            .cmp(&left.last_seen)
            .then_with(|| left.fingerprint.cmp(&right.fingerprint))
    });

    RecurrenceHistory {
        schema_version: RECURRENCE_SCHEMA_VERSION,
        scanned_runs: reports.len(),
        failed_runs: ordered.len(),
        unreadable_runs,
        fingerprints,
    }
}

fn assemble(
    fingerprint: String,
    mut failures: Vec<ObservedFailure>,
    passing: &[&RunReport],
) -> FingerprintRecurrence {
    failures.sort_by(|left, right| {
        left.occurrence
            .started_at
            .cmp(&right.occurrence.started_at)
            .then_with(|| left.occurrence.run_id.cmp(&right.occurrence.run_id))
    });

    let first_fingerprint_version = failures[0].explanation.fingerprint_version;
    let category = failures[0]
        .explanation
        .diagnosis
        .category
        .as_str()
        .to_string();
    let code = failures[0].explanation.diagnosis.code.clone();
    let project_root = canonical_scope(&failures[0].explanation.project_root);
    let package = failures[0].explanation.package.clone();
    let first_seen = failures[0].occurrence.started_at.clone();
    let mut status = if failures.iter().all(|failure| !failure.comparable) {
        RecurrenceStatus::Unknown
    } else if failures.len() == 1 {
        RecurrenceStatus::New
    } else {
        RecurrenceStatus::Recurring
    };
    let mut uncertainty = Vec::new();
    for failure in &failures {
        for line in &failure.explanation.uncertainty {
            if !uncertainty.contains(line) {
                uncertainty.push(line.clone());
            }
        }
    }

    let mut run_ids = Vec::new();
    let mut devices = BTreeSet::new();
    let mut app_versions = BTreeSet::new();
    let mut evidence = Vec::new();
    let mut regressed = false;

    for index in 0..failures.len() {
        let coverage = failures.get(index + 1).and_then(|next| {
            if !failures[index].comparable {
                return None;
            }
            let occurrence = &failures[index].occurrence;
            covering_success(
                &failures[index],
                &occurrence.run_id,
                &occurrence.started_at,
                Some(&next.occurrence.started_at),
                passing,
            )
            .map(|_| index)
        });
        {
            let occurrence = &mut failures[index].occurrence;
            if !run_ids.contains(&occurrence.run_id) {
                run_ids.push(occurrence.run_id.clone());
            }
            if let Some(device) = &occurrence.device_serial {
                devices.insert(device.clone());
            }
            if let Some(version) = app_version(occurrence) {
                app_versions.insert(version);
            }
            for item in &occurrence.evidence {
                if evidence.len() < 24 && !evidence.contains(item) {
                    evidence.push(item.clone());
                }
            }
            if coverage.is_some() {
                occurrence.followed_by_covered_success = true;
                regressed = true;
            }
        }
    }

    if regressed && status == RecurrenceStatus::Recurring {
        status = RecurrenceStatus::Regressed;
    }
    if status == RecurrenceStatus::Regressed {
        uncertainty.push(
            "A comparable passing run exercised the same step between two failures, so the later failure is classified as regressed."
                .to_string(),
        );
    } else if failures.len() == 1 {
        uncertainty.push(
            "Only one occurrence is stored. A single run cannot establish whether the failure is intermittent."
                .to_string(),
        );
    }

    let last = failures.last().expect("group is non-empty");
    let (resolved_by_run, resolved_at) = if last.comparable {
        covering_success(
            last,
            &last.occurrence.run_id,
            &last.occurrence.started_at,
            None,
            passing,
        )
        .map(|run| (Some(run.run_id.clone()), Some(run.started_at.clone())))
        .unwrap_or((None, None))
    } else {
        (None, None)
    };
    let currently_resolved = resolved_by_run.is_some();
    if currently_resolved {
        uncertainty.push(
            "A later comparable passing run exercised the same step; this fingerprint currently appears resolved."
                .to_string(),
        );
    }

    FingerprintRecurrence {
        fingerprint,
        fingerprint_version: first_fingerprint_version,
        category,
        code,
        project_root,
        package: package.or_else(|| failures[0].occurrence.package.clone()),
        first_seen,
        last_seen: last.occurrence.started_at.clone(),
        occurrence_count: failures.len(),
        run_ids,
        status,
        currently_resolved,
        resolved_by_run,
        resolved_at,
        devices: devices.into_iter().collect(),
        app_versions: app_versions.into_iter().collect(),
        occurrences: failures
            .into_iter()
            .map(|failure| failure.occurrence)
            .collect(),
        uncertainty,
    }
}

fn app_version(occurrence: &RecurrenceOccurrence) -> Option<String> {
    match (&occurrence.version_name, occurrence.version_code) {
        (Some(name), Some(code)) => Some(format!("{name} ({code})")),
        (Some(name), None) => Some(name.clone()),
        (None, Some(code)) => Some(code.to_string()),
        (None, None) => occurrence
            .apk_sha256
            .as_ref()
            .map(|sha| format!("sha256:{sha}")),
    }
}

fn canonical_scope(value: &str) -> String {
    let normalized = value.replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    if cfg!(windows) {
        trimmed.to_ascii_lowercase()
    } else {
        trimmed.to_string()
    }
}

fn occurrence_from(
    report: &RunReport,
    explanation: &DiagnosticExplanation,
    failed_step: Option<RunStep>,
) -> RecurrenceOccurrence {
    let device = report.device.as_ref();
    RecurrenceOccurrence {
        run_id: report.run_id.clone(),
        started_at: report.started_at.clone(),
        run_status: report.status,
        failed_step: failed_step.as_ref().map(|step| step.name.clone()),
        failed_operation: explanation.diagnosis.failed_operation.clone(),
        code: explanation.diagnosis.code.clone(),
        device_serial: report.device_serial.clone(),
        device_model: device.and_then(|device| device.model.clone()),
        api_level: device.and_then(|device| device.api_level),
        package: explanation.package.clone(),
        version_name: report.build.version_name.clone(),
        version_code: report.build.version_code,
        apk_sha256: report.build.apk_sha256.clone(),
        evidence: explanation
            .evidence
            .iter()
            .take(12)
            .map(|evidence| RecurrenceEvidence {
                run_id: report.run_id.clone(),
                source: evidence.source.clone(),
                description: evidence.description.clone(),
            })
            .collect(),
        followed_by_covered_success: false,
    }
}

fn failing_step(report: &RunReport) -> Option<RunStep> {
    if let Some(failure) = &report.failure
        && let Some(step) = report.steps.iter().find(|step| {
            step.failure.as_ref().is_some_and(|candidate| {
                candidate.code == failure.code
                    && candidate.failed_operation == failure.failed_operation
            })
        })
    {
        return Some(step.clone());
    }
    report
        .steps
        .iter()
        .find(|step| step.status == StepStatus::Failed)
        .cloned()
}

fn scenario_is_comparable(report: &RunReport, failed_step: Option<&RunStep>) -> bool {
    if report.project_root.trim().is_empty() || report.command.trim().is_empty() {
        return false;
    }
    report.failure.is_some() || failed_step.is_some()
}

/// A successful run covers a failure when it has the same project scope,
/// scenario namespace and package, ran after the failure, and the failing step
/// itself is present and successful. A success without a matching step never
/// proves the scenario was exercised.
fn covering_success<'a>(
    failure: &ObservedFailure,
    failed_run_id: &str,
    failed_at: &str,
    before: Option<&str>,
    passing: &[&'a RunReport],
) -> Option<&'a RunReport> {
    let scope = canonical_scope(&failure.project_root);
    let mut candidates = passing
        .iter()
        .copied()
        .filter(|candidate| {
            canonical_scope(&candidate.project_root) == scope
                && candidate.command == failure.scenario
                && package_compatible(failure.package.as_deref(), candidate)
                && (candidate.started_at.as_str() > failed_at
                    || (candidate.started_at == failed_at
                        && candidate.run_id.as_str() > failed_run_id))
                && before.is_none_or(|limit| candidate.started_at.as_str() < limit)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.started_at
            .cmp(&right.started_at)
            .then_with(|| left.run_id.cmp(&right.run_id))
    });
    candidates.into_iter().find(|candidate| {
        let Some(failed_step) = &failure.failed_step_name else {
            return !candidate.steps.is_empty()
                && candidate
                    .steps
                    .iter()
                    .all(|step| step.status != StepStatus::Failed);
        };
        candidate.steps.iter().any(|step| {
            step.name == *failed_step
                && step.status == StepStatus::Success
                && step.failure.is_none()
        })
    })
}

fn package_compatible(package: Option<&str>, candidate: &RunReport) -> bool {
    match (package, candidate.build.package.as_deref()) {
        (Some(expected), Some(actual)) => expected == actual,
        _ => true,
    }
}

pub fn filter_history(history: &RecurrenceHistory, filter: &RecurrenceFilter) -> RecurrenceHistory {
    let scope = filter.project_root.as_deref().map(canonical_scope);
    let mut fingerprints = history
        .fingerprints
        .iter()
        .filter(|entry| {
            if let Some(scope) = &scope
                && &entry.project_root != scope
            {
                return false;
            }
            if let Some(package) = &filter.package
                && entry.package.as_deref() != Some(package.as_str())
            {
                return false;
            }
            if let Some(fingerprint) = &filter.fingerprint
                && &entry.fingerprint != fingerprint
            {
                return false;
            }
            if let Some(status) = filter.status
                && entry.status != status
            {
                return false;
            }
            if let Some(device) = &filter.device
                && !entry.devices.iter().any(|value| value == device)
            {
                return false;
            }
            if let Some(version) = &filter.app_version
                && !entry.app_versions.iter().any(|value| value == version)
            {
                return false;
            }
            if let Some(since) = &filter.since_rfc3339
                && entry.last_seen.as_str() < since.as_str()
            {
                return false;
            }
            true
        })
        .cloned()
        .collect::<Vec<_>>();
    fingerprints.sort_by(|left, right| {
        right
            .last_seen
            .cmp(&left.last_seen)
            .then_with(|| left.fingerprint.cmp(&right.fingerprint))
    });
    RecurrenceHistory {
        schema_version: history.schema_version,
        scanned_runs: history.scanned_runs,
        failed_runs: history.failed_runs,
        unreadable_runs: history.unreadable_runs,
        fingerprints,
    }
}

pub fn find_fingerprint<'a>(
    history: &'a RecurrenceHistory,
    fingerprint: &str,
) -> Option<&'a FingerprintRecurrence> {
    history
        .fingerprints
        .iter()
        .find(|entry| entry.fingerprint == fingerprint)
}

/// Find a fingerprint inside one project scope. Fingerprints are intentionally
/// project-scoped so identical packages in two projects are never merged.
pub fn find_fingerprint_in_project<'a>(
    history: &'a RecurrenceHistory,
    fingerprint: &str,
    project_root: &str,
) -> Option<&'a FingerprintRecurrence> {
    let scope = canonical_scope(project_root);
    history
        .fingerprints
        .iter()
        .find(|entry| entry.fingerprint == fingerprint && entry.project_root == scope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::run::{FailureInfo, RunStep};

    fn report(
        run_id: &str,
        started_at: &str,
        status: RunStatus,
        step_name: &str,
        step_status: StepStatus,
        version_code: i64,
    ) -> RunReport {
        let mut report =
            RunReport::new(run_id, "conformance.run", "Example", "A:/Projects/Example");
        report.started_at = started_at.to_string();
        report.status = status;
        report.build.package = Some("com.example".to_string());
        report.build.version_name = Some("1.0".to_string());
        report.build.version_code = Some(version_code);
        report.device_serial = Some("emulator-5554".to_string());
        let mut step = RunStep {
            id: "step-001".to_string(),
            name: step_name.to_string(),
            command: format!("conformance {step_name}"),
            status: step_status,
            exit_code: Some(if step_status == StepStatus::Failed {
                7
            } else {
                0
            }),
            started_at: started_at.to_string(),
            ended_at: None,
            duration_ms: 10,
            failure: None,
            artifacts: Vec::new(),
            details: serde_json::json!({}),
            retries: 0,
        };
        if step_status == StepStatus::Failed {
            let failure = FailureInfo {
                code: ErrorCode::UiAssertionFailed,
                message: "expected dialog did not appear".to_string(),
                failed_operation: step_name.to_string(),
                recoverable: true,
                suggestions: Vec::new(),
                context: Default::default(),
                artifacts: Vec::new(),
            };
            step.failure = Some(failure.clone());
            report.failure = Some(failure);
        }
        report.steps.push(step);
        report
    }

    #[test]
    fn first_occurrence_is_new() {
        let reports = vec![report(
            "run-1",
            "2026-01-01T00:00:00Z",
            RunStatus::Failed,
            "conformance:release_smoke",
            StepStatus::Failed,
            1,
        )];
        let history = build_history(&reports, 0);
        assert_eq!(history.fingerprints.len(), 1);
        assert_eq!(
            history.fingerprints[0].status,
            RecurrenceStatus::New,
            "{:?}",
            history.fingerprints[0]
        );
        assert!(!history.fingerprints[0].currently_resolved);
    }

    #[test]
    fn repeated_failure_without_success_is_recurring() {
        let reports = vec![
            report(
                "run-1",
                "2026-01-01T00:00:00Z",
                RunStatus::Failed,
                "conformance:release_smoke",
                StepStatus::Failed,
                1,
            ),
            report(
                "run-2",
                "2026-01-02T00:00:00Z",
                RunStatus::Failed,
                "conformance:release_smoke",
                StepStatus::Failed,
                1,
            ),
        ];
        let history = build_history(&reports, 0);
        assert_eq!(history.fingerprints[0].status, RecurrenceStatus::Recurring);
        assert_eq!(history.fingerprints[0].occurrence_count, 2);
    }

    #[test]
    fn genuine_fixed_then_returned_is_regressed() {
        let reports = vec![
            report(
                "run-1",
                "2026-01-01T00:00:00Z",
                RunStatus::Failed,
                "conformance:release_smoke",
                StepStatus::Failed,
                1,
            ),
            report(
                "run-2",
                "2026-01-02T00:00:00Z",
                RunStatus::Passed,
                "conformance:release_smoke",
                StepStatus::Success,
                1,
            ),
            report(
                "run-3",
                "2026-01-03T00:00:00Z",
                RunStatus::Failed,
                "conformance:release_smoke",
                StepStatus::Failed,
                1,
            ),
        ];
        let history = build_history(&reports, 0);
        assert_eq!(history.fingerprints[0].status, RecurrenceStatus::Regressed);
        assert!(history.fingerprints[0].occurrences[0].followed_by_covered_success);
    }

    #[test]
    fn unrelated_success_does_not_mark_resolved() {
        let reports = vec![
            report(
                "run-1",
                "2026-01-01T00:00:00Z",
                RunStatus::Failed,
                "conformance:release_smoke",
                StepStatus::Failed,
                1,
            ),
            report(
                "run-2",
                "2026-01-02T00:00:00Z",
                RunStatus::Passed,
                "conformance:another_flow",
                StepStatus::Success,
                1,
            ),
        ];
        let history = build_history(&reports, 0);
        assert!(!history.fingerprints[0].currently_resolved);
        assert_eq!(history.fingerprints[0].status, RecurrenceStatus::New);
    }

    #[test]
    fn success_without_the_failing_step_does_not_mark_resolved() {
        let mut success = report(
            "run-2",
            "2026-01-02T00:00:00Z",
            RunStatus::Passed,
            "conformance:release_smoke",
            StepStatus::Success,
            1,
        );
        success.steps[0].name = "conformance:other_flow".to_string();
        let reports = vec![
            report(
                "run-1",
                "2026-01-01T00:00:00Z",
                RunStatus::Failed,
                "conformance:release_smoke",
                StepStatus::Failed,
                1,
            ),
            success,
        ];
        let history = build_history(&reports, 0);
        assert!(!history.fingerprints[0].currently_resolved);
    }

    #[test]
    fn success_after_last_failure_marks_resolved_without_regression() {
        let reports = vec![
            report(
                "run-1",
                "2026-01-01T00:00:00Z",
                RunStatus::Failed,
                "conformance:release_smoke",
                StepStatus::Failed,
                1,
            ),
            report(
                "run-2",
                "2026-01-02T00:00:00Z",
                RunStatus::Passed,
                "conformance:release_smoke",
                StepStatus::Success,
                1,
            ),
        ];
        let history = build_history(&reports, 0);
        assert!(history.fingerprints[0].currently_resolved);
        assert_eq!(
            history.fingerprints[0].resolved_by_run.as_deref(),
            Some("run-2")
        );
    }

    #[test]
    fn different_project_scope_is_isolated() {
        let failed = report(
            "run-1",
            "2026-01-01T00:00:00Z",
            RunStatus::Failed,
            "conformance:release_smoke",
            StepStatus::Failed,
            1,
        );
        let mut success = report(
            "run-2",
            "2026-01-02T00:00:00Z",
            RunStatus::Passed,
            "conformance:release_smoke",
            StepStatus::Success,
            1,
        );
        success.project_root = "A:/Projects/Other".to_string();
        let history = build_history(&[failed, success], 0);
        assert!(!history.fingerprints[0].currently_resolved);
        let filtered = filter_history(
            &history,
            &RecurrenceFilter {
                project_root: Some("A:/Projects/Example".to_string()),
                ..RecurrenceFilter::default()
            },
        );
        assert_eq!(filtered.fingerprints.len(), 1);
    }

    #[test]
    fn version_change_is_retained() {
        let reports = vec![
            report(
                "run-1",
                "2026-01-01T00:00:00Z",
                RunStatus::Failed,
                "conformance:release_smoke",
                StepStatus::Failed,
                1,
            ),
            report(
                "run-2",
                "2026-01-02T00:00:00Z",
                RunStatus::Failed,
                "conformance:release_smoke",
                StepStatus::Failed,
                2,
            ),
        ];
        let history = build_history(&reports, 0);
        assert_eq!(
            history.fingerprints[0].app_versions,
            vec!["1.0 (1)".to_string(), "1.0 (2)".to_string()]
        );
    }

    #[test]
    fn identical_failures_in_two_projects_stay_separate() {
        let first = report(
            "run-1",
            "2026-01-01T00:00:00Z",
            RunStatus::Failed,
            "conformance:release_smoke",
            StepStatus::Failed,
            1,
        );
        let mut second = first.clone();
        second.run_id = "run-2".to_string();
        second.started_at = "2026-01-02T00:00:00Z".to_string();
        second.project_root = "A:/Projects/Other".to_string();
        let history = build_history(&[first, second], 0);
        assert_eq!(history.fingerprints.len(), 2);
        let fingerprint = history.fingerprints[0].fingerprint.clone();
        assert!(
            find_fingerprint_in_project(&history, &fingerprint, "A:/Projects/Example").is_some()
        );
        assert!(find_fingerprint_in_project(&history, &fingerprint, "A:/Projects/Other").is_some());
    }

    #[test]
    fn filter_by_status_and_fingerprint_is_deterministic() {
        let reports = vec![
            report(
                "run-1",
                "2026-01-01T00:00:00Z",
                RunStatus::Failed,
                "conformance:release_smoke",
                StepStatus::Failed,
                1,
            ),
            report(
                "run-2",
                "2026-01-02T00:00:00Z",
                RunStatus::Passed,
                "conformance:release_smoke",
                StepStatus::Success,
                1,
            ),
            report(
                "run-3",
                "2026-01-03T00:00:00Z",
                RunStatus::Failed,
                "conformance:release_smoke",
                StepStatus::Failed,
                1,
            ),
        ];
        let history = build_history(&reports, 0);
        let fingerprint = history.fingerprints[0].fingerprint.clone();
        let filtered = filter_history(
            &history,
            &RecurrenceFilter {
                fingerprint: Some(fingerprint.clone()),
                status: Some(RecurrenceStatus::Regressed),
                ..RecurrenceFilter::default()
            },
        );
        assert_eq!(filtered.fingerprints.len(), 1);
        assert_eq!(filtered.fingerprints[0].fingerprint, fingerprint);
    }
}
