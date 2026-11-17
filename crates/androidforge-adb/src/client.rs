use std::path::PathBuf;
use std::time::{Duration, Instant};

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::{Device, DeviceKind, DeviceState};
use androidforge_process::{CommandOutput, CommandSpec, ProcessError, Runner, shell_quote};
use serde::{Deserialize, Serialize};

use crate::devices::{
    boot_completed, parse_devices_output, parse_getprop_output, parse_wm_density, parse_wm_size,
};

pub const BOOT_COMPLETED_PROP: &str = "sys.boot_completed";
pub const BOOT_COMPLETE_PROP: &str = "dev.bootcomplete";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForegroundApp {
    pub package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakeResult {
    pub screen_on: Option<bool>,
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WirelessResult {
    pub endpoint: String,
    pub action: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(default)]
    pub already_connected: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone)]
pub struct AdbClient {
    pub adb: PathBuf,
    pub runner: Runner,
    pub default_timeout: Duration,
    pub trace: bool,
}

impl AdbClient {
    pub fn new(adb: PathBuf, trace: bool, default_timeout: Duration) -> Self {
        Self {
            adb,
            runner: Runner::new(trace),
            default_timeout,
            trace,
        }
    }

    pub fn command_line(&self, serial: Option<&str>, args: &[String]) -> String {
        let mut parts = vec![self.adb.to_string_lossy().into_owned()];
        if let Some(serial) = serial {
            parts.push("-s".to_string());
            parts.push(serial.to_string());
        }
        parts.extend(args.iter().cloned());
        androidforge_process::shell_quote_join(&parts)
    }

    pub async fn run_raw(
        &self,
        serial: Option<&str>,
        args: &[String],
        timeout: Option<Duration>,
        max_output: Option<usize>,
    ) -> Result<CommandOutput> {
        let mut all = Vec::with_capacity(args.len() + 2);
        if let Some(serial) = serial {
            all.push("-s".to_string());
            all.push(serial.to_string());
        }
        all.extend(args.iter().cloned());
        let mut spec = CommandSpec::new(self.adb.to_string_lossy().into_owned())
            .args(all)
            .timeout(timeout.unwrap_or(self.default_timeout))
            .trace(self.trace)
            .hide_window(true);
        if let Some(max) = max_output {
            spec = spec.max_stdout(max);
        }
        self.runner
            .run(spec)
            .await
            .map_err(|error| map_process_error(error, serial))
    }

    pub async fn run_checked(
        &self,
        serial: Option<&str>,
        args: &[String],
        timeout: Option<Duration>,
        max_output: Option<usize>,
    ) -> Result<CommandOutput> {
        let output = self.run_raw(serial, args, timeout, max_output).await?;
        if !output.success {
            return Err(map_adb_failure(output, serial, self.adb.as_path()));
        }
        Ok(output)
    }

    pub async fn devices_raw(&self) -> Result<Vec<Device>> {
        let output = self
            .run_checked(
                None,
                &[s("devices"), s("-l")],
                Some(Duration::from_secs(15)),
                None,
            )
            .await?;
        Ok(parse_devices_output(&output.stdout_text()))
    }

    pub async fn list_devices(&self, deep: bool) -> Result<Vec<Device>> {
        let devices = self.devices_raw().await?;
        if !deep {
            return Ok(devices);
        }
        let mut enriched = Vec::with_capacity(devices.len());
        for device in devices {
            if device.state.is_operational() {
                let fallback = device.clone();
                enriched.push(self.enrich(device).await.unwrap_or(fallback));
            } else {
                enriched.push(device);
            }
        }
        Ok(enriched)
    }

    pub async fn inspect(&self, serial: &str) -> Result<Device> {
        let devices = self.devices_raw().await?;
        let device = devices
            .into_iter()
            .find(|device| device.serial == serial)
            .ok_or_else(|| {
                let available = self.devices_raw_sync_hint();
                ForgeError::new(
                    ErrorCode::DeviceNotFound,
                    format!("device serial '{serial}' is not connected"),
                )
                .with_suggestion("Run `androidforge devices list` to see connected devices.")
                .with_suggestion(available)
                .device(serial.to_string())
            })?;
        if !device.state.is_operational() {
            return Ok(device);
        }
        self.enrich(device).await
    }

    fn devices_raw_sync_hint(&self) -> String {
        format!(
            "Check that the device is connected and that `{} devices -l` lists it.",
            self.adb.display()
        )
    }

