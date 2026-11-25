use std::time::Duration;

use androidforge_adb::AdbClient;
use androidforge_app::{ApkInspector, AppManager};
use androidforge_core::config::ForgeConfig;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::{Device, DeviceKind, DeviceState};
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

/// Independent opt-ins for each physical-device capability. Granting one risk
/// never grants another; `--allow-physical-mutation` is tracked separately as
/// the superset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScopedPhysicalFlags {
    pub app_control: bool,
    pub data: bool,
    pub network: bool,
    pub install: bool,
    pub input: bool,
}

impl ScopedPhysicalFlags {
    fn from_cli(cli: &Cli) -> Self {
        Self {
            app_control: cli.allow_physical_app_control,
            data: cli.allow_physical_data,
            network: cli.allow_physical_network,
            install: cli.allow_physical_install,
            input: cli.allow_physical_input,
        }
    }

    pub fn allows(self, risk: Risk) -> bool {
        match risk {
            Risk::AppControl => self.app_control,
            Risk::Data => self.data,
            Risk::Network => self.network,
            Risk::Install => self.install,
            Risk::Input => self.input,
        }
    }
}

/// Capability category used to gate operations on physical devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    AppControl,
    Data,
    Network,
    Install,
    Input,
}

impl Risk {
    /// The narrowest CLI flag that grants this risk.
    pub fn flag(self) -> &'static str {
        match self {
            Self::AppControl => "--allow-physical-app-control",
            Self::Data => "--allow-physical-data",
            Self::Network => "--allow-physical-network",
            Self::Install => "--allow-physical-install",
            Self::Input => "--allow-physical-input",
        }
    }

    /// Classify the existing human-readable operation descriptions so callers
    /// that still use `ensure_physical_mutation_allowed` get scoped gating.
    /// Unmapped operations stay on the master flag only: no default loosening.
    pub fn for_operation(operation: &str) -> Option<Self> {
        match operation {
            "launching an app or intent"
            | "force-stopping an app"
            | "broadcasting an intent"
            | "starting or stopping a service"
            | "killing an app process"
            | "triggering a JobScheduler job"
            | "cancelling a JobScheduler job" => Some(Self::AppControl),
            "pushing files into an app"
            | "importing app data"
            | "resetting app data"
            | "seeding app data"
            | "clearing app data"
            | "clearing notifications"
            | "running an agent pipeline"
            | "running a conformance flow"
            | "running a Maestro flow"
            | "running an instrumentation test" => Some(Self::Data),
            "changing airplane mode"
            | "changing Wi-Fi state"
            | "changing mobile data"
            | "changing the device proxy"
            | "applying a network profile"
            | "restoring network state" => Some(Self::Network),
            "installing an app" | "uninstalling an app" => Some(Self::Install),
            "dismissing system overlays" | "tapping a notification" => Some(Self::Input),
            _ => None,
        }
    }
}

fn physical_consent_allows(master: bool, scoped: ScopedPhysicalFlags, risk: Risk) -> bool {
    master || scoped.allows(risk)
}

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
    pub allow_physical_mutation: bool,
    pub scoped_physical: ScopedPhysicalFlags,
    pub no_failure_capture: bool,
    pub session_arg: Option<String>,
    pub serial_arg: Option<String>,
    pub run_arg: Option<String>,
}

