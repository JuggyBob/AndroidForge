use std::time::{Duration, Instant};

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alarm_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_epoch_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_iso: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub raw_excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextAlarms {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarms: Vec<AlarmInfo>,
    pub best_effort: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip)]
    pub raw: String,
}

#[derive(Debug, Clone)]
pub struct Alarms {
    pub adb: AdbClient,
}

impl Alarms {
    pub fn new(adb: AdbClient) -> Self {
        Self { adb }
    }

    pub async fn list(&self, serial: &str, package: Option<&str>) -> Result<NextAlarms> {
        let output = self
            .adb
            .shell_args(
                serial,
                &["dumpsys".to_string(), "alarm".to_string()],
                Some(Duration::from_secs(30)),
                Some(16 * 1024 * 1024),
            )
            .await?;
        let raw = output.stdout_text();
        let mut alarms = parse_alarms(&raw);
        if let Some(package) = package {
            alarms.retain(|alarm| alarm.package.as_deref() == Some(package));
        }
        let notes = if alarms.is_empty() {
            vec![
                "No alarms were parsed. AlarmManager output varies by Android version; the raw dump is preserved when diagnostics run."
                    .to_string(),
            ]
        } else {
            Vec::new()
        };
        Ok(NextAlarms {
            alarms,
            best_effort: true,
            notes,
            raw,
        })
    }

    pub async fn wait(
        &self,
        serial: &str,
        package: Option<&str>,
        timeout: Duration,
    ) -> Result<NextAlarms> {
        let started = Instant::now();
        let mut last_notes = Vec::new();
        loop {
            let alarms = self.list(serial, package).await?;
            if !alarms.alarms.is_empty() {
                return Ok(alarms);
            }
            if last_notes.is_empty() {
                last_notes = alarms.notes;
            }
            if started.elapsed() >= timeout {
                return Err(ForgeError::new(
                    ErrorCode::UiAssertionFailed,
                    "no matching alarm was observed before the timeout",
                )
                .with_context(
                    "notes",
                    serde_json::to_value(last_notes).unwrap_or(serde_json::Value::Null),
                )
                .with_suggestion("Inspect raw AlarmManager state with `androidforge diagnostics collect`.")
                .with_suggestion("Some alarms are delivered through inexact batching and may not appear as exact times.")
                .device(serial.to_string()));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

fn parse_alarms(raw: &str) -> Vec<AlarmInfo> {
    let package_regex = Regex::new(r"(?:pkg|package)=([^\s,}\]]+)").expect("alarm package regex");
    let when_regex = Regex::new(r"\b(?:origWhen|when)[= ](\d{10,})").expect("alarm when regex");
    let iso_regex = Regex::new(r#"\borigWhen=(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)?)"#)
        .expect("alarm iso regex");
    let type_regex = Regex::new(r"\b(RTC_WAKEUP|RTC|ELAPSED_REALTIME_WAKEUP|ELAPSED_REALTIME)\b")
        .expect("alarm type regex");
    let tag_regex = Regex::new(r"\btag=([^\s,}\]]+)").expect("alarm tag regex");
    let alarm_regex = Regex::new(r"Alarm\{([^}]*)\}").expect("alarm body regex");
    let mut alarms = Vec::new();
    let mut current_type: Option<String> = None;
    for line in raw.lines() {
        if let Some(captures) = type_regex.captures(line) {
            current_type = captures.get(1).map(|value| value.as_str().to_string());
        }
        if !(line.contains("Alarm{")
            || line.contains("Next wake")
            || line.contains("Pending alarm"))
        {
            continue;
        }
        let package = package_regex
            .captures(line)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().trim_matches('"').to_string())
            .or_else(|| {
                alarm_regex.captures(line).and_then(|captures| {
                    let body = captures.get(1)?.as_str();
                    let candidate = body.split_whitespace().last()?.trim_end_matches('}');
                    if candidate.contains('.')
                        && !candidate.starts_with("type")
                        && candidate != "null"
                    {
                        Some(candidate.to_string())
                    } else {
                        None
                    }
                })
            });
        let when_epoch_ms = when_regex
            .captures(line)
            .and_then(|captures| captures.get(1))
            .and_then(|value| value.as_str().parse::<i64>().ok());
        let when_iso = when_epoch_ms
            .and_then(|millis| {
                DateTime::<Utc>::from_timestamp_millis(millis).map(|datetime| datetime.to_rfc3339())
            })
            .or_else(|| {
                iso_regex
                    .captures(line)
                    .and_then(|captures| captures.get(1))
                    .and_then(|value| {
                        chrono::NaiveDateTime::parse_from_str(
                            value.as_str(),
                            "%Y-%m-%d %H:%M:%S%.f",
                        )
                        .ok()
                        .map(|datetime| datetime.and_utc().to_rfc3339())
                    })
            });
        let alarm_type = type_regex
            .captures(line)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().to_string())
            .or_else(|| current_type.clone());
        if package.is_none() && when_epoch_ms.is_none() && alarm_type.is_none() {
            continue;
        }
        alarms.push(AlarmInfo {
            package,
            alarm_type,
            when_epoch_ms,
            when_iso,
            tag: tag_regex
                .captures(line)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().to_string()),
            raw_excerpt: line.trim().chars().take(2000).collect(),
        });
    }
    alarms.truncate(100);
    alarms
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"
Pending alarm batches: 2
  RTC_WAKEUP #0: Alarm{abc type 0 when 1760000000000 com.example}
    tag=*walarm*:com.example/.Receiver
  ELAPSED_REALTIME #1: Alarm{def type 3 when 12345 com.other}
"#;

    #[test]
    fn parses_android_16_alarm_lines() {
        let dump = r#"
  RTC_WAKEUP #11: Alarm{b71d644 type 0 origWhen 1794754052577 whenElapsed 4559998 com.example.app}
    tag=*walarm*:com.example.app.action.CHECK
    type=RTC_WAKEUP origWhen=2026-11-15 15:47:32.577 window=+44m59s995ms repeatInterval=0
"#;
        let alarms = parse_alarms(dump);
        assert_eq!(alarms.len(), 1);
        assert_eq!(alarms[0].package.as_deref(), Some("com.example.app"));
        assert_eq!(alarms[0].alarm_type.as_deref(), Some("RTC_WAKEUP"));
        assert_eq!(alarms[0].when_epoch_ms, Some(1794754052577));
    }

    #[test]
    fn parses_alarm_lines() {
        let alarms = parse_alarms(DUMP);
        assert!(alarms.len() >= 2);
        assert_eq!(alarms[0].package.as_deref(), Some("com.example"));
        assert_eq!(alarms[0].alarm_type.as_deref(), Some("RTC_WAKEUP"));
        assert_eq!(alarms[0].when_epoch_ms, Some(1760000000000));
        assert!(alarms[0].when_iso.is_some());
    }
}
