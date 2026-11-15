use std::time::Duration;

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionState {
    pub package: String,
    pub permission: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted: Option<bool>,
    #[serde(default)]
    pub determined: bool,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appop_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Permissions {
    pub adb: AdbClient,
}

impl Permissions {
    pub fn new(adb: AdbClient) -> Self {
        Self { adb }
    }

    pub async fn check(
        &self,
        serial: &str,
        package: &str,
        permission: &str,
    ) -> Result<PermissionState> {
        let permission = normalized_permission(permission);
        let mut notes = Vec::new();
        let mut granted = None;
        let mut source = "pm_check_permission".to_string();
        let pm_result = self
            .adb
            .shell_args(
                serial,
                &[
                    "pm".to_string(),
                    "check-permission".to_string(),
                    permission.clone(),
                    package.to_string(),
                ],
                Some(Duration::from_secs(15)),
                Some(1024 * 1024),
            )
            .await;
        match pm_result {
            Ok(output) => {
                let text = format!("{}\n{}", output.stdout_text(), output.stderr_text());
                if let Some(value) = parse_permission_text(&text) {
                    granted = Some(value);
                } else if !text.trim().is_empty() {
                    notes.push(format!(
                        "pm check-permission returned: {}",
                        truncate(text.trim(), 500)
                    ));
                }
            }
            Err(error) => {
                if error.code() == ErrorCode::PackageNotFound
                    || error
                        .message()
                        .to_ascii_lowercase()
                        .contains("unknown package")
                {
                    return Err(package_not_found(package, serial));
                }
                // Android 16 removed `pm check-permission`; dumpsys is the durable fallback.
                // Do not turn a normal fallback into noise in every permission result.
                if error.code() != ErrorCode::CommandFailed {
                    notes.push(format!("pm check-permission failed: {}", error.message()));
                }
            }
        }

        if granted.is_none() {
            source = "dumpsys_package".to_string();
            let dumpsys = self
                .adb
                .shell_args(
                    serial,
                    &[
                        "dumpsys".to_string(),
                        "package".to_string(),
                        package.to_string(),
                    ],
                    Some(Duration::from_secs(30)),
                    Some(16 * 1024 * 1024),
                )
                .await
                .map_err(|error| {
                    if error.code() == ErrorCode::CommandFailed {
                        package_not_found(package, serial)
                    } else {
                        error
                    }
                })?;
            let text = format!("{}\n{}", dumpsys.stdout_text(), dumpsys.stderr_text());
            if text.to_ascii_lowercase().contains("unknown package") {
                return Err(package_not_found(package, serial));
            }
            if text.trim().is_empty() {
                return Err(package_not_found(package, serial));
            }
            granted = parse_permission_from_dumpsys(&text, &permission);
        }

        let appop = self
            .appop_mode(serial, package, appop_for_permission(&permission))
            .await;
        if granted.is_none() {
            if let Some(mode) = &appop {
                source = "appops".to_string();
                granted = match mode.as_str() {
                    "allow" => Some(true),
                    "deny" | "ignore" => Some(false),
                    _ => None,
                };
            }
        }

        Ok(PermissionState {
            package: package.to_string(),
            permission: permission.to_string(),
            granted,
            determined: granted.is_some(),
            source,
            appop_mode: appop,
            raw: None,
            notes,
        })
    }

    pub async fn grant(
        &self,
        serial: &str,
        package: &str,
        permission: &str,
    ) -> Result<PermissionState> {
        let permission = normalized_permission(permission);
        let before = self.check(serial, package, &permission).await?;
        let pm = self
            .adb
            .shell_args(
                serial,
                &[
                    "pm".to_string(),
                    "grant".to_string(),
                    package.to_string(),
                    permission.clone(),
                ],
                Some(Duration::from_secs(20)),
                Some(1024 * 1024),
            )
            .await;
        let output_text = match pm {
            Ok(output) => output.combined_text(),
            Err(error) => error.message().to_string(),
        };
        if let Some(appop) = appop_for_permission(&permission) {
            if self.check(serial, package, &permission).await?.granted != Some(true) {
                let _ = self.set_appop(serial, package, appop, "allow").await;
            }
        }
        let after = self.check(serial, package, &permission).await?;
        if after.granted != Some(true) {
            return Err(ForgeError::new(
                ErrorCode::PermissionDenied,
                format!("permission {permission} was not granted to {package}"),
            )
            .with_context_string("before", format!("{:?}", before.granted))
            .with_context_string("after", format!("{:?}", after.granted))
            .with_context_string("output", truncate(output_text.trim(), 2000))
            .with_suggestion("The permission may not be declared by the application.")
            .with_suggestion("Some permissions require a special AppOps operation or user consent.")
            .device(serial.to_string()));
        }
        Ok(after)
    }

    pub async fn revoke(
        &self,
        serial: &str,
        package: &str,
        permission: &str,
    ) -> Result<PermissionState> {
        let permission = normalized_permission(permission);
        let pm = self
            .adb
            .shell_args(
                serial,
                &[
                    "pm".to_string(),
                    "revoke".to_string(),
                    package.to_string(),
                    permission.clone(),
                ],
                Some(Duration::from_secs(20)),
                Some(1024 * 1024),
            )
            .await;
        let output_text = match pm {
            Ok(output) => output.combined_text(),
            Err(error) => error.message().to_string(),
        };
        if let Some(appop) = appop_for_permission(&permission) {
            if self.check(serial, package, &permission).await?.granted != Some(false) {
                let _ = self.set_appop(serial, package, appop, "deny").await;
            }
        }
        let after = self.check(serial, package, &permission).await?;
        if after.granted == Some(true) {
            return Err(ForgeError::new(
                ErrorCode::PermissionDenied,
                format!("permission {permission} is still granted to {package}"),
            )
            .with_context_string("output", truncate(output_text.trim(), 2000))
            .with_suggestion(
                "Some permissions cannot be revoked with `pm revoke` on this Android version.",
            )
            .with_suggestion("Use `androidforge appops set <package> <op> deny` where applicable.")
            .device(serial.to_string()));
        }
        Ok(after)
    }

    async fn appop_mode(
        &self,
        serial: &str,
        package: &str,
        operation: Option<&str>,
    ) -> Option<String> {
        let operation = operation?;
        let output = self
            .adb
            .shell_args(
                serial,
                &[
                    "cmd".to_string(),
                    "appops".to_string(),
                    "get".to_string(),
                    package.to_string(),
                    operation.to_string(),
                ],
                Some(Duration::from_secs(15)),
                Some(512 * 1024),
            )
            .await
            .ok()?;
        parse_appop_mode(&output.stdout_text(), operation)
    }

    async fn set_appop(
        &self,
        serial: &str,
        package: &str,
        operation: &str,
        mode: &str,
    ) -> Result<()> {
        self.adb
            .shell_args(
                serial,
                &[
                    "cmd".to_string(),
                    "appops".to_string(),
                    "set".to_string(),
                    package.to_string(),
                    operation.to_string(),
                    mode.to_string(),
                ],
                Some(Duration::from_secs(20)),
                Some(1024 * 1024),
            )
            .await?;
        Ok(())
    }
}

/// Accept both `POST_NOTIFICATIONS` and `android.permission.POST_NOTIFICATIONS`.
/// `pm` itself requires the fully qualified name.
fn normalized_permission(permission: &str) -> String {
    let permission = permission.trim();
    if permission.contains('.') {
        permission.to_string()
    } else {
        format!("android.permission.{permission}")
    }
}

fn appop_for_permission(permission: &str) -> Option<&'static str> {
    match permission {
        "android.permission.POST_NOTIFICATIONS" => Some("POST_NOTIFICATION"),
        "android.permission.SCHEDULE_EXACT_ALARM" => Some("SCHEDULE_EXACT_ALARM"),
        "android.permission.USE_EXACT_ALARM" => Some("USE_EXACT_ALARM"),
        "android.permission.SYSTEM_ALERT_WINDOW" => Some("SYSTEM_ALERT_WINDOW"),
        "android.permission.MANAGE_EXTERNAL_STORAGE" => Some("MANAGE_EXTERNAL_STORAGE"),
        _ => None,
    }
}