/// Compare project roots the way Windows paths are used in commands. Sessions
/// are project-bound; cross-project reuse is refused by `load_session`.
pub(crate) fn same_project_root(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        let normalized = value.replace('\\', "/");
        let trimmed = normalized.trim_end_matches('/');
        if cfg!(windows) {
            trimmed.to_ascii_lowercase()
        } else {
            trimmed.to_string()
        }
    };
    normalize(left) == normalize(right)
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
            allow_physical_mutation: cli.allow_physical_mutation,
            scoped_physical: ScopedPhysicalFlags::from_cli(cli),
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
        let requested_project = self.paths.project_root.to_string_lossy().into_owned();
        if let Some(session_project) = &session.project_root
            && !same_project_root(session_project, &requested_project)
        {
            return Err(ForgeError::new(
                ErrorCode::SessionProjectMismatch,
                format!(
                    "session {id} is bound to project {session_project}, not {requested_project}"
                ),
            )
            .with_context_string("session_project", session_project.clone())
            .with_context_string("requested_project", requested_project)
            .with_suggestion(
                "Use a session acquired for this project, or pass --project with the project that owns the session.",
            )
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
        if let Ok(serial) = std::env::var("ANDROIDFORGE_SERIAL")
            && !serial.trim().is_empty()
        {
            return Ok(serial.trim().to_string());
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

    /// True when this risk is permitted, either by its narrow scoped flag or
    /// by the `--allow-physical-mutation` superset.
    pub fn risk_allowed(&self, risk: Risk) -> bool {
        physical_consent_allows(self.allow_physical_mutation, self.scoped_physical, risk)
    }

    /// Refuse a physical-device operation unless its own capability flag or
    /// the `--allow-physical-mutation` superset was passed. Emulators are
    /// always allowed. All decisions funnel through `check_physical_mutation`.
    pub fn ensure_risk_allowed(&self, device: &Device, risk: Risk, operation: &str) -> Result<()> {
        check_physical_risk(
            device,
            self.allow_physical_mutation,
            self.scoped_physical.allows(risk),
            risk,
            operation,
        )
    }

    /// Legacy entry point for callers that describe the operation but do not
    /// pass a risk explicitly. Known operations inherit their scoped risk;
    /// anything unmapped keeps the old master-flag-only behavior.
    pub fn ensure_physical_mutation_allowed(&self, device: &Device, operation: &str) -> Result<()> {
        match Risk::for_operation(operation) {
            Some(risk) => self.ensure_risk_allowed(device, risk, operation),
            None => check_physical_mutation(device, self.allow_physical_mutation, operation),
        }
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
        let package = package
            .map(str::to_string)
            .or_else(|| self.config.debug_package().map(str::to_string))
            .ok_or_else(|| {
                ForgeError::new(ErrorCode::InvalidArgument, "a package name is required")
                    .with_suggestion(
                        "Pass --package or set [project].package in .androidforge/project.toml.",
                    )
            })?;
        if !androidforge_core::util::is_valid_package_name(&package) {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                format!("'{package}' is not a valid Android package name"),
            )
            .with_suggestion(
                "Use a name like com.example.app; separators and '..' are not allowed.",
            ));
        }
        Ok(package)
    }
}

fn check_physical_risk(
    device: &Device,
    master_allowed: bool,
    scoped_allowed: bool,
    risk: Risk,
    operation: &str,
) -> Result<()> {
    check_physical_mutation(device, master_allowed || scoped_allowed, operation).map_err(|error| {
        error.with_suggestion(format!(
            "This operation is gated by {}; use --allow-physical-mutation only when every physical-device risk is intended.",
            risk.flag()
        ))
    })
}

