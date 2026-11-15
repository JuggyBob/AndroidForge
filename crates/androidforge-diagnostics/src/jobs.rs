use std::time::{Duration, Instant};

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobInfo {
    pub package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
    pub raw_excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobList {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jobs: Vec<JobInfo>,
    pub best_effort: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip)]
    pub raw: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRunResult {
    pub package: String,
    pub job_id: String,
    pub accepted_by_command: bool,
    #[serde(default)]
    pub confirmed: bool,
    pub output: String,
    pub best_effort: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobWaitResult {
    pub matched: bool,
    pub attempts: u32,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<JobInfo>,
}

#[derive(Debug, Clone)]
pub struct Jobs {
    pub adb: AdbClient,
}

impl Jobs {
    pub fn new(adb: AdbClient) -> Self {
        Self { adb }
    }

    pub async fn list(&self, serial: &str, package: Option<&str>) -> Result<JobList> {
        let output = self
            .adb
            .shell_args(
                serial,
                &["dumpsys".to_string(), "jobscheduler".to_string()],
                Some(Duration::from_secs(30)),
                Some(16 * 1024 * 1024),
            )
            .await?;
        let raw = output.stdout_text();
        let mut list = parse_jobs(&raw);
        if let Some(package) = package {
            list.jobs.retain(|job| job.package == package);
        }
        if list.jobs.is_empty() {
            list.notes.push(
                "No jobs were parsed. JobScheduler dumps vary by Android version; inspect the raw dump or use the app hook contract for deterministic triggering."
                    .to_string(),
            );
        }
        list.raw = raw;
        Ok(list)
    }

    pub async fn run(&self, serial: &str, package: &str, job_id: &str) -> Result<JobRunResult> {
        let output = self
            .adb
            .shell_args(
                serial,
                &[
                    "cmd".to_string(),
                    "jobscheduler".to_string(),
                    "run".to_string(),
                    "-f".to_string(),
                    package.to_string(),
                    job_id.to_string(),
                ],
                Some(Duration::from_secs(30)),
                Some(2 * 1024 * 1024),
            )
            .await?;
        let text = output.combined_text();
        let lower = text.to_ascii_lowercase();
        if lower.contains("no such job")
            || lower.contains("invalid")
            || lower.contains("error")
            || lower.contains("exception")
        {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                format!("JobScheduler rejected job {job_id} for {package}"),
            )
            .with_context_string("output", truncate(text.trim(), 3000))
            .with_suggestion("Run `androidforge jobs list` to discover the current job ID.")
            .with_suggestion(
                "Prefer an app-provided debug hook for deterministic background-work tests.",
            )
            .device(serial.to_string()));
        }
        let confirmed = lower.contains("running job") || lower.contains("job scheduled");
        Ok(JobRunResult {
            package: package.to_string(),
            job_id: job_id.to_string(),
            accepted_by_command: true,
            confirmed,
            output: truncate(text.trim(), 4000),
            best_effort: true,
            notes: vec![
                "JobScheduler IDs are dynamic; AndroidForge records the command result but does not assume the job completed."
                    .to_string(),
            ],
        })
    }

    pub async fn cancel(&self, serial: &str, package: &str, job_id: &str) -> Result<JobRunResult> {
        let output = self
            .adb
            .shell_args(
                serial,
                &[
                    "cmd".to_string(),
                    "jobscheduler".to_string(),
                    "cancel".to_string(),
                    package.to_string(),
                    job_id.to_string(),
                ],
                Some(Duration::from_secs(20)),
                Some(2 * 1024 * 1024),
            )
            .await?;
        let text = output.combined_text();
        Ok(JobRunResult {
            package: package.to_string(),
            job_id: job_id.to_string(),
            accepted_by_command: true,
            confirmed: false,
            output: truncate(text.trim(), 4000),
            best_effort: true,
            notes: vec!["`cmd jobscheduler cancel` support varies by Android version.".to_string()],
        })
    }

    pub async fn wait(
        &self,
        serial: &str,
        selector: &JobSelector,
        timeout: Duration,
    ) -> Result<JobWaitResult> {
        let started = Instant::now();
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            let list = self.list(serial, selector.package.as_deref()).await?;
            if let Some(job) = list.jobs.iter().find(|job| job_matches(job, selector)) {
                return Ok(JobWaitResult {
                    matched: true,
                    attempts,
                    duration_ms: clamp_ms(started.elapsed()),
                    job: Some(job.clone()),
                });
            }
            if started.elapsed() >= timeout {
                return Err(ForgeError::new(
                    ErrorCode::UiAssertionFailed,
                    "no JobScheduler job matched the selector before the timeout",
                )
                .with_context(
                    "selector",
                    serde_json::to_value(selector).unwrap_or(serde_json::Value::Null),
                )
                .with_context(
                    "jobs",
                    serde_json::to_value(&list.jobs).unwrap_or(serde_json::Value::Null),
                )
                .with_suggestion("Run `androidforge jobs list --package <package>`.")
                .with_suggestion(
                    "Use an app-provided debug hook for deterministic background tests.",
                )
                .device(serial.to_string()));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

fn job_matches(job: &JobInfo, selector: &JobSelector) -> bool {
    if let Some(package) = &selector.package {
        if &job.package != package {
            return false;
        }
    }
    if let Some(job_id) = &selector.job_id {
        if job.job_id.as_deref() != Some(job_id.as_str()) {
            return false;
        }
    }
    if let Some(state) = &selector.state {
        match state.as_str() {
            "running" => return job.running == Some(true),
            "ready" => return job.ready == Some(true),
            "pending" => return job.pending == Some(true),
            _ => {}
        }
    }
    true
}

fn parse_jobs(raw: &str) -> JobList {
    // Android 16 has two header shapes:
    //   JOB #u0a123/42: com.example/.Job
    //   JOB usagestats_mapping:1000/0: f00e75f @tag@com.example/.Job
    let job_regex = Regex::new(r"^\s*JOB\s+(\S+)/(\d+):\s*(.*)$").expect("jobscheduler job regex");
    let package_regex = Regex::new(r"\bpkg=([^\s,}\]]+)").expect("jobscheduler package regex");
    let constraint_regex = Regex::new(r"(?:Required|Satisfied|Unsatisfied) constraints:\s*(.*)")
        .expect("constraint regex");
    let mut jobs = Vec::new();
    let mut current: Option<JobInfo> = None;
    let mut block = Vec::new();

    let flush =
        |current: &mut Option<JobInfo>, block: &mut Vec<String>, jobs: &mut Vec<JobInfo>| {
            if let Some(mut job) = current.take() {
                job.raw_excerpt = block.join("\n").chars().take(4000).collect();
                jobs.push(job);
            }
            block.clear();
        };

    for line in raw.lines() {
        if let Some(captures) = job_regex.captures(line) {
            flush(&mut current, &mut block, &mut jobs);
            block.push(line.to_string());
            let job_id = captures.get(2).map(|value| value.as_str().to_string());
            let rest = captures.get(3).map(|value| value.as_str()).unwrap_or("");
            let mut tokens = rest.split_whitespace();
            let first = tokens.next();
            let second = tokens.next();
            let looks_like_component = |value: &str| value.contains('/') && !value.starts_with('@');
            let component = first
                .filter(|value| looks_like_component(value))
                .or_else(|| second.filter(|value| looks_like_component(value)))
                .map(str::to_string)
                .unwrap_or_default();
            let package = component.split('/').next().unwrap_or("").to_string();
            current = Some(JobInfo {
                package,
                job_id,
                service: if component.is_empty() {
                    None
                } else {
                    Some(component)
                },
                ready: None,
                running: None,
                pending: None,
                constraints: Vec::new(),
                raw_excerpt: String::new(),
            });
        } else if current.is_some() {
            block.push(line.to_string());
            if let Some(job) = current.as_mut() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("Ready:") {
                    job.ready = rest.trim().parse::<bool>().ok();
                }
                if let Some(rest) = trimmed.strip_prefix("Running:") {
                    job.running = rest.trim().parse::<bool>().ok();
                }
                if let Some(rest) = trimmed.strip_prefix("Pending:") {
                    job.pending = rest.trim().parse::<bool>().ok();
                }
                if job.package.is_empty() {
                    if let Some(captures) = package_regex.captures(trimmed) {
                        if let Some(value) = captures.get(1) {
                            job.package = value.as_str().trim_matches('"').to_string();
                        }
                    }
                }
                if job.service.is_none() {
                    if let Some(rest) = trimmed.strip_prefix("Service:") {
                        let service = rest.trim();
                        if !service.is_empty() {
                            job.service = Some(service.to_string());
                        }
                    }
                }
                if let Some(captures) = constraint_regex.captures(trimmed) {
                    if let Some(value) = captures.get(1) {
                        let value = value.as_str().trim();
                        if !value.is_empty() {
                            job.constraints.push(value.to_string());
                        }
                    }
                }
            }
        }
    }
    flush(&mut current, &mut block, &mut jobs);
    JobList {
        jobs,
        best_effort: true,
        notes: vec![
            "JobScheduler output is parsed best-effort and the raw dump remains the source of truth."
                .to_string(),
        ],
        raw: String::new(),
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect::<String>() + "\n[truncated]"
    }
}

fn clamp_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"
JOB #u0a123/42: com.example/.SyncJobService
  Required constraints: CONNECTIVITY
  Satisfied constraints: CONNECTIVITY
  Ready: true
  Pending: false
  Running: false
JOB #u0a123/77: com.other/.Job
  Ready: false
"#;

    #[test]
    fn parses_jobscheduler_blocks() {
        let list = parse_jobs(DUMP);
        assert_eq!(list.jobs.len(), 2);
        let job = &list.jobs[0];
        assert_eq!(job.package, "com.example");
        assert_eq!(job.job_id.as_deref(), Some("42"));
        assert_eq!(job.ready, Some(true));
        assert_eq!(job.running, Some(false));
        assert!(
            job.constraints
                .iter()
                .any(|value| value.contains("CONNECTIVITY"))
        );
    }

    #[test]
    fn parses_android_16_job_headers_and_package_source() {
        let dump = r#"
Registered jobs:
  JOB usagestats_mapping:1000/0: f00e75f @usagestats_mapping@android/com.android.server.usage.UsageStatsIdleService
    Source: uid=1000 user=0 pkg=android
    Ready: false
  JOB #10217/900: 4e7619e com.example/.SyncJobService
    Source: uid=10217 user=0 pkg=com.example
    Ready: true
"#;
        let list = parse_jobs(dump);
        assert_eq!(list.jobs.len(), 2);
        assert_eq!(list.jobs[0].package, "android");
        assert_eq!(list.jobs[0].job_id.as_deref(), Some("0"));
        assert_eq!(list.jobs[1].package, "com.example");
        assert_eq!(list.jobs[1].job_id.as_deref(), Some("900"));
        assert_eq!(list.jobs[1].ready, Some(true));
    }

    #[test]
    fn job_selector_filters_state() {
        let list = parse_jobs(DUMP);
        let selector = JobSelector {
            package: Some("com.example".to_string()),
            state: Some("ready".to_string()),
            ..JobSelector::default()
        };
        assert!(list.jobs.iter().any(|job| job_matches(job, &selector)));
    }
}
