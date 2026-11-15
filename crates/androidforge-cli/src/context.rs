use std::time::Duration;

use androidforge_adb::AdbClient;
use androidforge_app::{ApkInspector, AppManager};
use androidforge_core::config::ForgeConfig;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::{Device, DeviceState};
use androidforge_core::paths::ForgePaths;
use androidforge_core::redact::Redactor;
use androidforge_core::session::{
    Session, SessionState, check_serial_ownership, owner_of_serial, resolve_session_id,
};
use androidforge_core::toolchain::Toolchain;
use androidforge_core::util::parse_duration;
use androidforge_emulator::EmulatorManager;
use androidforge_ui::UiManager;

use crate::cli::Cli;

#[derive(Debug, Clone)]
pub struct CliContext {
    pub paths: ForgePaths,
    pub config: ForgeConfig,
    pub toolchain: Toolchain,
    pub trace: bool,
    pub json: bool,
    pub quiet: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub no_failure_capture: bool,
    pub session_arg: Option<String>,
    pub serial_arg: Option<String>,
    pub run_arg: Option<String>,
}

impl CliContext {
    pub fn new(cli: &Cli) -> Result<Self> {
        let mut paths = ForgePaths::discover(None, cli.project.clone())?;
        let mut config = ForgeConfig::load(&paths)?;
        paths = paths.with_artifact_dir(&config.artifact_dir);
        config.paths = paths.clone();
        let toolchain = Toolchain::discover(&config);
        Ok(Self {
            paths,
            config,
            toolchain,
            trace: cli.trace,
            json: cli.json,
            quiet: cli.quiet,
            dry_run: cli.dry_run,
            yes: cli.yes,
            no_failure_capture: cli.no_failure_capture,
            session_arg: cli.session.clone(),
            serial_arg: cli.serial.clone(),
            run_arg: cli.run.clone(),
        })
    }

    pub fn adb(&self) -> Result<AdbClient> {
        let path = self.toolchain.require_adb()?;
        Ok(AdbClient::new(
            path,
            self.trace,
            Duration::from_secs(self.config.file.timeouts.adb_secs.max(1)),
        ))
    }

    pub fn inspector(&self) -> ApkInspector {
        ApkInspector::new(
            self.toolchain.aapt2.clone(),
            self.toolchain.apksigner.clone(),
            self.toolchain.java.clone(),
            self.trace,
        )
    }

    pub fn app_manager(&self) -> Result<AppManager> {
        let adb = self.adb()?;
        Ok(AppManager::new(
            adb,
            self.inspector(),
            Duration::from_secs(self.config.file.timeouts.install_secs.max(1)),
            Duration::from_secs(self.config.file.timeouts.app_launch_secs.max(1)),
        ))
    }

    pub fn ui_manager(&self) -> Result<UiManager> {
        let adb = self.adb()?;
        Ok(UiManager::new(
            adb,
            Duration::from_secs(self.config.file.ui.default_timeout_secs.max(1)),
            Duration::from_millis(self.config.file.ui.poll_interval_ms.max(50)),
            Duration::from_millis(self.config.file.ui.settle_interval_ms.max(50)),
            self.config.file.ui.tap_clickable_ancestor,
            self.config.file.ui.text_verify,
        ))
    }

    pub fn emulator_manager(&self) -> Result<EmulatorManager> {
        Ok(EmulatorManager::new(
            self.toolchain.clone(),
            self.adb()?,
            self.trace,
            Duration::from_secs(self.config.file.timeouts.boot_secs.max(1)),
            Duration::from_secs(self.config.file.timeouts.emulator_stop_secs.max(1)),
        ))
    }

    pub fn redactor(&self) -> Redactor {
        Redactor::new(
            self.config.file.redaction.enabled && self.config.file.diagnostics.redact,
            &[],
            &self.config.file.redaction.patterns,
        )
    }

    pub fn session_id(&self) -> Result<Option<String>> {
        resolve_session_id(
            &self.paths,
            self.session_arg.as_deref(),
            std::env::var("ANDROIDFORGE_SESSION").ok().as_deref(),
        )
    }

    pub fn load_session(&self, touch: bool) -> Result<Option<Session>> {
        let Some(id) = self.session_id()? else {
            return Ok(None);
        };
        let mut session = Session::load(&self.paths, &id)?;
        if matches!(
            session.state,
            SessionState::Released | SessionState::Failed | SessionState::Abandoned
        ) {
            return Err(ForgeError::new(
                ErrorCode::SessionExpired,
                format!("session {id} is {:?} and cannot be used", session.state),
            )
            .with_suggestion("Start a new session with `androidforge session start --avd <AVD>`.")
            .with_suggestion("Run `androidforge session list` to inspect session state.")
            .session(id));
        }
        if touch {
            session.touch(&self.paths)?;
        }
        Ok(Some(session))
    }

    pub fn require_session(&self) -> Result<Session> {
        self.load_session(true)?.ok_or_else(|| {
            ForgeError::new(ErrorCode::SessionNotFound, "no session was specified")
                .with_suggestion("Pass --session <ID>, set ANDROIDFORGE_SESSION, or run `androidforge session start`.")
                .with_suggestion("Run `androidforge session list` to see available sessions.")
        })
    }

