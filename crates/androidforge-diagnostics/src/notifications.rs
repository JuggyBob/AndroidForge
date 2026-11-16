use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_ui::{Selector, UiManager};
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub raw_excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationList {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notifications: Vec<NotificationInfo>,
    pub best_effort: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channels: BTreeMap<String, Option<String>>,
    #[serde(skip)]
    pub raw: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_contains: Option<String>,
}

impl NotificationSelector {
    pub fn matches(&self, notification: &NotificationInfo) -> bool {
        if let Some(package) = &self.package
            && notification.package.as_deref() != Some(package.as_str())
        {
            return false;
        }
        if let Some(tag) = &self.tag
            && notification.tag.as_deref() != Some(tag.as_str())
        {
            return false;
        }
        if let Some(channel) = &self.channel
            && notification.channel.as_deref() != Some(channel.as_str())
        {
            return false;
        }
        if let Some(id) = self.id
            && notification.id != Some(id)
        {
            return false;
        }
        if let Some(title) = &self.title_contains
            && !notification
                .title
                .as_deref()
                .map(|value| value.contains(title))
                .unwrap_or(false)
        {
            return false;
        }
        if let Some(text) = &self.text_contains
            && !notification
                .text
                .as_deref()
                .map(|value| value.contains(text))
                .unwrap_or(false)
        {
            return false;
        }
        true
    }

