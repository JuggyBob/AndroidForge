use std::collections::BTreeMap;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::{BuildIdentity, Device};
use androidforge_core::output::CommandSuccess;
use androidforge_core::run::{
    RunContext, RunOptions, RunReport, RunStatus, clear_active_run, get_active_run,
};
use androidforge_core::session::Session;
use androidforge_diagnostics::capture_failure_bundle;

use crate::context::CliContext;

pub struct RunScope {
    pub run: RunContext,
    pub implicit: bool,
    pub step: Option<usize>,
}

impl RunScope {
    #[allow(clippy::too_many_arguments)]
    pub async fn begin(
        ctx: &CliContext,
        command: &str,
        name: Option<&str>,
        session: Option<&Session>,
        device: Option<&Device>,
        add_step: bool,
    ) -> Result<Self> {
        let explicit = ctx
            .run_arg
            .clone()
            .or_else(|| std::env::var("ANDROIDFORGE_RUN_ID").ok())
            .filter(|value| !value.trim().is_empty());
        let mut active = if explicit.is_none() {
            get_active_run(&ctx.paths, session.map(|session| session.id.as_str()))?
        } else {
            None
        };
        if let Some(active_run) = &active {
            let report_path = ctx.paths.run_dir(&active_run.run_id).join("report.json");
            if let Ok(report) = androidforge_core::util::read_json::<RunReport>(&report_path)
                && report.status != RunStatus::Running
            {
                clear_active_run(&ctx.paths, session.map(|session| session.id.as_str()))?;
                active = None;
            }
        }
        let run_id = explicit
            .clone()
            .or_else(|| active.as_ref().map(|active| active.run_id.clone()));
        let (mut run, implicit) = if let Some(run_id) = run_id {
            let run = RunContext::open(&ctx.paths.artifacts_dir, &run_id, ctx.redactor())?;
            if run.report.status != RunStatus::Running {
                return Err(ForgeError::new(
                    ErrorCode::InvalidArgument,
                    format!(
                        "run {run_id} is already {:?}; start a new run",
                        run.report.status
                    ),
                )
                .with_suggestion("Use `androidforge run start --name <name>` for a new run.")
                .with_suggestion(
                    "Use `androidforge report show <run>` to inspect the finished run.",
                )
                .with_context_string("run_id", run_id));
            }
            (run, false)
        } else {
            let build = build_identity_for_run(ctx, session).await;
            let mut options = RunOptions::new(
                ctx.paths.artifacts_dir.clone(),
                ctx.paths.project_root.clone(),
                ctx.config.project_name(),
                command,
            );
            options.name = Some(name.unwrap_or(command).to_string());
            options.project_config = ctx.paths.project_config_path.clone();
            options.session_id = session.map(|session| session.id.clone());
            options.device = device.cloned();
            options.build = build;
            options.tool_versions = tool_versions(ctx);
            let run = RunContext::create(options, ctx.redactor())?;
            (run, true)
        };
        if let Some(session) = session {
            run.report.session_id = Some(session.id.clone());
            run.metadata.session_id = Some(session.id.clone());
        }
        if let Some(device) = device {
            run.set_device(device.compact());
        }
        let step = if add_step {
            Some(run.start_step(name.unwrap_or(command), command)?)
        } else {
            None
        };
        run.save()?;
        Ok(Self {
            run,
            implicit,
            step,
        })
    }

    pub fn run_id(&self) -> String {
        self.run.run_id.clone()
    }

    pub fn finish_success(
        &mut self,
        details: serde_json::Value,
        artifacts: Vec<androidforge_core::model::ArtifactRef>,
    ) -> Result<String> {
        if let Some(step) = self.step.take() {
            self.run.finish_step_success(step, details, artifacts)?;
        } else {
            for artifact in artifacts {
                self.run.add_artifact(artifact)?;
            }
        }
        if self.implicit {
            self.run.finalize_pass()?;
        } else {
            self.run.save()?;
        }
        Ok(self.run.run_id.clone())
    }