    pub async fn enrich(&self, mut device: Device) -> Result<Device> {
        if !device.state.is_operational() {
            return Ok(device);
        }
        if let Ok(output) = self
            .shell_args(
                &device.serial,
                &[s("getprop")],
                Some(Duration::from_secs(15)),
                Some(2 * 1024 * 1024),
            )
            .await
        {
            let properties = parse_getprop_output(&output.stdout_text());
            if !properties.is_empty() {
                fill_from_properties(&mut device, &properties);
            }
        }

        if let Ok(output) = self
            .shell_args(
                &device.serial,
                &[s("wm"), s("size")],
                Some(Duration::from_secs(8)),
                None,
            )
            .await
            && let Some((width, height)) = parse_wm_size(&output.stdout_text())
        {
            device.resolution = Some(format!("{width}x{height}"));
        }
        if let Ok(output) = self
            .shell_args(
                &device.serial,
                &[s("wm"), s("density")],
                Some(Duration::from_secs(8)),
                None,
            )
            .await
            && let Some(density) = parse_wm_density(&output.stdout_text())
        {
            device.density = Some(density);
        }

        if let Ok(output) = self
            .shell_args(
                &device.serial,
                &[s("dumpsys"), s("power")],
                Some(Duration::from_secs(8)),
                Some(512 * 1024),
            )
            .await
        {
            device.screen_on = parse_screen_on(&output.stdout_text());
        }
        if let Ok(output) = self
            .shell_args(
                &device.serial,
                &[s("dumpsys"), s("window"), s("policy")],
                Some(Duration::from_secs(8)),
                Some(2 * 1024 * 1024),
            )
            .await
        {
            device.locked = parse_locked(&output.stdout_text());
        }
        Ok(device)
    }

