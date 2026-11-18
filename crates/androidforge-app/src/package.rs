use std::time::{Duration, Instant};

use androidforge_adb::{AdbClient, ForegroundApp};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use serde::{Deserialize, Serialize};

use crate::apk::ApkInspector;
use crate::install::{InstallOptions, InstallResult, InstalledPackage, install, installed_package};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pids: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    pub alive: bool,
    pub detection: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStates {
    pub package: String,
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<i64>,
    pub debuggable: bool,
    pub process: ProcessState,
    pub process_exists: bool,
    pub process_alive: bool,
    pub task_exists: bool,
    pub activity_exists: bool,
    pub foreground: bool,
    pub resumed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_app: Option<ForegroundApp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_resumed_activity: Option<String>,
    pub task_detection: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl AppStates {
    pub fn summary(&self) -> String {
        format!(
            "{} installed={} process_exists={} process_alive={} task={} activity={} foreground={} resumed={}",
            self.package,
            self.installed,
            self.process_exists,
            self.process_alive,
            self.task_exists,
            self.activity_exists,
            self.foreground,
            self.resumed
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppStateExpectation {
    Installed,
    NotInstalled,
    ProcessRunning,
    ProcessStopped,
    ProcessAlive,
    TaskExists,
    TaskAbsent,
    Foreground,
    Background,
    Resumed,
}

impl AppStateExpectation {
    pub fn matches(&self, states: &AppStates) -> bool {
        match self {
            Self::Installed => states.installed,
            Self::NotInstalled => !states.installed,
            Self::ProcessRunning => states.process_exists,
            Self::ProcessStopped => !states.process_exists,
            Self::ProcessAlive => states.process_alive,
            Self::TaskExists => states.task_exists,
            Self::TaskAbsent => !states.task_exists,
            Self::Foreground => states.foreground,
            Self::Background => !states.foreground,
            Self::Resumed => states.resumed,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::NotInstalled => "not_installed",
            Self::ProcessRunning => "process_running",
            Self::ProcessStopped => "process_stopped",
            Self::ProcessAlive => "process_alive",
            Self::TaskExists => "task_exists",
            Self::TaskAbsent => "task_absent",
            Self::Foreground => "foreground",
            Self::Background => "background",
            Self::Resumed => "resumed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentExtra {
    /// Android `am` extra flag without the leading `--`, for example `es` or `ei`.
    pub kind: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Default)]
pub struct IntentSpec {
    pub package: Option<String>,
    pub component: Option<String>,
    pub action: Option<String>,
    pub data: Option<String>,
    pub mime_type: Option<String>,
    pub categories: Vec<String>,
    pub extras: Vec<IntentExtra>,
    pub flags: Vec<String>,
    pub receiver_permission: Option<String>,
    pub asynchronous: bool,
    pub wait: bool,
    pub force_stop: bool,
    pub wake: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub output: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

fn append_intent_selector(args: &mut Vec<String>, spec: &IntentSpec, _service: bool) {
    if let Some(component) = &spec.component {
        args.push("-n".to_string());
        args.push(component.clone());
    }
    if let Some(action) = &spec.action {
        args.push("-a".to_string());
        args.push(action.clone());
    }
    if let Some(data) = &spec.data {
        args.push("-d".to_string());
        args.push(data.clone());
    }
    if let Some(mime) = &spec.mime_type {
        args.push("-t".to_string());
        args.push(mime.clone());
    }
    for category in &spec.categories {
        args.push("-c".to_string());
        args.push(category.clone());
    }
    for extra in &spec.extras {
        args.push(format!("--{}", extra.kind));
        args.push(extra.key.clone());
        args.push(extra.value.clone());
    }
    for flag in &spec.flags {
        args.push("-f".to_string());
        args.push(flag.clone());
    }
    if let Some(permission) = &spec.receiver_permission {
        args.push("--receiver-permission".to_string());
        args.push(permission.clone());
    }
    if spec.asynchronous {
        args.push("--async".to_string());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppLaunch {
    pub package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_time_ms: Option<u64>,
    #[serde(default)]
    pub foreground: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_app: Option<ForegroundApp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<InstalledPackage>,
    pub states: AppStates,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher_activity: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AppManager {
    pub adb: AdbClient,
    pub inspector: ApkInspector,
    pub install_timeout: Duration,
    pub launch_timeout: Duration,
}

impl AppManager {
    pub fn new(
        adb: AdbClient,
        inspector: ApkInspector,
        install_timeout: Duration,
        launch_timeout: Duration,
    ) -> Self {
        Self {
            adb,
            inspector,
            install_timeout,
            launch_timeout,
        }
    }

    pub async fn install(&self, serial: &str, options: &InstallOptions) -> Result<InstallResult> {
        let mut options = options.clone();
        options.timeout = self.install_timeout;
        install(&self.adb, &self.inspector, serial, &options).await
    }

    pub async fn uninstall(&self, serial: &str, package: &str, keep_data: bool) -> Result<()> {
        let mut args = vec!["uninstall".to_string()];
        if keep_data {
            args.push("-k".to_string());
        }
        args.push(package.to_string());
        let output = self
            .adb
            .run_checked(Some(serial), &args, Some(Duration::from_secs(120)), None)
            .await?;
        let text = output.combined_text();
        if !text.contains("Success") {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                format!("device did not confirm uninstall of {package}"),
            )
            .with_context_string("output", truncate(text.trim(), 3000))
            .device(serial.to_string()));
        }
        if installed_package(&self.adb, serial, package)
            .await?
            .is_some()
        {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                format!("package {package} is still installed after uninstall"),
            )
            .device(serial.to_string()));
        }
        Ok(())
    }

    pub async fn clear_data(&self, serial: &str, package: &str) -> Result<String> {
        let output = self
            .adb
            .shell_args(
                serial,
                &["pm".to_string(), "clear".to_string(), package.to_string()],
                Some(Duration::from_secs(30)),
                Some(1024 * 1024),
            )
            .await?;
        let text = output.combined_text();
        if !text.contains("Success") {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                format!("device did not clear data for {package}"),
            )
            .with_context_string("output", truncate(text.trim(), 3000))
            .device(serial.to_string()));
        }
        Ok(text.trim().to_string())
    }

    pub async fn launch(
        &self,
        serial: &str,
        package: &str,
        activity: Option<&str>,
        wake: bool,
    ) -> Result<AppLaunch> {
        if installed_package(&self.adb, serial, package)
            .await?
            .is_none()
        {
            return Err(ForgeError::new(
                ErrorCode::PackageNotFound,
                format!("package {package} is not installed"),
            )
            .with_suggestion("Install the application with `androidforge app install <apk>`.")
            .device(serial.to_string()));
        }
        let mut warnings = Vec::new();
        if wake && let Err(error) = self.adb.wake(serial).await {
            warnings.push(format!("device wake failed: {}", error.message()));
        }
        let component = match activity {
            Some(activity) => Some(normalize_component(package, activity)),
            None => self.resolve_launcher(serial, package).await,
        };
        let (status, total_time_ms, output_text) = if let Some(component) = &component {
            // A package can be installed a fraction of a second before its
            // launcher alias is ready. One bounded retry turns that transient
            // `am start` failure into a warning instead of a false red run.
            let mut last_output = String::new();
            let mut last_error = None;
            let mut started = None;
            for attempt in 1..=2 {
                match self
                    .adb
                    .shell_args(
                        serial,
                        &[
                            "am".to_string(),
                            "start".to_string(),
                            "-W".to_string(),
                            "-n".to_string(),
                            component.clone(),
                        ],
                        Some(self.launch_timeout),
                        Some(2 * 1024 * 1024),
                    )
                    .await
                {
                    Ok(output) => {
                        let text = output.combined_text();
                        let status = find_value(&text, "Status:");
                        let total_time = find_value(&text, "TotalTime:")
                            .and_then(|value| value.trim_end_matches("ms").parse::<u64>().ok());
                        last_output = text.clone();
                        if status.as_deref() == Some("ok")
                            || text.contains("Warning: Activity not started")
                        {
                            if attempt > 1 {
                                warnings.push(format!(
                                    "am start succeeded on attempt {attempt} after a transient failure"
                                ));
                            }
                            started = Some((status, total_time, text));
                            break;
                        }
                        last_error = Some(format!("am start did not report success for {package}"));
                    }
                    Err(error) => {
                        last_output = error.message().to_string();
                        last_error = Some(error.message().to_string());
                    }
                }
                if attempt == 1 {
                    warnings.push(
                        "am start failed once; retrying after the package/launcher settles"
                            .to_string(),
                    );
                    tokio::time::sleep(Duration::from_millis(700)).await;
                }
            }
            let Some(started) = started else {
                return Err(ForgeError::new(
                    ErrorCode::CommandFailed,
                    format!("am start did not report success for {package} after 2 attempts"),
                )
                .with_context_string("output", truncate(last_output.trim(), 4000))
                .with_context_string(
                    "last_error",
                    last_error.unwrap_or_else(|| "unknown am start failure".to_string()),
                )
                .with_context("attempts", 2)
                .device(serial.to_string()));
            };
            started
        } else {
            warnings.push(
                "could not resolve a launcher activity; falling back to monkey. The exact component is unknown."
                    .to_string(),
            );
            let output = self
                .adb
                .shell_args(
                    serial,
                    &[
                        "monkey".to_string(),
                        "-p".to_string(),
                        package.to_string(),
                        "-c".to_string(),
                        "android.intent.category.LAUNCHER".to_string(),
                        "1".to_string(),
                    ],
                    Some(self.launch_timeout),
                    Some(2 * 1024 * 1024),
                )
                .await?;
            let text = output.combined_text();
            if text.contains("** No activities found") || text.contains("monkey aborted") {
                return Err(ForgeError::new(
                    ErrorCode::CommandFailed,
                    format!("no launchable activity was found for {package}"),
                )
                .with_context_string("output", truncate(text.trim(), 4000))
                .with_suggestion(
                    "Pass --activity <component> if the app uses a nonstandard launcher.",
                )
                .device(serial.to_string()));
            }
            (None, None, text)
        };

        let started = Instant::now();
        let mut foreground_app = None;
        while started.elapsed() < self.launch_timeout {
            foreground_app = self.adb.current_focus(serial).await;
            if foreground_app
                .as_ref()
                .map(|app| app.package == package)
                .unwrap_or(false)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let foreground = foreground_app
            .as_ref()
            .map(|app| app.package == package)
            .unwrap_or(false);
        if !foreground {
            let focused = foreground_app
                .as_ref()
                .map(|app| app.package.clone())
                .unwrap_or_else(|| "unknown".to_string());
            warnings.push(format!(
                "app launch completed but {package} is not the current focus (focus: {focused})"
            ));
        }
        if output_text.trim().is_empty() {
            warnings.push("launch command produced no output".to_string());
        }
        Ok(AppLaunch {
            package: package.to_string(),
            component,
            action: None,
            data: None,
            status,
            total_time_ms,
            foreground,
            foreground_app,
            warnings,
        })
    }

    pub async fn start_intent(&self, serial: &str, spec: &IntentSpec) -> Result<AppLaunch> {
        let package = spec.package.clone();
        if let Some(package) = &package
            && installed_package(&self.adb, serial, package)
                .await?
                .is_none()
        {
            return Err(ForgeError::new(
                ErrorCode::PackageNotFound,
                format!("package {package} is not installed"),
            )
            .with_suggestion("Install the application with `androidforge app install <apk>`.")
            .device(serial.to_string()));
        }
        let mut warnings = Vec::new();
        if spec.wake
            && let Err(error) = self.adb.wake(serial).await
        {
            warnings.push(format!("device wake failed: {}", error.message()));
        }
        if spec.force_stop
            && let Some(package) = &package
        {
            self.force_stop(serial, package).await?;
        }
        let mut args = vec!["am".to_string(), "start".to_string()];
        if spec.wait {
            args.push("-W".to_string());
        }
        if let Some(component) = &spec.component {
            args.push("-n".to_string());
            args.push(component.clone());
        }
        if let Some(action) = &spec.action {
            args.push("-a".to_string());
            args.push(action.clone());
        }
        if let Some(data) = &spec.data {
            args.push("-d".to_string());
            args.push(data.clone());
        }
        if let Some(mime) = &spec.mime_type {
            args.push("-t".to_string());
            args.push(mime.clone());
        }
        for category in &spec.categories {
            args.push("-c".to_string());
            args.push(category.clone());
        }
        for extra in &spec.extras {
            args.push(format!("--{}", extra.kind));
            args.push(extra.key.clone());
            args.push(extra.value.clone());
        }
        for flag in &spec.flags {
            args.push("-f".to_string());
            args.push(flag.clone());
        }
        let output = self
            .adb
            .shell_args(
                serial,
                &args,
                Some(self.launch_timeout),
                Some(2 * 1024 * 1024),
            )
            .await?;
        let text = output.combined_text();
        let status = find_value(&text, "Status:");
        let total_time_ms = find_value(&text, "TotalTime:")
            .and_then(|value| value.trim_end_matches("ms").parse::<u64>().ok());
        let success = status.as_deref() == Some("ok")
            || text.contains("Warning: Activity not started")
            || text.contains("Starting: Intent");
        if !success {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                format!(
                    "am start did not report success{}",
                    spec.action
                        .as_deref()
                        .map(|action| format!(" for {action}"))
                        .unwrap_or_default()
                ),
            )
            .with_context_string("output", truncate(text.trim(), 4000))
            .device(serial.to_string()));
        }
        let foreground_app = self.adb.current_focus(serial).await;
        let foreground = package
            .as_ref()
            .map(|package| {
                foreground_app
                    .as_ref()
                    .is_some_and(|app| &app.package == package)
            })
            .unwrap_or(true);
        if package.is_some() && !foreground {
            warnings.push(format!(
                "intent completed but the requested package is not the current focus (focus: {})",
                foreground_app
                    .as_ref()
                    .map(|app| app.package.clone())
                    .unwrap_or_else(|| "unknown".to_string())
            ));
        }
        Ok(AppLaunch {
            package: package.unwrap_or_else(|| "intent".to_string()),
            component: spec.component.clone(),
            action: spec.action.clone(),
            data: spec.data.clone(),
            status,
            total_time_ms,
            foreground,
            foreground_app,
            warnings,
        })
    }

    pub async fn broadcast_intent(&self, serial: &str, spec: &IntentSpec) -> Result<IntentOutput> {
        let mut args = vec!["am".to_string(), "broadcast".to_string()];
        append_intent_selector(&mut args, spec, false);
        let output = self
            .adb
            .shell_args(
                serial,
                &args,
                Some(self.launch_timeout),
                Some(2 * 1024 * 1024),
            )
            .await?;
        let text = output.combined_text();
        Ok(IntentOutput {
            action: spec.action.clone(),
            package: spec.package.clone(),
            component: spec.component.clone(),
            status: find_value(&text, "Broadcast completed:"),
            output: text.trim().to_string(),
            warnings: Vec::new(),
        })
    }

    pub async fn service_intent(
        &self,
        serial: &str,
        spec: &IntentSpec,
        stop: bool,
    ) -> Result<IntentOutput> {
        let mut args = vec![
            "am".to_string(),
            if stop { "stopservice" } else { "startservice" }.to_string(),
        ];
        append_intent_selector(&mut args, spec, true);
        let output = self
            .adb
            .shell_args(
                serial,
                &args,
                Some(self.launch_timeout),
                Some(2 * 1024 * 1024),
            )
            .await?;
        let text = output.combined_text();
        Ok(IntentOutput {
            action: spec.action.clone(),
            package: spec.package.clone(),
            component: spec.component.clone(),
            status: if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            },
            output: text.trim().to_string(),
            warnings: Vec::new(),
        })
    }

    pub async fn resolve_launcher(&self, serial: &str, package: &str) -> Option<String> {
        let output = self
            .adb
            .shell_args(
                serial,
                &[
                    "cmd".to_string(),
                    "package".to_string(),
                    "resolve-activity".to_string(),
                    "--brief".to_string(),
                    package.to_string(),
                ],
                Some(Duration::from_secs(15)),
                Some(1024 * 1024),
            )
            .await
            .ok()?;
        let text = output.stdout_text();
        text.lines()
            .rev()
            .find_map(|line| {
                let line = line.trim();
                (line.contains('/') && line.contains('.')).then(|| line.to_string())
            })
            .or_else(|| {
                // Older devices may print just the component on the last line.
                text.lines()
                    .map(str::trim)
                    .find(|line| line.contains('/') && line.contains('.'))
                    .map(str::to_string)
            })
    }

    pub async fn force_stop(&self, serial: &str, package: &str) -> Result<()> {
        self.adb
            .shell_args(
                serial,
                &[
                    "am".to_string(),
                    "force-stop".to_string(),
                    package.to_string(),
                ],
                Some(Duration::from_secs(20)),
                Some(1024 * 1024),
            )
            .await?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            if self.pids(serial, package).await.is_empty() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let states = self.status(serial, package).await?;
        Err(ForgeError::new(
            ErrorCode::CommandFailed,
            format!("am force-stop completed but {package} still has a process"),
        )
        .with_context(
            "states",
            serde_json::to_value(&states).unwrap_or(serde_json::Value::Null),
        )
        .with_suggestion("Inspect the process with `androidforge app status <package>`.")
        .device(serial.to_string()))
    }

    pub async fn kill(&self, serial: &str, package: &str) -> Result<()> {
        let output = self
            .adb
            .shell_args(
                serial,
                &["am".to_string(), "kill".to_string(), package.to_string()],
                Some(Duration::from_secs(20)),
                Some(1024 * 1024),
            )
            .await?;
        let text = output.combined_text();
        if text.to_ascii_lowercase().contains("error") {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                format!("am kill failed for {package}"),
            )
            .with_context_string("output", truncate(text.trim(), 3000))
            .device(serial.to_string()));
        }
        // `am kill` only kills background processes. It intentionally does not
        // promise to terminate a foreground app; report the real state instead.
        Ok(())
    }

    pub async fn status(&self, serial: &str, package: &str) -> Result<AppStates> {
        let installed = installed_package(&self.adb, serial, package).await?;
        let pids = self.pids(serial, package).await;
        let process = self.process_state(serial, &pids).await;
        let activity_text = self
            .adb
            .shell_args(
                serial,
                &[
                    "dumpsys".to_string(),
                    "activity".to_string(),
                    "activities".to_string(),
                ],
                Some(Duration::from_secs(30)),
                Some(16 * 1024 * 1024),
            )
            .await
            .map(|output| output.stdout_text())
            .unwrap_or_default();
        let snapshot = activity_snapshot(&activity_text, package);
        let foreground_app = self.adb.current_focus(serial).await;
        let foreground = foreground_app
            .as_ref()
            .map(|app| app.package == package)
            .unwrap_or(false)
            || snapshot
                .top_resumed_package
                .as_deref()
                .map(|value| value == package)
                .unwrap_or(false);
        let resumed = snapshot
            .top_resumed_package
            .as_deref()
            .map(|value| value == package)
            .unwrap_or(false);
        let mut notes = Vec::new();
        if activity_text.is_empty() {
            notes.push(
                "dumpsys activity activities returned no data; task state is best-effort"
                    .to_string(),
            );
        }
        if foreground_app.is_none() {
            notes.push("the current window focus could not be determined".to_string());
        }
        let process_exists = !pids.is_empty();
        let process_alive = process.alive;
        let mut task_exists = snapshot.task_exists;
        let activity_exists = snapshot.activity_exists;
        if activity_exists {
            task_exists = true;
        }
        Ok(AppStates {
            package: package.to_string(),
            installed: installed.is_some(),
            version_name: installed.as_ref().and_then(|pkg| pkg.version_name.clone()),
            version_code: installed.as_ref().and_then(|pkg| pkg.version_code),
            debuggable: installed
                .as_ref()
                .map(|pkg| pkg.debuggable)
                .unwrap_or(false),
            process,
            process_exists,
            process_alive,
            task_exists,
            activity_exists,
            foreground,
            resumed,
            foreground_app,
            top_resumed_activity: snapshot.top_resumed_activity,
            task_detection: "dumpsys_best_effort".to_string(),
            notes,
        })
    }

    pub async fn wait_for_expectation(
        &self,
        serial: &str,
        package: &str,
        expectation: AppStateExpectation,
        timeout: Duration,
    ) -> Result<AppStates> {
        let started = Instant::now();
        loop {
            let states = self.status(serial, package).await?;
            if expectation.matches(&states) {
                return Ok(states);
            }
            if started.elapsed() >= timeout {
                return Err(ForgeError::new(
                    ErrorCode::UiAssertionFailed,
                    format!(
                        "timed out waiting for {} to be {}",
                        package,
                        expectation.as_str()
                    ),
                )
                .with_context(
                    "states",
                    serde_json::to_value(&states).unwrap_or(serde_json::Value::Null),
                )
                .with_context_string("expectation", expectation.as_str())
                .with_suggestion("Inspect the device with `androidforge app status <package>`.")
                .with_suggestion("Capture diagnostics with `androidforge diagnostics collect`.")
                .device(serial.to_string()));
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    pub async fn assert_process_stable(
        &self,
        serial: &str,
        package: &str,
        stable_for: Duration,
    ) -> Result<AppStates> {
        let initial = self.status(serial, package).await?;
        let Some(pid) = initial.process.pid else {
            return Err(ForgeError::new(
                ErrorCode::UiAssertionFailed,
                format!("{package} has no running process"),
            )
            .with_context(
                "states",
                serde_json::to_value(&initial).unwrap_or(serde_json::Value::Null),
            )
            .device(serial.to_string()));
        };
        let started = Instant::now();
        let mut states = initial;
        while started.elapsed() < stable_for {
            tokio::time::sleep(Duration::from_millis(250)).await;
            states = self.status(serial, package).await?;
            if !states.process_alive || states.process.pid != Some(pid) {
                return Err(ForgeError::new(
                    ErrorCode::UiAssertionFailed,
                    format!("process for {package} did not remain stable"),
                )
                .with_context(
                    "states",
                    serde_json::to_value(&states).unwrap_or(serde_json::Value::Null),
                )
                .with_context_string("initial_pid", pid.to_string())
                .with_context_string(
                    "current_pid",
                    states
                        .process
                        .pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                )
                .with_suggestion(
                    "Inspect crash/ANR evidence with `androidforge diagnostics crashes`.",
                )
                .device(serial.to_string()));
            }
        }
        Ok(states)
    }

    pub async fn info(&self, serial: &str, package: &str) -> Result<PackageInfo> {
        let installed = installed_package(&self.adb, serial, package).await?;
        if installed.is_none() {
            return Err(ForgeError::new(
                ErrorCode::PackageNotFound,
                format!("package {package} is not installed"),
            )
            .with_suggestion("Run `androidforge app list` to inspect installed packages.")
            .device(serial.to_string()));
        }
        let states = self.status(serial, package).await?;
        let launcher_activity = self.resolve_launcher(serial, package).await;
        Ok(PackageInfo {
            installed,
            states,
            launcher_activity,
        })
    }

    pub async fn list_packages(
        &self,
        serial: &str,
        third_party_only: bool,
        filter: Option<&str>,
    ) -> Result<Vec<String>> {
        let mut args = vec!["pm".to_string(), "list".to_string(), "packages".to_string()];
        if third_party_only {
            args.push("-3".to_string());
        }
        let output = self
            .adb
            .shell_args(serial, &args, Some(Duration::from_secs(30)), None)
            .await?;
        let mut packages = output
            .stdout_text()
            .lines()
            .filter_map(|line| line.trim().strip_prefix("package:"))
            .filter(|package| {
                filter
                    .map(|filter| package.contains(filter))
                    .unwrap_or(true)
            })
            .map(str::to_string)
            .collect::<Vec<_>>();
        packages.sort();
        Ok(packages)
    }

    async fn pids(&self, serial: &str, package: &str) -> Vec<u32> {
        let output = self
            .adb
            .shell_args(
                serial,
                &["pidof".to_string(), package.to_string()],
                Some(Duration::from_secs(10)),
                None,
            )
            .await;
        match output {
            Ok(output) => output
                .stdout_text()
                .split_whitespace()
                .filter_map(|value| value.parse::<u32>().ok())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    async fn process_state(&self, serial: &str, pids: &[u32]) -> ProcessState {
        let Some(pid) = pids.first().copied() else {
            return ProcessState {
                pids: Vec::new(),
                pid: None,
                state: None,
                alive: false,
                detection: "pidof".to_string(),
            };
        };
        let output = self
            .adb
            .shell_args(
                serial,
                &[
                    "ps".to_string(),
                    "-A".to_string(),
                    "-o".to_string(),
                    "PID,STAT,NAME".to_string(),
                ],
                Some(Duration::from_secs(15)),
                Some(2 * 1024 * 1024),
            )
            .await;
        if let Ok(output) = output {
            for line in output.stdout_text().lines() {
                let mut fields = line.split_whitespace();
                let Some(candidate) = fields.next() else {
                    continue;
                };
                if candidate.parse::<u32>().ok() != Some(pid) {
                    continue;
                }
                let state = fields.next().map(str::to_string);
                let alive = state
                    .as_deref()
                    .map(|state| !state.starts_with('Z'))
                    .unwrap_or(true);
                return ProcessState {
                    pids: pids.to_vec(),
                    pid: Some(pid),
                    state,
                    alive,
                    detection: "ps_pid_stat".to_string(),
                };
            }
            return ProcessState {
                pids: pids.to_vec(),
                pid: Some(pid),
                state: None,
                alive: false,
                detection: "ps_pid_missing".to_string(),
            };
        }
        ProcessState {
            pids: pids.to_vec(),
            pid: Some(pid),
            state: None,
            alive: true,
            detection: "pidof_only_best_effort".to_string(),
        }
    }
}

#[derive(Debug, Default)]
struct ActivitySnapshot {
    task_exists: bool,
    activity_exists: bool,
    top_resumed_package: Option<String>,
    top_resumed_activity: Option<String>,
}

fn activity_snapshot(text: &str, package: &str) -> ActivitySnapshot {
    let mut snapshot = ActivitySnapshot::default();
    let mut task_window = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.contains("Task{") || trimmed.contains("* Task{") {
            task_window = 60;
        }
        if task_window > 0 {
            if trimmed.contains(package) {
                snapshot.task_exists = true;
            }
            task_window -= 1;
        }
        if trimmed.contains("ActivityRecord{") && trimmed.contains(package) {
            snapshot.activity_exists = true;
        }
        if (trimmed.contains("topResumedActivity=") || trimmed.contains("mResumedActivity="))
            && let Some(component) = extract_component(trimmed)
        {
            snapshot.top_resumed_package = Some(component.0);
            snapshot.top_resumed_activity = Some(component.1);
        }
    }
    snapshot
}

fn extract_component(line: &str) -> Option<(String, String)> {
    let token = line
        .split_whitespace()
        .find(|token| token.contains('/') && token.contains('.'))?
        .trim_matches(|ch| matches!(ch, '}' | ')' | ','));
    let (package, activity) = token.split_once('/')?;
    if package.is_empty() {
        return None;
    }
    Some((package.to_string(), format!("{package}/{activity}")))
}

fn normalize_component(package: &str, activity: &str) -> String {
    if activity.contains('/') {
        activity.to_string()
    } else {
        format!("{package}/{activity}")
    }
}

fn find_value(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(key) {
            return Some(rest.trim().to_string());
        }
    }
    None
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
    fn parses_activity_states() {
        let text = r#"
  * Task{taskId=12 realActivity=com.example/.MainActivity}
    * ActivityRecord{abc u0 com.example/.MainActivity t12}
    topResumedActivity=ActivityRecord{abc u0 com.example/.MainActivity t12}
"#;
        let snapshot = activity_snapshot(text, "com.example");
        assert!(snapshot.task_exists);
        assert!(snapshot.activity_exists);
        assert_eq!(snapshot.top_resumed_package.as_deref(), Some("com.example"));
    }

    #[test]
    fn task_absent_for_unrelated_package() {
        let text = "topResumedActivity=ActivityRecord{abc u0 com.other/.MainActivity t12}";
        let snapshot = activity_snapshot(text, "com.example");
        assert!(!snapshot.task_exists);
        assert!(!snapshot.activity_exists);
    }

    #[test]
    fn normalizes_component_names() {
        assert_eq!(
            normalize_component("com.example", ".MainActivity"),
            "com.example/.MainActivity"
        );
        assert_eq!(
            normalize_component("com.example", "com.example.MainActivity"),
            "com.example/com.example.MainActivity"
        );
    }
}