fn check_physical_mutation(device: &Device, allowed: bool, operation: &str) -> Result<()> {
    if device.kind == DeviceKind::Emulator || allowed {
        return Ok(());
    }
    Err(ForgeError::new(
        ErrorCode::PhysicalDeviceMutationBlocked,
        format!(
            "{operation} would change physical device {} ({}); pass --allow-physical-mutation to confirm",
            device.serial,
            device.model.as_deref().unwrap_or("unknown model")
        ),
    )
    .with_suggestion(
        "Physical devices are protected by default because their data is not disposable.",
    )
    .with_suggestion(
        "Use a disposable emulator for destructive tests, or rerun with --allow-physical-mutation.",
    )
    .device(device.serial.clone()))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{
        Risk, ScopedPhysicalFlags, check_physical_mutation, check_physical_risk,
        physical_consent_allows, same_project_root,
    };
    use crate::cli::Cli;
    use androidforge_core::error::{ErrorCode, ForgeError};
    use androidforge_core::model::{Device, DeviceKind, DeviceState};

    fn physical_device() -> Device {
        let mut device = Device::basic("R58M123456A", DeviceState::Device);
        device.kind = DeviceKind::Physical;
        device.model = Some("POCO X4 GT".to_string());
        device
    }

    fn error_mentions(error: &ForgeError, text: &str) -> bool {
        error.message().contains(text)
            || error
                .body()
                .suggestions
                .iter()
                .any(|suggestion| suggestion.contains(text))
    }

    #[test]
    fn physical_mutations_require_an_explicit_flag() {
        let device = physical_device();

        let error = check_physical_mutation(&device, false, "uninstalling an app").unwrap_err();
        assert_eq!(error.code(), ErrorCode::PhysicalDeviceMutationBlocked);
        assert!(error.message().contains("--allow-physical-mutation"));
        assert!(check_physical_mutation(&device, true, "uninstalling an app").is_ok());

        let mut emulator = device;
        emulator.kind = DeviceKind::Emulator;
        assert!(check_physical_mutation(&emulator, false, "uninstalling an app").is_ok());
    }

    #[test]
    fn scoped_flag_allows_only_its_own_risk() {
        let device = physical_device();
        let flags = ScopedPhysicalFlags {
            input: true,
            ..ScopedPhysicalFlags::default()
        };

        assert!(physical_consent_allows(false, flags, Risk::Input));
        assert!(!physical_consent_allows(false, flags, Risk::AppControl));
        assert!(!physical_consent_allows(false, flags, Risk::Data));
        assert!(!physical_consent_allows(false, flags, Risk::Network));
        assert!(!physical_consent_allows(false, flags, Risk::Install));

        assert!(
            check_physical_risk(
                &device,
                false,
                flags.allows(Risk::Input),
                Risk::Input,
                "injecting input"
            )
            .is_ok()
        );
        let error = check_physical_risk(
            &device,
            false,
            flags.allows(Risk::Data),
            Risk::Data,
            "clearing app data",
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::PhysicalDeviceMutationBlocked);
        assert!(error_mentions(&error, "--allow-physical-data"));
    }

    #[test]
    fn master_flag_allows_every_risk_and_unrelated_scoped_flags_still_block() {
        let device = physical_device();
        for risk in [
            Risk::AppControl,
            Risk::Data,
            Risk::Network,
            Risk::Install,
            Risk::Input,
        ] {
            assert!(physical_consent_allows(
                true,
                ScopedPhysicalFlags::default(),
                risk
            ));
            assert!(
                check_physical_risk(&device, true, false, risk, "physical operation").is_ok(),
                "master flag must allow {risk:?}"
            );
        }

        let data_only = ScopedPhysicalFlags {
            data: true,
            ..ScopedPhysicalFlags::default()
        };
        let error = check_physical_risk(
            &device,
            false,
            data_only.allows(Risk::AppControl),
            Risk::AppControl,
            "force-stopping an app",
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::PhysicalDeviceMutationBlocked);
        assert!(error_mentions(&error, "--allow-physical-app-control"));
    }

    #[test]
    fn cli_scoped_flags_map_to_independent_risks() {
        let cli = Cli::try_parse_from([
            "androidforge",
            "config",
            "show",
            "--allow-physical-app-control",
            "--allow-physical-data",
            "--allow-physical-network",
            "--allow-physical-install",
            "--allow-physical-input",
        ])
        .expect("parse scoped flags");
        let flags = ScopedPhysicalFlags::from_cli(&cli);
        assert!(flags.allows(Risk::AppControl));
        assert!(flags.allows(Risk::Data));
        assert!(flags.allows(Risk::Network));
        assert!(flags.allows(Risk::Install));
        assert!(flags.allows(Risk::Input));

        let partial =
            Cli::try_parse_from(["androidforge", "config", "show", "--allow-physical-input"])
                .expect("parse one scoped flag");
        let flags = ScopedPhysicalFlags::from_cli(&partial);
        assert_eq!(
            flags,
            ScopedPhysicalFlags {
                input: true,
                ..ScopedPhysicalFlags::default()
            }
        );
    }

    #[test]
    fn operation_risk_mapping_covers_all_scoped_groups() {
        assert_eq!(
            Risk::for_operation("launching an app or intent"),
            Some(Risk::AppControl)
        );
        assert_eq!(
            Risk::for_operation("force-stopping an app"),
            Some(Risk::AppControl)
        );
        assert_eq!(
            Risk::for_operation("broadcasting an intent"),
            Some(Risk::AppControl)
        );
        assert_eq!(
            Risk::for_operation("starting or stopping a service"),
            Some(Risk::AppControl)
        );
        assert_eq!(
            Risk::for_operation("killing an app process"),
            Some(Risk::AppControl)
        );
        assert_eq!(
            Risk::for_operation("pushing files into an app"),
            Some(Risk::Data)
        );
        assert_eq!(Risk::for_operation("seeding app data"), Some(Risk::Data));
        assert_eq!(
            Risk::for_operation("installing an app"),
            Some(Risk::Install)
        );
        assert_eq!(
            Risk::for_operation("uninstalling an app"),
            Some(Risk::Install)
        );
        assert_eq!(
            Risk::for_operation("changing airplane mode"),
            Some(Risk::Network)
        );
        assert_eq!(
            Risk::for_operation("changing the device proxy"),
            Some(Risk::Network)
        );
        assert_eq!(
            Risk::for_operation("dismissing system overlays"),
            Some(Risk::Input)
        );
        // Unmapped operations deliberately stay on the master flag only.
        assert_eq!(Risk::for_operation("changing device time"), None);
    }

    #[test]
    fn project_roots_compare_slashes_and_windows_case() {
        assert!(same_project_root(
            "A:\\Projects\\Active\\Example\\Internal",
            "A:/Projects/Active/Example/Internal/"
        ));
        if cfg!(windows) {
            assert!(same_project_root(
                "A:\\Projects\\Active\\Example\\Internal",
                "a:\\projects\\active\\example\\internal"
            ));
        }
        assert!(!same_project_root(
            "A:/Projects/Active/Example/Internal",
            "A:/Projects/Active/Other/Internal"
        ));
    }
}