    pub fn finish_failure(
        &mut self,
        error: &ForgeError,
        artifacts: Vec<androidforge_core::model::ArtifactRef>,
    ) -> Result<()> {
        if let Some(step) = self.step.take() {
            self.run.finish_step_failure(step, error, artifacts)?;
        } else {
            for artifact in artifacts {
                self.run.add_artifact(artifact)?;
            }
        }
        if self.implicit {
            self.run.finalize_failure(error)?;
        } else {
            self.run.save()?;
        }
        Ok(())
    }

    /// Finish the current step but deliberately leave the run open. Used by
    /// start/stop command pairs such as screen recordings and logcat capture.
    pub fn finish_success_held(
        &mut self,
        details: serde_json::Value,
        artifacts: Vec<androidforge_core::model::ArtifactRef>,
    ) -> Result<String> {
        if let Some(step) = self.step.take() {
            self.run.finish_step_success(step, details, artifacts)?;
        } else {
            for artifact in artifacts {
                self.run.add_artifact(artifact)?;
            }
        }
        self.run.save()?;
        Ok(self.run.run_id.clone())
    }
}

/// Execute a block inside a run. This is a macro rather than a generic
/// higher-ranked async function so action blocks can freely borrow their
/// command arguments and the surrounding CLI context without fighting the
/// borrow checker or introducing a hidden `'static` requirement.
#[macro_export]
macro_rules! execute_in_run {
    ($ctx:expr, $command:expr, $name:expr, $session:expr, $device:expr, $add_step:expr, |$run:ident| $body:expr $(,)?) => {{
        async {
            let mut scope = $crate::run_scope::RunScope::begin(
                $ctx, $command, $name, $session, $device, $add_step,
            )
            .await?;
            let run_id = scope.run_id();
            let outcome: androidforge_core::error::Result<
                androidforge_core::output::CommandSuccess,
            > = {
                let $run = &mut scope.run;
                ($body).await
            };
            $crate::run_scope::finish_scope(
                $ctx, &mut scope, outcome, run_id, $session, $device, $command,
            )
            .await
        }
    }};
}

pub async fn finish_scope(
    ctx: &CliContext,
    scope: &mut RunScope,
    outcome: Result<CommandSuccess>,
    run_id: String,
    session: Option<&Session>,
    device: Option<&Device>,
    command: &str,
) -> Result<CommandSuccess> {
    match outcome {
        Ok(mut success) => {
            let artifacts = success.artifacts.clone();
            match scope.finish_success(success.result.clone(), artifacts) {
                Ok(actual_run_id) => {
                    success.run_id = Some(actual_run_id);
                    success.session_id = success
                        .session_id
                        .or_else(|| session.map(|session| session.id.clone()));
                    success.device_serial = success
                        .device_serial
                        .or_else(|| device.map(|device| device.serial.clone()));
                    Ok(success)
                }
                Err(error) => Err(error.with_context_string("run_id", run_id)),
            }
        }
        Err(mut error) => {
            let mut evidence = Vec::new();
            if should_capture_failure(ctx, &error)
                && let Some(serial) = device.map(|device| device.serial.as_str())
                && let Ok(app) = ctx.app_manager()
                && let Ok(ui) = ctx.ui_manager()
            {
                let package = ctx.config.package().map(str::to_string);
                let bundle = capture_failure_bundle(
                    &app.adb,
                    Some(&ui),
                    Some(&app),
                    &mut scope.run,
                    serial,
                    package.as_deref(),
                    &format!("{command} failed: {}", error.message()),
                    ctx.config.file.diagnostics.include_dumps,
                )
                .await;
                evidence.extend(bundle.artifacts.clone());
                error = error.with_artifacts(bundle.artifacts);
            }
            let finish = scope.finish_failure(&error, evidence);
            if let Err(finish_error) = finish {
                error = error.with_context_string("run_finalize_error", finish_error.message());
            }
            error = error.with_context_string("run_id", run_id);
            Err(error)
        }
    }
}