    pub fn has_conditions(&self) -> bool {
        self.package.is_some()
            || self.tag.is_some()
            || self.channel.is_some()
            || self.id.is_some()
            || self.title_contains.is_some()
            || self.text_contains.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationWaitResult {
    pub present: bool,
    pub matched: bool,
    pub attempts: u32,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification: Option<NotificationInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationChannel {
    pub package: Option<String>,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importance: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationChannels {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<NotificationChannel>,
    pub best_effort: bool,
}

#[derive(Debug, Clone)]
pub struct Notifications {
    pub adb: AdbClient,
}

impl Notifications {
    pub fn new(adb: AdbClient) -> Self {
        Self { adb }
    }

    pub async fn list(&self, serial: &str, package: Option<&str>) -> Result<NotificationList> {
        let output = self
            .adb
            .shell_args(
                serial,
                &[
                    "dumpsys".to_string(),
                    "notification".to_string(),
                    "--noredact".to_string(),
                ],
                Some(Duration::from_secs(30)),
                Some(16 * 1024 * 1024),
            )
            .await;
        let output = match output {
            Ok(output) => output,
            Err(_) => {
                self.adb
                    .shell_args(
                        serial,
                        &["dumpsys".to_string(), "notification".to_string()],
                        Some(Duration::from_secs(30)),
                        Some(16 * 1024 * 1024),
                    )
                    .await?
            }
        };
        let raw = output.stdout_text();
        let mut parsed = parse_notifications(&raw);
        if let Some(package) = package {
            parsed
                .notifications
                .retain(|notification| notification.package.as_deref() == Some(package));
        }
        if parsed.notifications.is_empty() {
            parsed.notes.push(
                "No notification records were parsed. NotificationManagerService output varies by Android version; inspect the raw dump or use `androidforge ui` on the notification shade."
                    .to_string(),
            );
        }
        parsed.raw = raw;
        Ok(parsed)
    }

    pub async fn channels(
        &self,
        serial: &str,
        package: Option<&str>,
    ) -> Result<NotificationChannels> {
        let list = self.list(serial, package).await?;
        let mut channels = parse_channel_catalog(&list.raw);
        if let Some(package) = package {
            channels.retain(|channel| channel.package.as_deref() == Some(package));
        }
        if channels.is_empty() {
            // Fall back to channels visible on active notification records.
            channels = list
                .channels
                .iter()
                .map(|(id, importance)| NotificationChannel {
                    package: package.map(str::to_string),
                    id: id.clone(),
                    importance: importance.clone(),
                })
                .collect();
        }
        Ok(NotificationChannels {
            channels,
            best_effort: true,
        })
    }

    pub async fn wait(
        &self,
        serial: &str,
        selector: &NotificationSelector,
        timeout: Duration,
        absent: bool,
    ) -> Result<NotificationWaitResult> {
        if !selector.has_conditions() {
            return Err(ForgeError::invalid_argument(
                "notification selector needs at least one condition such as --package or --text",
            ));
        }
        let started = Instant::now();
        let mut attempts = 0u32;
        let mut last_notes = Vec::new();
        loop {
            attempts += 1;
            let list = self.list(serial, selector.package.as_deref()).await?;
            let matched = list
                .notifications
                .iter()
                .find(|notification| selector.matches(notification))
                .cloned();
            if absent {
                if matched.is_none() {
                    return Ok(NotificationWaitResult {
                        present: false,
                        matched: true,
                        attempts,
                        duration_ms: clamp_ms(started.elapsed()),
                        notification: None,
                        notes: list.notes,
                    });
                }
            } else if let Some(notification) = matched {
                return Ok(NotificationWaitResult {
                    present: true,
                    matched: true,
                    attempts,
                    duration_ms: clamp_ms(started.elapsed()),
                    notification: Some(notification),
                    notes: list.notes,
                });
            }
            if last_notes.is_empty() {
                last_notes = list.notes;
            }
            if started.elapsed() >= timeout {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let notes = last_notes;
        if absent {
            Err(ForgeError::new(
                ErrorCode::UiAssertionFailed,
                "notification still exists after waiting for its absence",
            )
            .with_context(
                "selector",
                serde_json::to_value(selector).unwrap_or(serde_json::Value::Null),
            )
            .with_suggestion("Inspect notifications with `androidforge notifications list`.")
            .device(serial.to_string()))
        } else {
            Err(ForgeError::new(
                ErrorCode::UiAssertionFailed,
                "notification was not observed before the timeout",
            )
            .with_context(
                "selector",
                serde_json::to_value(selector).unwrap_or(serde_json::Value::Null),
            )
            .with_context("attempts", attempts)
            .with_suggestion("Check notification permission and the app's notification channel.")
            .with_suggestion("Inspect raw NotificationManagerService state with `androidforge diagnostics collect`.")
            .with_context("notes", serde_json::to_value(notes).unwrap_or(serde_json::Value::Null))
            .device(serial.to_string()))
        }
    }

    pub async fn tap(
        &self,
        serial: &str,
        selector: &NotificationSelector,
        ui: &UiManager,
    ) -> Result<NotificationInfo> {
        let result = self
            .wait(serial, selector, Duration::from_secs(1), false)
            .await?;
        let notification = result.notification.ok_or_else(|| {
            ForgeError::new(
                ErrorCode::SelectorNotFound,
                "notification was not found before tapping it",
            )
            .device(serial.to_string())
        })?;
        let expand = self
            .adb
            .shell_args(
                serial,
                &[
                    "cmd".to_string(),
                    "statusbar".to_string(),
                    "expand-notifications".to_string(),
                ],
                Some(Duration::from_secs(15)),
                None,
            )
            .await;
        if let Err(error) = expand {
            return Err(ForgeError::new(
                ErrorCode::UnsupportedOperation,
                "the device did not expose a notification shade expansion command",
            )
            .with_cause(error.message())
            .with_suggestion("Open the notification shade manually and use `androidforge ui tap`.")
            .device(serial.to_string()));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let text = notification
            .title
            .clone()
            .or(notification.text.clone())
            .ok_or_else(|| {
                ForgeError::new(
                    ErrorCode::UnsupportedOperation,
                    "the notification has no readable title/text to target",
                )
                .with_suggestion(
                    "Use `androidforge ui dump` and tap the notification by resource id or bounds.",
                )
                .device(serial.to_string())
            })?;
        let tap_result = match ui
            .tap(
                serial,
                &Selector {
                    text: Some(text.clone()),
                    ..Selector::default()
                },
            )
            .await
        {
            Ok(result) => Ok(result),
            Err(_) => {
                ui.tap(
                    serial,
                    &Selector {
                        text_contains: Some(text),
                        ..Selector::default()
                    },
                )
                .await
            }
        };
        let _ = self
            .adb
            .shell_args(
                serial,
                &[
                    "cmd".to_string(),
                    "statusbar".to_string(),
                    "collapse".to_string(),
                ],
                Some(Duration::from_secs(10)),
                None,
            )
            .await;
        tap_result?;
        Ok(notification)
    }

    pub async fn clear(
        &self,
        serial: &str,
        selector: &NotificationSelector,
    ) -> Result<Vec<NotificationInfo>> {
        let list = self.list(serial, selector.package.as_deref()).await?;
        let matched = list
            .notifications
            .into_iter()
            .filter(|notification| selector.matches(notification))
            .collect::<Vec<_>>();
        if matched.is_empty() {
            return Ok(Vec::new());
        }
        let mut cleared = Vec::new();
        for notification in matched {
            let Some(package) = notification.package.as_deref() else {
                continue;
            };
            let Some(id) = notification.id else {
                continue;
            };
            let tag = notification.tag.as_deref().unwrap_or("null");
            let mut success = false;
            for args in [
                vec![
                    "cmd".to_string(),
                    "notification".to_string(),
                    "cancel".to_string(),
                    package.to_string(),
                    id.to_string(),
                ],
                vec![
                    "cmd".to_string(),
                    "notification".to_string(),
                    "cancel".to_string(),
                    package.to_string(),
                    tag.to_string(),
                    id.to_string(),
                ],
            ] {
                if self
                    .adb
                    .shell_args(serial, &args, Some(Duration::from_secs(15)), None)
                    .await
                    .is_ok()
                {
                    success = true;
                    break;
                }
            }
            if success {
                cleared.push(notification);
            } else {
                return Err(ForgeError::new(
                    ErrorCode::UnsupportedOperation,
                    "this Android build does not expose `cmd notification cancel`",
                )
                .with_suggestion("Dismiss the notification through the shade with `androidforge notifications tap` or `androidforge ui`.")
                .device(serial.to_string()));
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let remaining = self.list(serial, selector.package.as_deref()).await?;
        let still_present = remaining
            .notifications
            .iter()
            .filter(|notification| selector.matches(notification))
            .count();
        if still_present > 0 {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                format!("{still_present} matching notification(s) remain after cancel"),
            )
            .with_suggestion(
                "Inspect notifications with `androidforge notifications list --package <package>`.",
            )
            .device(serial.to_string()));
        }
        Ok(cleared)
    }
}

pub fn parse_notifications(raw: &str) -> NotificationList {
    let pkg_regex = Regex::new(r"pkg=([^\s,}\]]+)").expect("notification package regex");
    let id_regex = Regex::new(r"\bid=(-?\d+)").expect("notification id regex");
    let tag_regex = Regex::new(r"\btag=([^\s,}\]]+)").expect("notification tag regex");
    let channel_regex =
        Regex::new(r"(?:channel|mChannelId)=([^\s,}\]]+)").expect("notification channel regex");
    let importance_regex =
        Regex::new(r"(?:importance|mImportance)=(\d+)").expect("notification importance regex");
    let title_regex =
        Regex::new(r"(?:android\.title|mTitle)=(.*)").expect("notification title regex");
    let text_regex = Regex::new(r"(?:android\.text|mText)=(.*)").expect("notification text regex");
    let mut notifications = Vec::new();
    let mut channels = BTreeMap::new();
    let mut current: Option<NotificationInfo> = None;
    let mut block = Vec::new();

    let flush = |current: &mut Option<NotificationInfo>,
                 block: &mut Vec<String>,
                 notifications: &mut Vec<NotificationInfo>| {
        if let Some(mut notification) = current.take() {
            notification.raw_excerpt = block.join("\n").chars().take(4000).collect();
            notifications.push(notification);
        }
        block.clear();
    };

    for line in raw.lines() {
        if line.contains("NotificationRecord{") || line.contains("NotificationRecord(") {
            flush(&mut current, &mut block, &mut notifications);
            block.push(line.to_string());
            let package = pkg_regex
                .captures(line)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().trim_matches('"').to_string());
            let id = id_regex
                .captures(line)
                .and_then(|captures| captures.get(1))
                .and_then(|value| value.as_str().parse::<i32>().ok());
            let tag = tag_regex
                .captures(line)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().to_string())
                .filter(|value| value != "null");
            let channel = channel_regex
                .captures(line)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().trim_matches('"').to_string())
                .filter(|value| value != "null");
            if let Some(channel) = &channel {
                channels.entry(channel.clone()).or_insert(None);
            }
            current = Some(NotificationInfo {
                package,
                id,
                tag,
                channel,
                importance: importance_regex
                    .captures(line)
                    .and_then(|captures| captures.get(1))
                    .map(|value| value.as_str().to_string()),
                title: None,
                text: None,
                raw_excerpt: String::new(),
            });
        } else if current.is_some() {
            block.push(line.to_string());
            if let Some(notification) = current.as_mut() {
                if notification.title.is_none()
                    && let Some(captures) = title_regex.captures(line)
                {
                    notification.title = clean_extra(captures.get(1).unwrap().as_str());
                }
                if notification.text.is_none()
                    && let Some(captures) = text_regex.captures(line)
                {
                    notification.text = clean_extra(captures.get(1).unwrap().as_str());
                }
                if notification.channel.is_none()
                    && let Some(captures) = channel_regex.captures(line)
                {
                    let channel = captures.get(1).unwrap().as_str().trim_matches('"');
                    if channel != "null" {
                        notification.channel = Some(channel.to_string());
                        channels.entry(channel.to_string()).or_insert(None);
                    }
                }
                if notification.importance.is_none()
                    && let Some(captures) = importance_regex.captures(line)
                {
                    notification.importance = Some(captures.get(1).unwrap().as_str().to_string());
                }
            }
            if line.trim().is_empty() {
                flush(&mut current, &mut block, &mut notifications);
            }
        }
    }
    flush(&mut current, &mut block, &mut notifications);
    for notification in &notifications {
        if let Some(channel) = &notification.channel
            && let Some(importance) = &notification.importance
        {
            channels.insert(channel.clone(), Some(importance.clone()));
        }
    }
    NotificationList {
        notifications,
        best_effort: true,
        notes: Vec::new(),
        channels,
        raw: String::new(),
    }
}

fn parse_channel_catalog(raw: &str) -> Vec<NotificationChannel> {
    let app_regex =
        Regex::new(r"^\s*AppSettings:\s+([^\s(]+)").expect("notification app settings regex");
    let id_regex = Regex::new(r"mId='([^']+)'").expect("notification channel id regex");
    let numeric_id_regex =
        Regex::new(r"\bid='([^']+)'").expect("notification channel numeric id regex");
    let importance_regex =
        Regex::new(r"\bmImportance=(\d+)").expect("notification channel importance regex");
    let mut current_package: Option<String> = None;
    let mut channels = Vec::new();
    for line in raw.lines() {
        if let Some(captures) = app_regex.captures(line) {
            current_package = captures
                .get(1)
                .map(|value| value.as_str().trim_matches('"').to_string());
            continue;
        }
        if !line.contains("NotificationChannel{") || line.contains("NotificationChannelGroup{") {
            continue;
        }
        let id = id_regex
            .captures(line)
            .or_else(|| numeric_id_regex.captures(line))
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().to_string());
        let Some(id) = id else { continue };
        let importance = importance_regex
            .captures(line)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().to_string());
        if channels.iter().any(|channel: &NotificationChannel| {
            channel.id == id && channel.package == current_package
        }) {
            continue;
        }
        channels.push(NotificationChannel {
            package: current_package.clone(),
            id,
            importance,
        });
    }
    channels
}

fn clean_extra(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value == "null" {
        return None;
    }
    let value = value
        .strip_prefix("String (")
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(value);
    Some(value.trim_matches('"').to_string())
}

fn clamp_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"
NotificationRecord{7f3a1b2c pkg=com.example id=42 tag=work channel=deals importance=3}
  android.title=String (New deal)
  android.text=String (Save 20%)
  ...
NotificationRecord{8b2c3d4e pkg=com.other id=1 tag=null channel=default importance=2}
  android.title=String (Other)
  android.text=String (Hello)
"#;

    #[test]
    fn parses_notification_records() {
        let parsed = parse_notifications(DUMP);
        assert_eq!(parsed.notifications.len(), 2);
        let first = &parsed.notifications[0];
        assert_eq!(first.package.as_deref(), Some("com.example"));
        assert_eq!(first.id, Some(42));
        assert_eq!(first.tag.as_deref(), Some("work"));
        assert_eq!(first.channel.as_deref(), Some("deals"));
        assert_eq!(first.title.as_deref(), Some("New deal"));
        assert_eq!(first.text.as_deref(), Some("Save 20%"));
        assert!(parsed.channels.contains_key("deals"));
    }

    #[test]
    fn parses_all_channels_from_app_settings_catalog() {
        let dump = r#"
PackagePreferences:
  AppSettings: com.example (10217) importance=DEFAULT userSet=false
    NotificationChannel{mId='example_free', mName=Free, mImportance=4, mBlockableSystem=false}
    NotificationChannel{mId='example_sales', mName=Sales, mImportance=4, mBlockableSystem=false}
  AppSettings: com.other (10100)
    NotificationChannel{mId='other_channel', mName=Other, mImportance=2}
"#;
        let channels = parse_channel_catalog(dump);
        assert_eq!(channels.len(), 3);
        assert!(channels.iter().any(|channel| channel.id == "example_sales"
            && channel.package.as_deref() == Some("com.example")
            && channel.importance.as_deref() == Some("4")));
    }

    #[test]
    fn selector_matches_text() {
        let parsed = parse_notifications(DUMP);
        let selector = NotificationSelector {
            package: Some("com.example".to_string()),
            text_contains: Some("20%".to_string()),
            ..NotificationSelector::default()
        };
        assert!(
            parsed
                .notifications
                .iter()
                .any(|item| selector.matches(item))
        );
    }
}