    pub async fn serial(&self) -> Result<String> {
        if let Some(session) = self.load_session(true)? {
            return Ok(session.serial()?.to_string());
        }
        self.resolve_serial(None).await
    }

    pub async fn resolve_serial(&self, session: Option<&Session>) -> Result<String> {
        if let Some(session) = session {
            if let Some(cli_serial) = &self.serial_arg {
                let session_serial = session.serial()?;
                if cli_serial != session_serial {
                    return Err(ForgeError::new(
                        ErrorCode::InvalidArgument,
                        format!(
                            "session {} belongs to {session_serial}, but --serial {cli_serial} was requested",
                            session.id
                        ),
                    )
                    .with_suggestion("Use the session serial or omit --serial.")
                    .session(session.id.clone())
                    .device(cli_serial.clone()));
                }
            }
            return Ok(session.serial()?.to_string());
        }
        if let Some(serial) = &self.serial_arg {
            return Ok(serial.clone());
        }
        if let Ok(serial) = std::env::var("ANDROIDFORGE_SERIAL") {
            if !serial.trim().is_empty() {
                return Ok(serial.trim().to_string());
            }
        }
        if let Some(serial) = &self.config.file.project.default_device {
            return Ok(serial.clone());
        }
        let adb = self.adb()?;
        let devices = adb.devices_raw().await?;
        let operational = devices
            .iter()
            .filter(|device| device.state == DeviceState::Device)
            .collect::<Vec<_>>();
        match operational.len() {
            1 => Ok(operational[0].serial.clone()),
            0 => {
                if devices.is_empty() {
                    Err(ForgeError::new(
                        ErrorCode::DeviceNotFound,
                        "no Android devices or emulators are connected",
                    )
                    .with_suggestion(
                        "Start an emulator with `androidforge session start --avd <AVD>`.",
                    )
                    .with_suggestion("Connect a physical device and approve USB debugging.")
                    .with_suggestion(
                        "Run `androidforge session list` and `androidforge devices list`.",
                    ))
                } else {
                    let state = devices[0].state.as_str();
                    Err(ForgeError::new(
                        if devices[0].state == DeviceState::Unauthorized {
                            ErrorCode::DeviceUnauthorized
                        } else {
                            ErrorCode::DeviceOffline
                        },
                        format!("the only connected device {} is {state}", devices[0].serial),
                    )
                    .with_suggestion("Approve USB debugging or reconnect the device.")
                    .device(devices[0].serial.clone()))
                }
            }
            _ => Err(ForgeError::new(
                ErrorCode::DeviceAmbiguous,
                "multiple devices are connected; choose one explicitly",
            )
            .with_context(
                "devices",
                serde_json::to_value(
                    operational
                        .iter()
                        .map(|device| {
                            serde_json::json!({
                                "serial": device.serial,
                                "model": device.model,
                                "state": device.state.as_str(),
                            })
                        })
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_default(),
            )
            .with_suggestion("Pass --serial <SERIAL> or --session <ID>.")
            .with_suggestion("Run `androidforge devices list` to inspect connected devices.")),
        }
    }

    pub async fn resolve_device(&self, session: Option<&Session>) -> Result<Device> {
        let adb = self.adb()?;
        let serial = self.resolve_serial(session).await?;
        if let Some(session) = session {
            check_serial_ownership(&self.paths, &serial, Some(&session.id))?;
        } else {
            check_serial_ownership(&self.paths, &serial, None)?;
        }
        let device = adb.inspect(&serial).await?;
        if !device.state.is_operational() {
            let code = match device.state {
                DeviceState::Unauthorized => ErrorCode::DeviceUnauthorized,
                DeviceState::Offline => ErrorCode::DeviceOffline,
                _ => ErrorCode::DeviceNotFound,
            };
            return Err(ForgeError::new(
                code,
                format!("device {serial} is {}", device.state.as_str()),
            )
            .with_suggestion("Approve USB debugging or reconnect the device.")
            .with_suggestion("Run `androidforge devices list`.")
            .device(serial));
        }
        Ok(device)
    }

    pub fn owner_for_serial(&self, serial: &str) -> Result<Option<Session>> {
        owner_of_serial(&self.paths, serial)
    }

    pub fn parse_timeout(&self, value: Option<&str>, fallback_secs: u64) -> Result<Duration> {
        match value {
            Some(value) => parse_duration(value).map_err(ForgeError::invalid_argument),
            None => Ok(Duration::from_secs(fallback_secs.max(1))),
        }
    }

    pub fn package_or_config(&self, package: Option<&str>) -> Result<String> {
        package
            .map(str::to_string)
            .or_else(|| self.config.debug_package().map(str::to_string))
            .ok_or_else(|| {
                ForgeError::new(ErrorCode::InvalidArgument, "a package name is required")
                    .with_suggestion(
                        "Pass --package or set [project].package in .androidforge/project.toml.",
                    )
            })
    }
}