fn should_capture_failure(ctx: &CliContext, error: &ForgeError) -> bool {
    if ctx.no_failure_capture || ctx.dry_run || !ctx.config.file.diagnostics.capture_on_failure {
        return false;
    }
    // Package/certificate checks happen before the device is touched. A full
    // screenshot/logcat/dumpsys bundle for those adds megabytes of noise and
    // cannot explain the local validation result.
    if error
        .body()
        .context
        .get("preflight")
        .and_then(|value| value.as_bool())
        == Some(true)
    {
        return false;
    }
    !matches!(
        error.code(),
        ErrorCode::AdbNotFound
            | ErrorCode::SdkNotFound
            | ErrorCode::DeviceNotFound
            | ErrorCode::DeviceAmbiguous
            | ErrorCode::DeviceOffline
            | ErrorCode::DeviceUnauthorized
            | ErrorCode::SessionNotFound
            | ErrorCode::SessionExpired
            | ErrorCode::InvalidArgument
            | ErrorCode::InvalidCommand
            | ErrorCode::ConfigInvalid
            | ErrorCode::ApkInvalid
    )
}

fn config_build_identity(ctx: &CliContext) -> BuildIdentity {
    // This is only a declared project target until an install observes the real
    // APK. Never copy an expected certificate into an observed identity field:
    // debug and release packages legitimately have different certificates.
    BuildIdentity {
        package: ctx.config.debug_package().map(str::to_string),
        ..BuildIdentity::default()
    }
}

/// Resolve the build identity a run should carry:
///
/// 1. the exact identity captured by an install in this session;
/// 2. otherwise the installed package/version, read once and cached in the
///    session, so later runs do not repeat the query;
/// 3. otherwise the adapter's declared debug/release package.
pub async fn build_identity_for_run(ctx: &CliContext, session: Option<&Session>) -> BuildIdentity {
    if let Some(build) = session.and_then(|session| session.build.clone()) {
        let configured = [ctx.config.package(), ctx.config.debug_package()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if configured.is_empty()
            || build
                .package
                .as_deref()
                .is_some_and(|package| configured.contains(&package))
        {
            return build;
        }
    }
    let mut build = config_build_identity(ctx);
    let Some(session) = session else {
        return build;
    };
    let Some(serial) = session.serial.as_deref() else {
        return build;
    };
    let package = build.package.clone();
    let Some(package) = package else {
        return build;
    };
    let Ok(app) = ctx.app_manager() else {
        return build;
    };
    let Ok(info) = app.info(serial, &package).await else {
        return build;
    };
    if let Some(installed) = &info.installed {
        build.package = Some(package.clone());
        build.version_name = installed.version_name.clone();
        build.version_code = installed.version_code;
        build.installed_apk_path = installed.base_apk_path.clone();
        build.verified = false;
        build.notes.push(
            "package/version read from the installed app; APK hash and certificate are filled by an AndroidForge install"
                .to_string(),
        );
        let _ = androidforge_core::session::record_build(&ctx.paths, &session.id, build.clone());
    }
    build
}

pub fn tool_versions(ctx: &CliContext) -> BTreeMap<String, String> {
    let mut versions = BTreeMap::new();
    if let Some(path) = &ctx.toolchain.adb {
        versions.insert("adb".to_string(), path.to_string_lossy().into_owned());
    }
    if let Some(path) = &ctx.toolchain.emulator {
        versions.insert("emulator".to_string(), path.to_string_lossy().into_owned());
    }
    if let Some(path) = &ctx.toolchain.aapt2 {
        versions.insert("aapt2".to_string(), path.to_string_lossy().into_owned());
    }
    if let Some(path) = &ctx.toolchain.apksigner {
        versions.insert("apksigner".to_string(), path.to_string_lossy().into_owned());
    }
    if let Some(path) = &ctx.toolchain.java {
        versions.insert("java".to_string(), path.to_string_lossy().into_owned());
    }
    if let Some(path) = &ctx.toolchain.maestro {
        versions.insert("maestro".to_string(), path.to_string_lossy().into_owned());
    }
    versions
}