fn parse_permission_text(text: &str) -> Option<bool> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("granted=true") || lower.contains("is granted") {
        return Some(true);
    }
    if lower.contains("granted=false")
        || lower.contains("is denied")
        || lower.contains("not granted")
        || lower.contains("denied")
    {
        return Some(false);
    }
    let trimmed = lower.trim();
    if trimmed == "granted" || trimmed == "true" {
        return Some(true);
    }
    if trimmed == "denied" || trimmed == "false" {
        return Some(false);
    }
    None
}

fn parse_permission_from_dumpsys(text: &str, permission: &str) -> Option<bool> {
    let pattern = format!(
        r"(?im)^\s*{}\s*:\s*granted=(true|false)",
        regex::escape(permission)
    );
    let regex = Regex::new(&pattern).ok()?;
    regex
        .captures(text)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str() == "true")
}

fn parse_appop_mode(text: &str, operation: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case(operation) {
                let value = value.trim();
                if matches!(
                    value,
                    "allow" | "deny" | "ignore" | "default" | "foreground"
                ) {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn package_not_found(package: &str, serial: &str) -> ForgeError {
    ForgeError::new(
        ErrorCode::PackageNotFound,
        format!("package {package} is not installed on {serial}"),
    )
    .with_suggestion("Run `androidforge app list` to inspect installed packages.")
    .with_suggestion("Install the application with `androidforge app install <apk>`.")
    .device(serial.to_string())
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
    fn normalizes_short_and_qualified_permission_names() {
        assert_eq!(
            normalized_permission("POST_NOTIFICATIONS"),
            "android.permission.POST_NOTIFICATIONS"
        );
        assert_eq!(
            normalized_permission("android.permission.CAMERA"),
            "android.permission.CAMERA"
        );
        assert_eq!(
            normalized_permission("com.example.CUSTOM"),
            "com.example.CUSTOM"
        );
    }

    #[test]
    fn parses_permission_outputs() {
        assert_eq!(parse_permission_text("granted=true"), Some(true));
        assert_eq!(parse_permission_text("Permission is denied"), Some(false));
        assert_eq!(parse_permission_text("garbage"), None);
        let dumpsys =
            "runtime permissions:\n  android.permission.CAMERA: granted=true, flags=[ ]\n";
        assert_eq!(
            parse_permission_from_dumpsys(dumpsys, "android.permission.CAMERA"),
            Some(true)
        );
    }

    #[test]
    fn parses_appop_modes() {
        assert_eq!(
            parse_appop_mode("POST_NOTIFICATION: allow\n", "POST_NOTIFICATION").as_deref(),
            Some("allow")
        );
        assert_eq!(
            parse_appop_mode("No operations.\n", "POST_NOTIFICATION"),
            None
        );
    }
}
