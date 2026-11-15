use std::time::Duration;

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppOpState {
    pub package: String,
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default)]
    pub determined: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AppOps {
    pub adb: AdbClient,
}

impl AppOps {
    pub fn new(adb: AdbClient) -> Self {
        Self { adb }
    }

    pub async fn get(&self, serial: &str, package: &str, operation: &str) -> Result<AppOpState> {
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
                Some(1024 * 1024),
            )
            .await;
        let output = match output {
            Ok(output) => output,
            Err(error) if error.code() == ErrorCode::CommandFailed => {
                self.adb
                    .shell_args(
                        serial,
                        &[
                            "appops".to_string(),
                            "get".to_string(),
                            package.to_string(),
                            operation.to_string(),
                        ],
                        Some(Duration::from_secs(15)),
                        Some(1024 * 1024),
                    )
                    .await?
            }
            Err(error) => return Err(error),
        };
        Ok(parse_appop(
            package,
            operation,
            &output.stdout_text(),
            &output.stderr_text(),
        ))
    }

    pub async fn set(
        &self,
        serial: &str,
        package: &str,
        operation: &str,
        mode: &str,
    ) -> Result<AppOpState> {
        if !matches!(mode, "allow" | "deny" | "ignore" | "default" | "foreground") {
            return Err(ForgeError::invalid_argument(format!(
                "invalid AppOps mode '{mode}'; expected allow, deny, ignore, default or foreground"
            )));
        }
        let output = self
            .adb
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
        let state = self.get(serial, package, operation).await?;
        if state.mode.as_deref() != Some(mode) {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                format!(
                    "AppOps did not accept {operation}={mode} for {package}; current mode is {}",
                    state.mode.as_deref().unwrap_or("unknown")
                ),
            )
            .with_context_string("output", truncate(output.combined_text().trim(), 2000))
            .with_suggestion("The device or Android version may restrict this operation.")
            .device(serial.to_string()));
        }
        Ok(state)
    }

    pub async fn reset(
        &self,
        serial: &str,
        package: &str,
        operation: Option<&str>,
    ) -> Result<AppOpState> {
        let mut args = vec![
            "cmd".to_string(),
            "appops".to_string(),
            "reset".to_string(),
            package.to_string(),
        ];
        if let Some(operation) = operation {
            args.push(operation.to_string());
        }
        let output = self
            .adb
            .shell_args(
                serial,
                &args,
                Some(Duration::from_secs(20)),
                Some(1024 * 1024),
            )
            .await?;
        if let Some(operation) = operation {
            let state = self.get(serial, package, operation).await?;
            Ok(state)
        } else {
            Ok(AppOpState {
                package: package.to_string(),
                operation: operation.unwrap_or("*").to_string(),
                mode: None,
                determined: true,
                raw: Some(truncate(output.combined_text().trim(), 2000)),
            })
        }
    }
}

fn parse_appop(package: &str, operation: &str, stdout: &str, stderr: &str) -> AppOpState {
    let combined = format!("{}\n{}", stdout, stderr);
    let mut mode = None;
    let mut determined = false;
    for line in combined.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.eq_ignore_ascii_case("no operations.")
            || line.starts_with("Error:")
        {
            continue;
        }
        let value = if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case(operation)
                || name
                    .trim()
                    .eq_ignore_ascii_case(&operation.to_ascii_uppercase())
                || name.trim().eq_ignore_ascii_case("mode")
            {
                Some(value.trim())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(value) = value {
            if matches!(
                value,
                "allow" | "deny" | "ignore" | "default" | "foreground"
            ) {
                mode = Some(value.to_string());
                determined = true;
                break;
            }
        }
    }
    AppOpState {
        package: package.to_string(),
        operation: operation.to_string(),
        mode,
        determined,
        raw: Some(truncate(combined.trim(), 4000)),
    }
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
    fn parses_appops_output() {
        let state = parse_appop(
            "com.example",
            "POST_NOTIFICATION",
            "POST_NOTIFICATION: deny\n",
            "",
        );
        assert_eq!(state.mode.as_deref(), Some("deny"));
        assert!(state.determined);
    }

    #[test]
    fn parses_unknown_appop() {
        let state = parse_appop("com.example", "UNKNOWN", "No operations.\n", "");
        assert_eq!(state.mode, None);
        assert!(!state.determined);
    }
}