    pub async fn shell_args(
        &self,
        serial: &str,
        args: &[String],
        timeout: Option<Duration>,
        max_output: Option<usize>,
    ) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(ForgeError::invalid_argument(
                "shell command cannot be empty",
            ));
        }
        let remote = args
            .iter()
            .map(|arg| shell_quote(arg))
            .collect::<Vec<_>>()
            .join(" ");
        self.run_checked(Some(serial), &[s("shell"), remote], timeout, max_output)
            .await
    }

    pub async fn shell_args_with_stdin(
        &self,
        serial: &str,
        args: &[String],
        stdin: Vec<u8>,
        timeout: Option<Duration>,
        max_output: Option<usize>,
    ) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(ForgeError::invalid_argument(
                "shell command cannot be empty",
            ));
        }
        let remote = args
            .iter()
            .map(|arg| shell_quote(arg))
            .collect::<Vec<_>>()
            .join(" ");
        let mut all = vec![
            "-s".to_string(),
            serial.to_string(),
            "shell".to_string(),
            remote,
        ];
        let mut spec = CommandSpec::new(self.adb.to_string_lossy().into_owned())
            .args(all.drain(..))
            .timeout(timeout.unwrap_or(self.default_timeout))
            .trace(self.trace)
            .hide_window(true)
            .stdin(stdin);
        if let Some(max) = max_output {
            spec = spec.max_stdout(max);
        }
        self.runner
            .run(spec)
            .await
            .map_err(|error| map_process_error(error, Some(serial)))
    }

    pub async fn shell_text(
        &self,
        serial: &str,
        args: &[String],
        timeout: Option<Duration>,
    ) -> Result<String> {
        Ok(self
            .shell_args(serial, args, timeout, None)
            .await?
            .stdout_text())
    }

    pub async fn exec_out_args(
        &self,
        serial: &str,
        args: &[String],
        timeout: Option<Duration>,
        max_output: Option<usize>,
    ) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(ForgeError::invalid_argument(
                "exec-out command cannot be empty",
            ));
        }
        let remote = args
            .iter()
            .map(|arg| shell_quote(arg))
            .collect::<Vec<_>>()
            .join(" ");
        self.run_checked(Some(serial), &[s("exec-out"), remote], timeout, max_output)
            .await
    }

    pub async fn emu(
        &self,
        serial: &str,
        args: &[String],
        timeout: Option<Duration>,
    ) -> Result<CommandOutput> {
        let mut full = vec![s("emu")];
        full.extend(args.iter().cloned());
        self.run_checked(Some(serial), &full, timeout, None).await
    }

    pub async fn getprop_all(
        &self,
        serial: &str,
    ) -> Result<std::collections::BTreeMap<String, String>> {
        let output = self
            .shell_args(
                serial,
                &[s("getprop")],
                Some(Duration::from_secs(15)),
                Some(2 * 1024 * 1024),
            )
            .await?;
        Ok(parse_getprop_output(&output.stdout_text()))
    }

    pub async fn getprop(&self, serial: &str, key: &str) -> Result<String> {
        Ok(self
            .shell_args(
                serial,
                &[s("getprop"), key.to_string()],
                Some(Duration::from_secs(10)),
                None,
            )
            .await?
            .stdout_text()
            .trim()
            .to_string())
    }

    pub async fn wait_for_device(
        &self,
        serial: Option<&str>,
        require_boot: bool,
        timeout: Duration,
    ) -> Result<Device> {
        let started = Instant::now();
        let mut last_state: Option<DeviceState> = None;
        let mut last_serial: Option<String> = serial.map(str::to_string);
        loop {
            let mut devices = self.devices_raw().await.unwrap_or_default();
            let candidate = if let Some(serial) = serial {
                devices
                    .iter()
                    .position(|device| device.serial == serial)
                    .map(|index| devices.remove(index))
            } else {
                devices
                    .iter()
                    .position(|device| device.state.is_operational())
                    .or_else(|| (!devices.is_empty()).then_some(0))
                    .map(|index| devices.remove(index))
            };
            if let Some(device) = candidate {
                last_state = Some(device.state.clone());
                last_serial = Some(device.serial.clone());
                if device.state.is_operational() {
                    if !require_boot {
                        return self.inspect(&device.serial).await;
                    }
                    if self.is_boot_complete(&device.serial).await {
                        return self.inspect(&device.serial).await;
                    }
                } else if matches!(
                    device.state,
                    DeviceState::Unauthorized | DeviceState::Offline
                ) && !require_boot
                {
                    // Keep waiting. A device that is present but not usable may
                    // become usable (authorization prompt, reconnect).
                }
            }

            if started.elapsed() >= timeout {
                let serial_label = last_serial.as_deref().unwrap_or("(any)");
                let code = match last_state {
                    Some(DeviceState::Unauthorized) => ErrorCode::DeviceUnauthorized,
                    Some(DeviceState::Offline) => ErrorCode::DeviceOffline,
                    Some(DeviceState::Device) => {
                        if serial
                            .map(|serial| serial.starts_with("emulator-"))
                            .unwrap_or(false)
                        {
                            ErrorCode::EmulatorBootTimeout
                        } else {
                            ErrorCode::CommandTimeout
                        }
                    }
                    _ => ErrorCode::DeviceNotFound,
                };
                let message = match code {
                    ErrorCode::EmulatorBootTimeout => format!(
                        "emulator {serial_label} did not finish booting within {} seconds",
                        timeout.as_secs()
                    ),
                    ErrorCode::CommandTimeout => format!(
                        "device {serial_label} was connected but did not become ready within {} seconds",
                        timeout.as_secs()
                    ),
                    ErrorCode::DeviceUnauthorized => format!(
                        "device {serial_label} is unauthorized; approve the USB debugging prompt"
                    ),
                    ErrorCode::DeviceOffline => format!(
                        "device {serial_label} is offline; reconnect it or restart the emulator"
                    ),
                    _ => format!(
                        "device {serial_label} was not available within {} seconds",
                        timeout.as_secs()
                    ),
                };
                return Err(ForgeError::new(code, message)
                    .with_suggestion(
                        "Run `androidforge devices list` to inspect connected devices.",
                    )
                    .with_suggestion("Check the USB connection or emulator process.")
                    .device(serial_label.to_string()));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    pub async fn is_boot_complete(&self, serial: &str) -> bool {
        let Ok(properties) = self.getprop_all(serial).await else {
            return false;
        };
        if !boot_completed(&properties) {
            return false;
        }
        // sys.boot_completed can be set a little before the package manager is
        // ready. Probe the framework package without using shell pipelines.
        self.shell_args(
            serial,
            &[s("pm"), s("path"), s("android")],
            Some(Duration::from_secs(8)),
            None,
        )
        .await
        .map(|output| !output.stdout_text().trim().is_empty())
        .unwrap_or(false)
    }

    pub async fn screen_on(&self, serial: &str) -> Option<bool> {
        let output = self
            .shell_args(
                serial,
                &[s("dumpsys"), s("power")],
                Some(Duration::from_secs(8)),
                Some(512 * 1024),
            )
            .await
            .ok()?;
        parse_screen_on(&output.stdout_text())
    }

    pub async fn keyboard_shown(&self, serial: &str) -> Option<bool> {
        let output = self
            .shell_args(
                serial,
                &[s("dumpsys"), s("input_method")],
                Some(Duration::from_secs(8)),
                Some(2 * 1024 * 1024),
            )
            .await
            .ok()?;
        parse_keyboard_shown(&output.stdout_text())
    }

    pub async fn hide_keyboard(&self, serial: &str) -> Result<bool> {
        let was_visible = self.keyboard_shown(serial).await.unwrap_or(false);
        if !was_visible {
            return Ok(false);
        }
        self.keyevent(serial, "KEYCODE_BACK", None).await?;
        tokio::time::sleep(Duration::from_millis(450)).await;
        if self.keyboard_shown(serial).await == Some(true) {
            return Err(ForgeError::new(
                ErrorCode::ImeVisible,
                "the soft keyboard is still visible after pressing Back",
            )
            .with_suggestion("Retry the hide-keyboard operation or dismiss the IME manually.")
            .device(serial.to_string()));
        }
        Ok(true)
    }

    pub async fn keyevent(
        &self,
        serial: &str,
        key: &str,
        timeout: Option<Duration>,
    ) -> Result<CommandOutput> {
        self.shell_args(
            serial,
            &[s("input"), s("keyevent"), key.to_string()],
            timeout,
            None,
        )
        .await
    }

    pub async fn input_text(&self, serial: &str, encoded: &str) -> Result<CommandOutput> {
        self.shell_args(
            serial,
            &[s("input"), s("text"), encoded.to_string()],
            Some(Duration::from_secs(20)),
            None,
        )
        .await
    }

    pub async fn input_tap(&self, serial: &str, x: u32, y: u32) -> Result<CommandOutput> {
        self.shell_args(
            serial,
            &[s("input"), s("tap"), x.to_string(), y.to_string()],
            Some(Duration::from_secs(15)),
            None,
        )
        .await
    }

    pub async fn input_swipe(
        &self,
        serial: &str,
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u32,
    ) -> Result<CommandOutput> {
        self.shell_args(
            serial,
            &[
                s("input"),
                s("swipe"),
                x1.to_string(),
                y1.to_string(),
                x2.to_string(),
                y2.to_string(),
                duration_ms.to_string(),
            ],
            Some(Duration::from_secs(20)),
            None,
        )
        .await
    }

    pub async fn wake(&self, serial: &str) -> Result<WakeResult> {
        let mut notes = Vec::new();
        let screen_on = self.screen_on(serial).await;
        if screen_on == Some(false) {
            self.keyevent(serial, "KEYCODE_WAKEUP", None).await?;
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
        if let Err(error) = self
            .shell_args(
                serial,
                &[s("wm"), s("dismiss-keyguard")],
                Some(Duration::from_secs(10)),
                None,
            )
            .await
        {
            notes.push(format!("wm dismiss-keyguard failed: {}", error.message()));
        }

        let mut locked = self
            .shell_args(
                serial,
                &[s("dumpsys"), s("window"), s("policy")],
                Some(Duration::from_secs(8)),
                Some(2 * 1024 * 1024),
            )
            .await
            .ok()
            .and_then(|output| parse_locked(&output.stdout_text()));
        if locked == Some(true) {
            let _ = self.keyevent(serial, "KEYCODE_MENU", None).await;
            tokio::time::sleep(Duration::from_millis(250)).await;
            let size = self.display_size(serial).await.unwrap_or((1080, 2400));
            let _ = self
                .input_swipe(
                    serial,
                    size.0 / 2,
                    (size.1 as f64 * 0.82) as u32,
                    size.0 / 2,
                    (size.1 as f64 * 0.18) as u32,
                    300,
                )
                .await;
            tokio::time::sleep(Duration::from_millis(350)).await;
            locked = self
                .shell_args(
                    serial,
                    &[s("dumpsys"), s("window"), s("policy")],
                    Some(Duration::from_secs(8)),
                    Some(2 * 1024 * 1024),
                )
                .await
                .ok()
                .and_then(|output| parse_locked(&output.stdout_text()));
        }
        if locked == Some(true) {
            notes.push(
                "device still reports a locked keyguard; app launch may require manual device configuration"
                    .to_string(),
            );
        }
        Ok(WakeResult {
            screen_on: self.screen_on(serial).await,
            locked,
            notes,
        })
    }

    pub async fn display_size(&self, serial: &str) -> Option<(u32, u32)> {
        let output = self
            .shell_args(
                serial,
                &[s("wm"), s("size")],
                Some(Duration::from_secs(8)),
                None,
            )
            .await
            .ok()?;
        parse_wm_size(&output.stdout_text())
    }

    pub async fn current_focus(&self, serial: &str) -> Option<ForegroundApp> {
        let output = self
            .shell_args(
                serial,
                &[s("dumpsys"), s("window")],
                Some(Duration::from_secs(10)),
                Some(2 * 1024 * 1024),
            )
            .await
            .ok()?;
        parse_foreground(&output.stdout_text())
    }

    pub async fn settings_get(&self, serial: &str, namespace: &str, key: &str) -> Result<String> {
        Ok(self
            .shell_args(
                serial,
                &[
                    s("settings"),
                    s("get"),
                    namespace.to_string(),
                    key.to_string(),
                ],
                Some(Duration::from_secs(10)),
                None,
            )
            .await?
            .stdout_text()
            .trim()
            .to_string())
    }

    pub async fn settings_put(
        &self,
        serial: &str,
        namespace: &str,
        key: &str,
        value: &str,
    ) -> Result<()> {
        self.shell_args(
            serial,
            &[
                s("settings"),
                s("put"),
                namespace.to_string(),
                key.to_string(),
                value.to_string(),
            ],
            Some(Duration::from_secs(10)),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn version(&self) -> Result<String> {
        let output = self
            .run_checked(None, &[s("version")], Some(Duration::from_secs(10)), None)
            .await?;
        Ok(output
            .stdout_text()
            .lines()
            .next()
            .unwrap_or("unknown")
            .trim()
            .to_string())
    }

    pub async fn start_server(&self) -> Result<()> {
        self.run_checked(
            None,
            &[s("start-server")],
            Some(Duration::from_secs(30)),
            None,
        )
        .await?;
        Ok(())
    }

    pub fn validate_endpoint(endpoint: &str) -> Result<String> {
        let endpoint = endpoint.trim();
        let (host, port) = endpoint.rsplit_once(':').ok_or_else(|| {
            ForgeError::new(
                ErrorCode::WirelessEndpointInvalid,
                format!("wireless endpoint '{endpoint}' must be HOST:PORT"),
            )
            .with_suggestion(
                "Use the endpoint shown on the device, for example 192.168.1.20:37521.",
            )
        })?;
        let host = host.trim();
        if host.is_empty()
            || host.chars().any(char::is_whitespace)
            || matches!(host, "*" | "0.0.0.0" | "::")
        {
            return Err(ForgeError::new(
                ErrorCode::WirelessEndpointInvalid,
                format!("wireless endpoint host '{host}' is not a specific device address"),
            )
            .with_suggestion("Use the device's LAN address; AndroidForge does not expose ADB on public interfaces."));
        }
        let port = port.trim().parse::<u16>().map_err(|_| {
            ForgeError::new(
                ErrorCode::WirelessEndpointInvalid,
                format!("wireless endpoint port '{port}' is not a valid TCP port"),
            )
        })?;
        if port == 0 {
            return Err(ForgeError::new(
                ErrorCode::WirelessEndpointInvalid,
                "wireless endpoint port cannot be 0",
            ));
        }
        Ok(format!("{host}:{port}"))
    }

    async fn run_secret(&self, args: &[String], timeout: Duration) -> Result<CommandOutput> {
        // Never trace pairing argv: ADB requires the pairing code as an
        // argument, and trace output would persist it in logs.
        let spec = CommandSpec::new(self.adb.to_string_lossy().into_owned())
            .args(args.iter().cloned())
            .timeout(timeout)
            .trace(false)
            .hide_window(true)
            .max_stdout(1024 * 1024)
            .max_stderr(1024 * 1024);
        Runner::new(false)
            .run(spec)
            .await
            .map_err(|error| map_process_error(error, None))
    }

    fn pairing_succeeded(output: &CommandOutput) -> bool {
        let combined = output.combined_text().to_ascii_lowercase();
        output.success
            && (combined.contains("successfully paired") || combined.contains("already paired"))
    }

    fn connect_succeeded(output: &CommandOutput) -> bool {
        let lower = output.combined_text().to_ascii_lowercase();
        output.success && (lower.contains("connected to") || lower.contains("already connected"))
    }

    fn redact_secret(output: &str, secret: &str) -> String {
        if secret.is_empty() {
            output.to_string()
        } else {
            output.replace(secret, "[REDACTED]")
        }
    }

    /// Pair with an Android wireless-debugging endpoint. The pairing code is
    /// never included in output, JSON, errors or trace logs.
    pub async fn pair(&self, endpoint: &str, code: &str) -> Result<WirelessResult> {
        let endpoint = Self::validate_endpoint(endpoint)?;
        let code = code.trim();
        if code.len() != 6 || !code.chars().all(|ch| ch.is_ascii_digit()) {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                "wireless pairing codes are six digits",
            )
            .with_suggestion("Read the six-digit code from the device's Wireless debugging screen.")
            .with_suggestion("Prefer --code-stdin to keep the code out of shell history."));
        }
        let started = Instant::now();
        let args = vec![s("pair"), endpoint.clone(), code.to_string()];
        let output = self.run_secret(&args, Duration::from_secs(60)).await?;
        let stdout = Self::redact_secret(&output.stdout_text(), code);
        let stderr = Self::redact_secret(&output.stderr_text(), code);
        let combined = format!("{}\n{}", stdout, stderr);
        let success = Self::pairing_succeeded(&output);
        if !success {
            let mut error = ForgeError::new(
                ErrorCode::WirelessPairingFailed,
                format!("ADB could not pair with {endpoint}"),
            )
            .with_context_string("endpoint", endpoint.clone())
            .with_suggestion(
                "Confirm the pairing endpoint and code on the device's Wireless debugging screen.",
            )
            .with_suggestion("The pairing port is different from the connection port.")
            .with_suggestion("Pairing codes are never stored or logged.");
            if !combined.trim().is_empty() {
                error = error.with_context_string("adb_output", truncate_context(combined.trim()));
            }
            return Err(error);
        }
        Ok(WirelessResult {
            endpoint,
            action: "pair".to_string(),
            success: true,
            message: Some(truncate_context(combined.trim())),
            serial: None,
            already_connected: false,
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }

    pub async fn connect(&self, endpoint: &str) -> Result<WirelessResult> {
        let endpoint = Self::validate_endpoint(endpoint)?;
        let started = Instant::now();
        let output = self
            .run_secret(&[s("connect"), endpoint.clone()], Duration::from_secs(30))
            .await?;
        let text = output.combined_text();
        let lower = text.to_ascii_lowercase();
        let already_connected = lower.contains("already connected");
        let success = Self::connect_succeeded(&output);
        if !success {
            return Err(ForgeError::new(
                ErrorCode::WirelessConnectFailed,
                format!("ADB could not connect to {endpoint}"),
            )
            .with_context_string("endpoint", endpoint.clone())
            .with_context_string("adb_output", truncate_context(text.trim()))
            .with_suggestion("Confirm the device is paired and the connect endpoint is correct.")
            .with_suggestion("The connect port changes when wireless debugging restarts."));
        }
        Ok(WirelessResult {
            endpoint: endpoint.clone(),
            action: "connect".to_string(),
            success: true,
            message: Some(truncate_context(text.trim())),
            serial: Some(endpoint),
            already_connected,
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }

    pub async fn disconnect(&self, endpoint: &str) -> Result<WirelessResult> {
        let endpoint = Self::validate_endpoint(endpoint)?;
        let started = Instant::now();
        let output = self
            .run_secret(
                &[s("disconnect"), endpoint.clone()],
                Duration::from_secs(15),
            )
            .await?;
        let text = output.combined_text();
        if !output.success {
            return Err(ForgeError::new(
                ErrorCode::WirelessConnectFailed,
                format!("ADB could not disconnect {endpoint}"),
            )
            .with_context_string("endpoint", endpoint)
            .with_context_string("adb_output", truncate_context(text.trim())));
        }
        Ok(WirelessResult {
            endpoint,
            action: "disconnect".to_string(),
            success: true,
            message: Some(truncate_context(text.trim())),
            serial: None,
            already_connected: false,
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }

    pub async fn raw_passthrough(
        &self,
        serial: Option<&str>,
        args: &[String],
        timeout: Option<Duration>,
        max_output: Option<usize>,
    ) -> Result<CommandOutput> {
        self.run_raw(serial, args, timeout, max_output).await
    }

    pub async fn dumpsys(&self, serial: &str, service: &str) -> Result<String> {
        Ok(self
            .shell_args(
                serial,
                &[s("dumpsys"), service.to_string()],
                Some(Duration::from_secs(30)),
                Some(16 * 1024 * 1024),
            )
            .await?
            .stdout_text())
    }

    pub async fn pull(&self, serial: &str, remote: &str, local: &std::path::Path) -> Result<()> {
        self.run_checked(
            Some(serial),
            &[
                s("pull"),
                remote.to_string(),
                local.to_string_lossy().into_owned(),
            ],
            Some(Duration::from_secs(120)),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn push(&self, serial: &str, local: &std::path::Path, remote: &str) -> Result<()> {
        self.run_checked(
            Some(serial),
            &[
                s("push"),
                local.to_string_lossy().into_owned(),
                remote.to_string(),
            ],
            Some(Duration::from_secs(120)),
            None,
        )
        .await?;
        Ok(())
    }
}

fn s(value: &str) -> String {
    value.to_string()
}

fn fill_from_properties(
    device: &mut Device,
    properties: &std::collections::BTreeMap<String, String>,
) {
    device.properties = properties.clone();
    device.model = properties
        .get("ro.product.model")
        .cloned()
        .or_else(|| device.model.clone());
    device.manufacturer = properties.get("ro.product.manufacturer").cloned();
    device.android_version = properties.get("ro.build.version.release").cloned();
    device.api_level = properties
        .get("ro.build.version.sdk")
        .and_then(|value| value.parse::<u32>().ok());
    device.architecture = properties.get("ro.product.cpu.abi").cloned();
    device.product = properties
        .get("ro.product.name")
        .cloned()
        .or_else(|| device.product.clone());
    let booted = boot_completed(properties);
    device.boot_completed = Some(booted);
    device.boot_state = Some(if booted {
        "completed".to_string()
    } else if properties
        .get("init.svc.bootanim")
        .map(|value| value == "running")
        .unwrap_or(false)
    {
        "boot-animation".to_string()
    } else {
        "booting".to_string()
    });
    if let Some(kind) = properties.get("ro.kernel.qemu")
        && kind == "1"
    {
        device.kind = DeviceKind::Emulator;
    }
}

fn parse_screen_on(output: &str) -> Option<bool> {
    for line in output.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("mWakefulness=") {
            return Some(match value.trim() {
                "Awake" | "Dreaming" => true,
                "Asleep" | "Dozing" => false,
                _ => return None,
            });
        }
        if let Some(value) = line.strip_prefix("Display Power: state=") {
            return Some(matches!(value.trim(), "ON" | "ON_SUSPEND" | "DOZE"));
        }
    }
    None
}

fn parse_locked(output: &str) -> Option<bool> {
    let mut saw_keyguard = false;
    let mut occluded = false;
    let mut locked = false;
    for line in output.lines() {
        let line = line.trim();
        for key in [
            "mShowingLockscreen=",
            "isStatusBarKeyguard=",
            "mKeyguardShowing=",
            "mDreamingLockscreen=",
        ] {
            if let Some(rest) = line.strip_prefix(key) {
                saw_keyguard = true;
                if rest.trim().eq_ignore_ascii_case("true") {
                    locked = true;
                }
            }
        }
        if let Some(rest) = line.strip_prefix("mKeyguardOccluded=") {
            occluded = rest.trim().eq_ignore_ascii_case("true");
        }
    }
    if !saw_keyguard {
        return None;
    }
    if occluded {
        // The keyguard is occluded by an app, so an app interaction can proceed.
        Some(false)
    } else {
        Some(locked)
    }
}

fn parse_keyboard_shown(output: &str) -> Option<bool> {
    let mut shown = None;
    for line in output.lines() {
        let line = line.trim();
        for key in ["mInputShown=", "mIsInputViewShown="] {
            if let Some(value) = line.strip_prefix(key) {
                let value = value.trim();
                if value.eq_ignore_ascii_case("true") {
                    return Some(true);
                }
                if value.eq_ignore_ascii_case("false") {
                    shown = Some(false);
                }
            }
        }
    }
    shown
}

fn parse_foreground(output: &str) -> Option<ForegroundApp> {
    let mut fallback = None;
    for line in output.lines() {
        let line = line.trim();
        let relevant = [
            "mCurrentFocus=",
            "mFocusedApp=",
            "topResumedActivity=",
            "mResumedActivity=",
        ]
        .iter()
        .any(|marker| line.contains(marker));
        if !relevant {
            continue;
        }
        if let Some((package, activity)) = component_in_line(line) {
            let app = ForegroundApp {
                package,
                activity: Some(activity),
                window: Some(line.to_string()),
            };
            if line.contains("mCurrentFocus=") {
                return Some(app);
            }
            if fallback.is_none() {
                fallback = Some(app);
            }
        }
    }
    fallback
}

fn component_in_line(line: &str) -> Option<(String, String)> {
    let token = line
        .split_whitespace()
        .rev()
        .find(|token| token.contains('/') && token.contains('.'))?
        .trim_matches(|ch| matches!(ch, '}' | ')' | ','));
    let (package, activity) = token.split_once('/')?;
    let package = package.trim().to_string();
    if package.is_empty() {
        return None;
    }
    Some((package, activity.trim().to_string()))
}

fn map_process_error(error: ProcessError, serial: Option<&str>) -> ForgeError {
    match error {
        ProcessError::Spawn { source, .. } => ForgeError::new(
            ErrorCode::AdbNotFound,
            format!("could not start adb: {source}"),
        )
        .with_suggestion("Install Android SDK Platform-Tools or set ANDROIDFORGE_ADB.")
        .with_suggestion("Run `androidforge doctor` to inspect tool discovery."),
        ProcessError::Timeout {
            timeout_ms, stderr, ..
        } => {
            let mut forge = ForgeError::new(
                ErrorCode::CommandTimeout,
                format!("ADB command timed out after {timeout_ms} ms"),
            )
            .with_suggestion("Retry with a larger timeout after checking the device state.");
            if let Some(serial) = serial {
                forge = forge.device(serial.to_string());
            }
            let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
            if !stderr.is_empty() {
                forge = forge.with_context_string("stderr", truncate_context(&stderr));
            }
            forge
        }
        other => {
            let mut forge = ForgeError::from(other);
            if let Some(serial) = serial {
                forge = forge.device(serial.to_string());
            }
            forge
        }
    }
}

fn map_adb_failure(
    output: CommandOutput,
    serial: Option<&str>,
    adb: &std::path::Path,
) -> ForgeError {
    let stderr = output.stderr_text();
    let stdout = output.stdout_text();
    let combined = format!("{}\n{}", stdout, stderr).to_ascii_lowercase();
    let code = if combined.contains("device offline") {
        ErrorCode::DeviceOffline
    } else if combined.contains("unauthorized") {
        ErrorCode::DeviceUnauthorized
    } else if combined.contains("device not found")
        || combined.contains("no devices/emulators found")
    {
        ErrorCode::DeviceNotFound
    } else if combined.contains("more than one device") {
        ErrorCode::DeviceAmbiguous
    } else if combined.contains("connection reset")
        || combined.contains("closed")
        || combined.contains("device disconnected")
    {
        ErrorCode::DeviceDisconnected
    } else if combined.contains("permission denied") {
        ErrorCode::PermissionDenied
    } else {
        ErrorCode::CommandFailed
    };
    let exit = output
        .exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let message = format!("ADB command failed with exit code {exit}");
    let mut forge = ForgeError::new(code, message)
        .with_context_string("adb", adb.to_string_lossy().into_owned())
        .with_context_string("exit_code", exit);
    if !stderr.trim().is_empty() {
        forge = forge.with_context_string("stderr", truncate_context(stderr.trim()));
    }
    if !stdout.trim().is_empty() {
        forge = forge.with_context_string("stdout", truncate_context(stdout.trim()));
    }
    if let Some(serial) = serial {
        forge = forge.device(serial.to_string());
    }
    match code {
        ErrorCode::DeviceOffline => forge.with_suggestion(
            "Reconnect the device or wait for the emulator to finish booting.",
        ),
        ErrorCode::DeviceUnauthorized => {
            forge.with_suggestion("Approve the USB debugging prompt on the device.")
        }
        ErrorCode::DeviceNotFound => {
            forge.with_suggestion("Run `androidforge devices list` to see available devices.")
        }
        ErrorCode::DeviceAmbiguous => {
            forge.with_suggestion("Pass --serial or --session to choose a device explicitly.")
        }
        ErrorCode::DeviceDisconnected => forge.with_suggestion(
            "Inspect the emulator process and run `androidforge session gc` if the session is stale.",
        ),
        ErrorCode::PermissionDenied => {
            forge.with_suggestion("Check the package, path and device permissions.")
        }
        _ => forge,
    }
}

fn truncate_context(value: &str) -> String {
    const MAX: usize = 4000;
    if value.chars().count() <= MAX {
        return value.to_string();
    }
    let truncated: String = value.chars().take(MAX).collect();
    format!("{truncated}\n[truncated]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_screen_and_keyguard_state() {
        assert_eq!(
            parse_screen_on("mWakefulness=Awake\nDisplay Power: state=ON\n"),
            Some(true)
        );
        assert_eq!(parse_screen_on("mWakefulness=Asleep\n"), Some(false));
        assert_eq!(parse_screen_on("nothing"), None);
        assert_eq!(
            parse_locked("mShowingLockscreen=true\nisStatusBarKeyguard=true\n"),
            Some(true)
        );
        assert_eq!(
            parse_locked("mShowingLockscreen=false\nisStatusBarKeyguard=false\n"),
            Some(false)
        );
        assert_eq!(parse_locked("nothing"), None);
    }

    #[test]
    fn parses_keyboard_state() {
        assert_eq!(parse_keyboard_shown("mInputShown=true\n"), Some(true));
        assert_eq!(parse_keyboard_shown("mInputShown=false\n"), Some(false));
        assert_eq!(parse_keyboard_shown("no input method data"), None);
    }

    #[test]
    fn parses_foreground_component() {
        let app = parse_foreground(
            "mCurrentFocus=Window{123 u0 com.example.app/com.example.app.MainActivity}\n",
        )
        .unwrap();
        assert_eq!(app.package, "com.example.app");
        assert_eq!(
            app.activity.as_deref(),
            Some("com.example.app.MainActivity")
        );
    }

    #[test]
    fn wireless_endpoint_validation_rejects_unsafe_or_malformed_hosts() {
        assert_eq!(
            AdbClient::validate_endpoint("192.168.1.20:37521").unwrap(),
            "192.168.1.20:37521"
        );
        assert_eq!(
            AdbClient::validate_endpoint("[::1]:5555").unwrap(),
            "[::1]:5555"
        );
        for endpoint in [
            "192.168.1.20",
            "0.0.0.0:5555",
            "*:5555",
            "host:0",
            "host:notaport",
            "host:70000",
            "bad host:5555",
        ] {
            assert!(
                AdbClient::validate_endpoint(endpoint).is_err(),
                "{endpoint} should be rejected"
            );
        }
    }

    #[test]
    fn wireless_success_parsing_and_pairing_code_redaction() {
        let output = CommandOutput {
            program: "adb".to_string(),
            args: vec!["pair".to_string()],
            exit_code: Some(0),
            success: true,
            stdout: b"Successfully paired to 192.168.1.20:37123 [guid=adb-123]\n".to_vec(),
            stderr: Vec::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 10,
            timed_out: false,
        };
        assert!(AdbClient::pairing_succeeded(&output));
        let redacted = AdbClient::redact_secret("Failed with pairing code 123456", "123456");
        assert!(!redacted.contains("123456"));
        assert!(redacted.contains("[REDACTED]"));

        let rejected = CommandOutput {
            program: "adb".to_string(),
            args: vec!["pair".to_string()],
            exit_code: Some(1),
            success: false,
            stdout: Vec::new(),
            stderr: b"Failed to authenticate\n".to_vec(),
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 10,
            timed_out: false,
        };
        assert!(!AdbClient::pairing_succeeded(&rejected));

        let connected = CommandOutput {
            program: "adb".to_string(),
            args: vec!["connect".to_string()],
            exit_code: Some(0),
            success: true,
            stdout: b"already connected to 192.168.1.20:37521\n".to_vec(),
            stderr: Vec::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 10,
            timed_out: false,
        };
        assert!(AdbClient::connect_succeeded(&connected));
    }
}
