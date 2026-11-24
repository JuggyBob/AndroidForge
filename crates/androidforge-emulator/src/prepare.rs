use androidforge_adb::{AdbClient, WakeResult};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::paths::ForgePaths;
use androidforge_core::session::Session;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevicePrepareOptions {
    pub animations: bool,
    pub keep_awake: bool,
    pub wake: bool,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub font_scale: Option<String>,
    pub dark_mode: Option<bool>,
    pub orientation: Option<String>,
    pub density: Option<u32>,
}

impl Default for DevicePrepareOptions {
    fn default() -> Self {
        Self {
            animations: true,
            keep_awake: true,
            wake: true,
            locale: None,
            timezone: None,
            font_scale: None,
            dark_mode: None,
            orientation: None,
            density: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepareChange {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub applied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepareResult {
    pub changes: Vec<PrepareChange>,
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake: Option<WakeResult>,
    pub prepared: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreResult {
    pub restored: Vec<String>,
    pub failed: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DevicePreparer {
    pub adb: AdbClient,
}

impl DevicePreparer {
    pub fn new(adb: AdbClient) -> Self {
        Self { adb }
    }

    pub async fn prepare(
        &self,
        serial: &str,
        options: &DevicePrepareOptions,
        session: &mut Session,
        paths: &ForgePaths,
    ) -> Result<PrepareResult> {
        let mut changes = Vec::new();
        let mut warnings = Vec::new();
        let mut wake = None;
        if options.wake {
            match self.adb.wake(serial).await {
                Ok(result) => wake = Some(result),
                Err(error) => warnings.push(format!("wake/unlock failed: {}", error.message())),
            }
        }
        if options.animations {
            for key in [
                "window_animation_scale",
                "transition_animation_scale",
                "animator_duration_scale",
            ] {
                match self
                    .set_setting(serial, "global", key, "0.0", session, paths)
                    .await
                {
                    Ok(change) => changes.push(change),
                    Err(error) => {
                        warnings.push(format!("could not disable {key}: {}", error.message()))
                    }
                }
            }
        }
        if options.keep_awake {
            match self
                .set_setting(
                    serial,
                    "global",
                    "stay_on_while_plugged_in",
                    "7",
                    session,
                    paths,
                )
                .await
            {
                Ok(change) => changes.push(change),
                Err(error) => {
                    warnings.push(format!("could not keep screen awake: {}", error.message()))
                }
            }
        }
        if let Some(locale) = &options.locale {
            match self
                .set_setting(serial, "system", "system_locales", locale, session, paths)
                .await
            {
                Ok(change) => changes.push(change),
                Err(error) => warnings.push(format!("could not set locale: {}", error.message())),
            }
        }
        if let Some(font_scale) = &options.font_scale {
            match self
                .set_setting(serial, "system", "font_scale", font_scale, session, paths)
                .await
            {
                Ok(change) => changes.push(change),
                Err(error) => {
                    warnings.push(format!("could not set font scale: {}", error.message()))
                }
            }
        }
        if let Some(timezone) = &options.timezone {
            match self.set_timezone(serial, timezone, session, paths).await {
                Ok(change) => changes.push(change),
                Err(error) => warnings.push(format!("could not set timezone: {}", error.message())),
            }
        }
        if let Some(dark_mode) = options.dark_mode {
            match self.set_dark_mode(serial, dark_mode, session, paths).await {
                Ok(change) => changes.push(change),
                Err(error) => {
                    warnings.push(format!("could not set dark mode: {}", error.message()))
                }
            }
        }
        if let Some(orientation) = &options.orientation {
            match self
                .set_orientation(serial, orientation, session, paths)
                .await
            {
                Ok(mut orientation_changes) => changes.append(&mut orientation_changes),
                Err(error) => {
                    warnings.push(format!("could not set orientation: {}", error.message()))
                }
            }
        }
        if let Some(density) = options.density {
            match self.set_density(serial, density, session, paths).await {
                Ok(change) => changes.push(change),
                Err(error) => warnings.push(format!("could not set density: {}", error.message())),
            }
        }
        let prepared = changes.iter().any(|change| change.applied);
        Ok(PrepareResult {
            changes,
            warnings,
            wake,
            prepared,
        })
    }

    pub async fn restore(
        &self,
        serial: &str,
        session: &mut Session,
        paths: &ForgePaths,
    ) -> Result<RestoreResult> {
        let settings = session.device_settings.clone();
        let mut restored = Vec::new();
        let mut failed = Vec::new();
        let mut warnings = Vec::new();
        for (key, previous) in settings {
            let result = if let Some(namespace_key) = key.strip_prefix("global:") {
                self.restore_setting(serial, "global", namespace_key, previous.as_deref())
                    .await
            } else if let Some(namespace_key) = key.strip_prefix("system:") {
                self.restore_setting(serial, "system", namespace_key, previous.as_deref())
                    .await
            } else if let Some(namespace_key) = key.strip_prefix("secure:") {
                self.restore_setting(serial, "secure", namespace_key, previous.as_deref())
                    .await
            } else if let Some(prop) = key.strip_prefix("prop:") {
                self.restore_prop(serial, prop, previous.as_deref()).await
            } else if key == "wm:density" {
                self.restore_wm_density(serial, previous.as_deref()).await
            } else if key == "wm:size" {
                self.restore_wm_size(serial, previous.as_deref()).await
            } else if key == "cmd:uimode_night" {
                self.restore_night_mode(serial, previous.as_deref()).await
            } else if key == "svc:stayon" {
                self.restore_stayon(serial, previous.as_deref()).await
            } else {
                Err(ForgeError::new(
                    ErrorCode::UnsupportedOperation,
                    format!("no restore handler for {key}"),
                ))
            };
            match result {
                Ok(()) => {
                    restored.push(key.clone());
                    session.device_settings.remove(&key);
                }
                Err(error) => {
                    failed.push(key.clone());
                    warnings.push(format!("could not restore {key}: {}", error.message()));
                }
            }
        }
        if let Err(error) = session.save(paths) {
            warnings.push(format!(
                "could not save restored session state: {}",
                error.message()
            ));
        }
        Ok(RestoreResult {
            restored,
            failed,
            warnings,
        })
    }

    async fn set_setting(
        &self,
        serial: &str,
        namespace: &str,
        key: &str,
        value: &str,
        session: &mut Session,
        paths: &ForgePaths,
    ) -> Result<PrepareChange> {
        let current = self.adb.settings_get(serial, namespace, key).await.ok();
        let previous = current
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "null")
            .map(str::to_string);
        if previous.as_deref() == Some(value) {
            return Ok(PrepareChange {
                key: format!("{namespace}:{key}"),
                previous,
                value: Some(value.to_string()),
                applied: false,
                note: Some("already at requested value".to_string()),
            });
        }
        if current.is_none() {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                format!("could not read current value of {namespace}:{key}"),
            )
            .with_suggestion("AndroidForge will not change a setting it cannot restore."));
        }
        session.record_device_setting(paths, format!("{namespace}:{key}"), previous.clone())?;
        self.adb.settings_put(serial, namespace, key, value).await?;
        let actual = self.adb.settings_get(serial, namespace, key).await?;
        if actual.trim() != value {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                format!("{namespace}:{key} did not accept {value}; current value is {actual}"),
            ));
        }
        Ok(PrepareChange {
            key: format!("{namespace}:{key}"),
            previous,
            value: Some(value.to_string()),
            applied: true,
            note: None,
        })
    }

    async fn set_timezone(
        &self,
        serial: &str,
        timezone: &str,
        session: &mut Session,
        paths: &ForgePaths,
    ) -> Result<PrepareChange> {
        let current = self.adb.getprop(serial, "persist.sys.timezone").await.ok();
        let previous = current
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if previous.as_deref() == Some(timezone) {
            return Ok(PrepareChange {
                key: "prop:persist.sys.timezone".to_string(),
                previous,
                value: Some(timezone.to_string()),
                applied: false,
                note: Some("already at requested value".to_string()),
            });
        }
        session.record_device_setting(paths, "prop:persist.sys.timezone", previous.clone())?;
        let output = self
            .adb
            .shell_args(
                serial,
                &[
                    "setprop".to_string(),
                    "persist.sys.timezone".to_string(),
                    timezone.to_string(),
                ],
                Some(std::time::Duration::from_secs(10)),
                Some(1024 * 1024),
            )
            .await;
        if let Err(error) = output {
            return Err(ForgeError::new(
                ErrorCode::PermissionDenied,
                "the device rejected the timezone property change",
            )
            .with_cause(error.message())
            .with_suggestion(
                "Timezone changes may require a userdebug build or an app-specific hook.",
            ));
        }
        let after = self
            .adb
            .getprop(serial, "persist.sys.timezone")
            .await
            .unwrap_or_default();
        if after.trim() != timezone {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                format!("timezone did not change; device reports '{after}'"),
            )
            .with_suggestion("This property may be read-only on a production device."));
        }
        Ok(PrepareChange {
            key: "prop:persist.sys.timezone".to_string(),
            previous,
            value: Some(timezone.to_string()),
            applied: true,
            note: None,
        })
    }

    async fn set_dark_mode(
        &self,
        serial: &str,
        dark: bool,
        session: &mut Session,
        paths: &ForgePaths,
    ) -> Result<PrepareChange> {
        let before = self
            .adb
            .shell_args(
                serial,
                &["cmd".to_string(), "uimode".to_string(), "night".to_string()],
                Some(std::time::Duration::from_secs(10)),
                None,
            )
            .await
            .ok()
            .map(|output| output.stdout_text());
        let previous = before.as_deref().and_then(|text| {
            text.split(':')
                .nth(1)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });
        let mode = if dark { "yes" } else { "no" };
        session.record_device_setting(paths, "cmd:uimode_night", previous.clone())?;
        self.adb
            .shell_args(
                serial,
                &[
                    "cmd".to_string(),
                    "uimode".to_string(),
                    "night".to_string(),
                    mode.to_string(),
                ],
                Some(std::time::Duration::from_secs(10)),
                Some(1024 * 1024),
            )
            .await?;
        let after = self
            .adb
            .shell_args(
                serial,
                &["cmd".to_string(), "uimode".to_string(), "night".to_string()],
                Some(std::time::Duration::from_secs(10)),
                None,
            )
            .await
            .map(|output| output.stdout_text())
            .unwrap_or_default();
        if !after
            .to_ascii_lowercase()
            .contains(&format!("night mode: {mode}"))
        {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                "the device did not confirm the requested night mode",
            )
            .with_context_string("output", after.trim().to_string()));
        }
        Ok(PrepareChange {
            key: "cmd:uimode_night".to_string(),
            previous,
            value: Some(mode.to_string()),
            applied: true,
            note: None,
        })
    }

    async fn set_orientation(
        &self,
        serial: &str,
        orientation: &str,
        session: &mut Session,
        paths: &ForgePaths,
    ) -> Result<Vec<PrepareChange>> {
        let rotation = match orientation {
            "portrait" => 0,
            "landscape" => 1,
            "reverse_portrait" => 2,
            "reverse_landscape" => 3,
            other => {
                return Err(ForgeError::invalid_argument(format!(
                    "unsupported orientation '{other}'; use portrait, landscape, reverse_portrait or reverse_landscape"
                )));
            }
        };
        let mut changes = Vec::new();
        changes.push(
            self.set_setting(
                serial,
                "system",
                "accelerometer_rotation",
                "0",
                session,
                paths,
            )
            .await?,
        );
        changes.push(
            self.set_setting(
                serial,
                "system",
                "user_rotation",
                &rotation.to_string(),
                session,
                paths,
            )
            .await?,
        );
        Ok(changes)
    }

    async fn set_density(
        &self,
        serial: &str,
        density: u32,
        session: &mut Session,
        paths: &ForgePaths,
    ) -> Result<PrepareChange> {
        if !(72..=1000).contains(&density) {
            return Err(ForgeError::invalid_argument(
                "--density must be between 72 and 1000",
            ));
        }
        let before = self
            .adb
            .shell_args(
                serial,
                &["wm".to_string(), "density".to_string()],
                Some(std::time::Duration::from_secs(10)),
                None,
            )
            .await
            .map(|output| output.stdout_text())
            .unwrap_or_default();
        let previous = if before.trim().is_empty() {
            None
        } else {
            Some(before.clone())
        };
        session.record_device_setting(paths, "wm:density", previous.clone())?;
        self.adb
            .shell_args(
                serial,
                &["wm".to_string(), "density".to_string(), density.to_string()],
                Some(std::time::Duration::from_secs(10)),
                None,
            )
            .await?;
        let after = self
            .adb
            .shell_args(
                serial,
                &["wm".to_string(), "density".to_string()],
                Some(std::time::Duration::from_secs(10)),
                None,
            )
            .await
            .map(|output| output.stdout_text())
            .unwrap_or_default();
        if !after.contains(&density.to_string()) {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                "density did not change",
            ));
        }
        Ok(PrepareChange {
            key: "wm:density".to_string(),
            previous,
            value: Some(density.to_string()),
            applied: true,
            note: None,
        })
    }

    async fn restore_setting(
        &self,
        serial: &str,
        namespace: &str,
        key: &str,
        previous: Option<&str>,
    ) -> Result<()> {
        match previous {
            Some(value) => {
                self.adb.settings_put(serial, namespace, key, value).await?;
                let actual = self.adb.settings_get(serial, namespace, key).await?;
                if actual.trim() != value {
                    return Err(ForgeError::new(
                        ErrorCode::CommandFailed,
                        format!("restore of {namespace}:{key} was not confirmed"),
                    ));
                }
                Ok(())
            }
            None => {
                self.adb
                    .shell_args(
                        serial,
                        &[
                            "settings".to_string(),
                            "delete".to_string(),
                            namespace.to_string(),
                            key.to_string(),
                        ],
                        Some(std::time::Duration::from_secs(10)),
                        None,
                    )
                    .await?;
                Ok(())
            }
        }
    }

    async fn restore_prop(&self, serial: &str, key: &str, previous: Option<&str>) -> Result<()> {
        let value = previous.unwrap_or("");
        self.adb
            .shell_args(
                serial,
                &["setprop".to_string(), key.to_string(), value.to_string()],
                Some(std::time::Duration::from_secs(10)),
                None,
            )
            .await?;
        Ok(())
    }

    async fn restore_wm_density(&self, serial: &str, previous: Option<&str>) -> Result<()> {
        let previous = previous.unwrap_or_default();
        let override_density = previous.lines().find_map(|line| {
            line.trim()
                .strip_prefix("Override density:")
                .map(str::trim)
                .and_then(|value| value.parse::<u32>().ok())
        });
        if let Some(density) = override_density {
            self.adb
                .shell_args(
                    serial,
                    &["wm".to_string(), "density".to_string(), density.to_string()],
                    Some(std::time::Duration::from_secs(10)),
                    None,
                )
                .await?;
        } else {
            self.adb
                .shell_args(
                    serial,
                    &["wm".to_string(), "density".to_string(), "reset".to_string()],
                    Some(std::time::Duration::from_secs(10)),
                    None,
                )
                .await?;
        }
        Ok(())
    }

    async fn restore_wm_size(&self, serial: &str, previous: Option<&str>) -> Result<()> {
        let previous = previous.unwrap_or_default();
        let override_size = previous.lines().find_map(|line| {
            line.trim()
                .strip_prefix("Override size:")
                .map(str::trim)
                .and_then(|value| value.parse::<String>().ok())
        });
        if let Some(size) = override_size {
            self.adb
                .shell_args(
                    serial,
                    &["wm".to_string(), "size".to_string(), size],
                    Some(std::time::Duration::from_secs(10)),
                    None,
                )
                .await?;
        } else {
            self.adb
                .shell_args(
                    serial,
                    &["wm".to_string(), "size".to_string(), "reset".to_string()],
                    Some(std::time::Duration::from_secs(10)),
                    None,
                )
                .await?;
        }
        Ok(())
    }

    async fn restore_night_mode(&self, serial: &str, previous: Option<&str>) -> Result<()> {
        let mode = previous.unwrap_or("auto");
        self.adb
            .shell_args(
                serial,
                &[
                    "cmd".to_string(),
                    "uimode".to_string(),
                    "night".to_string(),
                    mode.to_string(),
                ],
                Some(std::time::Duration::from_secs(10)),
                None,
            )
            .await?;
        Ok(())
    }

    async fn restore_stayon(&self, serial: &str, previous: Option<&str>) -> Result<()> {
        let value = previous.unwrap_or("false");
        let output = self
            .adb
            .shell_args(
                serial,
                &[
                    "svc".to_string(),
                    "power".to_string(),
                    "stayon".to_string(),
                    value.to_string(),
                ],
                Some(std::time::Duration::from_secs(10)),
                None,
            )
            .await?;
        let _ = output;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn density_restore_prefers_override() {
        let previous = "Physical density: 420\nOverride density: 560\n";
        let override_density = previous.lines().find_map(|line| {
            line.trim()
                .strip_prefix("Override density:")
                .map(str::trim)
                .and_then(|value| value.parse::<u32>().ok())
        });
        assert_eq!(override_density, Some(560));
    }
}
