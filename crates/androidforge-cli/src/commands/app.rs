use std::path::{Path, PathBuf};
use std::time::Duration;

use androidforge_app::{AppFiles, AppStateExpectation, AppStates, InstallOptions};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::session::Session;

use crate::cli::{
    ApkVerifyArgs, AppCommand, AppInstallArgs, AppLaunchArgs, AppListArgs, AppPackageArgs,
    AppStatusArgs, AppUninstallArgs,
};
use crate::commands::common::success;
use crate::context::CliContext;

pub async fn app(ctx: &CliContext, command: &AppCommand) -> Result<CommandSuccess> {
    match command {
        AppCommand::Install(args) => install(ctx, args).await,
        AppCommand::Uninstall(args) => uninstall(ctx, args).await,
        AppCommand::ClearData(args) => clear_data(ctx, args).await,
        AppCommand::Launch(args) => launch(ctx, args).await,
        AppCommand::ForceStop(args) => force_stop(ctx, args).await,
        AppCommand::Kill(args) => kill(ctx, args).await,
        AppCommand::Status(args) => status(ctx, args).await,
        AppCommand::Info(args) => info(ctx, args).await,
        AppCommand::List(args) => list(ctx, args).await,
        AppCommand::FilePush(args) => {
            let session = ctx.load_session(true)?;
            let device = ctx.resolve_device(session.as_ref()).await?;
            let package = ctx.package_or_config(args.package.as_deref())?;
            let local = local_path(ctx, &args.local);
            if ctx.dry_run {
                return Ok(plan(
                    "app.file-push",
                    &device.serial,
                    session.as_ref(),
                    format!(
                        "Would push {} to {} on {}\n",
                        local.display(),
                        args.remote,
                        package
                    ),
                ));
            }
            execute_in_run!(
                ctx,
                "app.file-push",
                Some("app file push"),
                session.as_ref(),
                Some(&device),
                true,
                |run| async {
                    let files = AppFiles::new(androidforge_adb::AdbClient::new(
                        ctx.toolchain.require_adb()?,
                        ctx.trace,
                        Duration::from_secs(ctx.config.file.timeouts.adb_secs),
                    ));
                    let transfer = files
                        .push(
                            &device.serial,
                            &package,
                            &local,
                            &args.remote,
                            args.external,
                        )
                        .await?;
                    let artifact = run.write_text_artifact(
                        "logs/app-file-push.json",
                        "file_transfer",
                        Some("App file transfer result"),
                        &serde_json::to_string_pretty(&transfer).unwrap_or_default(),
                    )?;
                    Ok(success(
                        "app.file-push",
                        serde_json::to_value(&transfer)?,
                        format!(
                            "Pushed {} to {} ({})\n",
                            local.display(),
                            transfer.path,
                            transfer.size_bytes
                        ),
                    )
                    .with_artifacts(vec![artifact])
                    .with_optional_session(session_id(session.as_ref()))
                    .with_device(device.serial.clone()))
                },
            )
            .await
        }
        AppCommand::FilePull(args) => {
            let session = ctx.load_session(true)?;
            let device = ctx.resolve_device(session.as_ref()).await?;
            let package = ctx.package_or_config(args.package.as_deref())?;
            let local = local_path(ctx, &args.local);
            if ctx.dry_run {
                return Ok(plan(
                    "app.file-pull",
                    &device.serial,
                    session.as_ref(),
                    format!("Would pull {} to {}\n", args.remote, local.display()),
                ));
            }
            execute_in_run!(
                ctx,
                "app.file-pull",
                Some("app file pull"),
                session.as_ref(),
                Some(&device),
                true,
                |run| async {
                    let files = AppFiles::new(ctx.adb()?);
                    let transfer = files
                        .pull(
                            &device.serial,
                            &package,
                            &args.remote,
                            &local,
                            args.external,
                        )
                        .await?;
                    let artifact = run.write_text_artifact(
                        "logs/app-file-pull.json",
                        "file_transfer",
                        Some("App file transfer result"),
                        &serde_json::to_string_pretty(&transfer).unwrap_or_default(),
                    )?;
                    Ok(success(
                        "app.file-pull",
                        serde_json::to_value(&transfer)?,
                        format!(
                            "Pulled {} to {} ({})\n",
                            transfer.path,
                            local.display(),
                            transfer.size_bytes
                        ),
                    )
                    .with_artifacts(vec![artifact])
                    .with_optional_session(session_id(session.as_ref()))
                    .with_device(device.serial.clone()))
                },
            )
            .await
        }
        AppCommand::DataBackup(args) => {
            let session = ctx.load_session(true)?;
            let device = ctx.resolve_device(session.as_ref()).await?;
            if ctx.dry_run {
                return Ok(plan(
                    "app.data-backup",
                    &device.serial,
                    session.as_ref(),
                    format!(
                        "Would back up {} data to {}\n",
                        args.package,
                        args.path.display()
                    ),
                ));
            }
            let package = args.package.clone();
            let path = local_path(ctx, &args.path);
            execute_in_run!(
                ctx,
                "app.data-backup",
                Some("app data backup"),
                session.as_ref(),
                Some(&device),
                true,
                |run| async {
                    let files = AppFiles::new(ctx.adb()?);
                    let transfer = files.backup_data(&device.serial, &package, &path).await?;
                    let artifact = run.write_text_artifact(
                        "logs/app-data-backup.json",
                        "app_data_backup",
                        Some("App data backup metadata"),
                        &serde_json::to_string_pretty(&transfer).unwrap_or_default(),
                    )?;
                    Ok(success(
                        "app.data-backup",
                        serde_json::to_value(&transfer)?,
                        format!(
                            "Backed up {} data to {} ({})\n",
                            package,
                            path.display(),
                            transfer.size_bytes
                        ),
                    )
                    .with_artifacts(vec![artifact])
                    .with_optional_session(session_id(session.as_ref()))
                    .with_device(device.serial.clone()))
                },
            )
            .await
        }
        AppCommand::DataRestore(args) => {
            if !ctx.yes {
                return Err(ForgeError::new(
                    ErrorCode::InvalidArgument,
                    "restoring app data overwrites app-private files; pass --yes",
                )
                .with_suggestion("Back up the current data first where the app is debuggable."));
            }
            let session = ctx.load_session(true)?;
            let device = ctx.resolve_device(session.as_ref()).await?;
            if ctx.dry_run {
                return Ok(plan(
                    "app.data-restore",
                    &device.serial,
                    session.as_ref(),
                    format!(
                        "Would restore {} data from {}\n",
                        args.package,
                        args.path.display()
                    ),
                ));
            }
            let package = args.package.clone();
            let path = local_path(ctx, &args.path);
            execute_in_run!(
                ctx,
                "app.data-restore",
                Some("app data restore"),
                session.as_ref(),
                Some(&device),
                true,
                |run| async {
                    let files = AppFiles::new(ctx.adb()?);
                    let transfer = files.restore_data(&device.serial, &package, &path).await?;
                    let artifact = run.write_text_artifact(
                        "logs/app-data-restore.json",
                        "app_data_restore",
                        Some("App data restore metadata"),
                        &serde_json::to_string_pretty(&transfer).unwrap_or_default(),
                    )?;
                    Ok(success(
                        "app.data-restore",
                        serde_json::to_value(&transfer)?,
                        format!("Restored {} data from {}\n", package, path.display()),
                    )
                    .with_artifacts(vec![artifact])
                    .with_optional_session(session_id(session.as_ref()))
                    .with_device(device.serial.clone()))
                },
            )
            .await
        }
    }
}

pub(crate) async fn install(ctx: &CliContext, args: &AppInstallArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let mut options = InstallOptions::new(local_path(ctx, &args.apk));
    options.fresh = args.fresh;
    options.confirmed = ctx.yes;
    options.allow_downgrade = args.downgrade;
    options.verify_installed = !args.no_verify && ctx.config.file.project.verify_installed_apk;
    options.only_if_changed = args.only_if_changed;
    options.expected_package = args.expected_package.clone();
    options.expected_cert_sha256 = if let Some(certificate) = &args.expected_cert_sha256 {
        vec![certificate.clone()]
    } else {
        ctx.config.file.project.expected_cert_sha256.clone()
    };
    if options.expected_package.is_none() {
        let mut allowed = Vec::new();
        if let Some(package) = ctx.config.package() {
            allowed.push(package.to_string());
        }
        if let Some(package) = ctx.config.debug_package()
            && !allowed.iter().any(|value| value == package)
        {
            allowed.push(package.to_string());
        }
        options.allowed_packages = allowed;
    }
    options.timeout = ctx.parse_timeout(
        args.timeout.as_deref(),
        ctx.config.file.timeouts.install_secs,
    )?;
    if ctx.dry_run {
        let inspector = ctx.inspector();
        let apk = inspector.inspect(&options.apk_path, false).await?;
        let package = apk.package.clone();
        let mut result = serde_json::json!({
            "dry_run": true,
            "serial": device.serial,
            "apk": apk,
            "install": {
                "fresh": options.fresh,
                "allow_downgrade": options.allow_downgrade,
                "verify_installed": options.verify_installed,
                "expected_package": options.expected_package,
                "allowed_packages": options.allowed_packages,
                "expected_cert_sha256": options.expected_cert_sha256,
                "only_if_changed": options.only_if_changed,
            }
        });
        if options.fresh && !ctx.yes {
            result["install"]["requires_confirmation"] = serde_json::json!(true);
        }
        return Ok(success(
            "app.install",
            result,
            format!(
                "Would install {} on {}{}{}\npackage: {}\nversion: {}\ncertificate: {}\n",
                options.apk_path.display(),
                device.serial,
                if options.fresh {
                    " with --fresh (data loss)"
                } else {
                    ""
                },
                if options.allow_downgrade {
                    " allowing downgrade"
                } else {
                    ""
                },
                package.as_deref().unwrap_or("-"),
                apk.version_name.as_deref().unwrap_or("-"),
                apk.signature.cert_sha256.as_deref().unwrap_or("-"),
            ),
        )
        .with_optional_session(session_id(session.as_ref()))
        .with_device(device.serial.clone()));
    }
    let apk_display = options.apk_path.display().to_string();
    let outcome = execute_in_run!(
        ctx,
        "app.install",
        Some("app install"),
        session.as_ref(),
        Some(&device),
        true,
        |run| async {
            let app = ctx.app_manager()?;
            let result = app.install(&device.serial, &options).await.map_err(|error| {
                error.with_context_string(
                    "apk_path",
                    options.apk_path.to_string_lossy().into_owned(),
                )
            })?;
            run.set_build(result.build.clone());
            let identity = serde_json::to_string_pretty(&result.build).unwrap_or_default();
            let artifact = run.write_text_artifact(
                "build/install-identity.json",
                "build_identity",
                Some("Installed package, APK hash and signing certificate identity"),
                &identity,
            )?;
            let human = if result.skipped {
                format!(
                    "Installation skipped: {}\npackage: {}\nversion: {}\nbase APK SHA-256: {}\n",
                    result.reason.as_deref().unwrap_or("the installed build already matches"),
                    result.build.package.as_deref().unwrap_or("-"),
                    result.build.version_name.as_deref().unwrap_or("-"),
                    result.build.apk_sha256.as_deref().unwrap_or("-"),
                )
            } else {
                format!(
                    "Installed {}\npackage: {}\nversion: {}\ninstalled APK SHA-256: {}\nsigning cert SHA-256: {}\ndebug signed: {}\n{}",
                    apk_display,
                    result.build.package.as_deref().unwrap_or("-"),
                    result.build.version_name.as_deref().unwrap_or("-"),
                    result.build.installed_apk_sha256.as_deref().or(result.build.apk_sha256.as_deref()).unwrap_or("-"),
                    result.build.cert_sha256.as_deref().unwrap_or("-"),
                    result.build.debug_signed.map(|value| value.to_string()).unwrap_or_else(|| "-".to_string()),
                    result.reason.as_ref().map(|reason| format!("decision reason: {reason}\n")).unwrap_or_default(),
                )
            };
            Ok(success("app.install", serde_json::to_value(&result)?, human)
                .with_artifacts(vec![artifact])
                .with_optional_session(session_id(session.as_ref()))
                .with_device(device.serial.clone()))
        },
    )
    .await?;
    if outcome
        .result
        .get("skipped")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(outcome);
    }
    if let Some(session) = session.as_ref()
        && let Some(build) = outcome
            .result
            .get("build")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
    {
        androidforge_core::session::record_build(&ctx.paths, &session.id, build)?;
    }
    Ok(outcome)
}

async fn uninstall(ctx: &CliContext, args: &AppUninstallArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    if ctx.dry_run {
        return Ok(plan(
            "app.uninstall",
            &device.serial,
            session.as_ref(),
            format!("Would uninstall {} from {}\n", args.package, device.serial),
        ));
    }
    let package = args.package.clone();
    let keep_data = args.keep_data;
    let outcome = execute_in_run!(
        ctx,
        "app.uninstall",
        Some("app uninstall"),
        session.as_ref(),
        Some(&device),
        true,
        |_run| async {
            let app = ctx.app_manager()?;
            app.uninstall(&device.serial, &package, keep_data).await?;
            Ok(success(
                "app.uninstall",
                serde_json::json!({"package": package, "keep_data": keep_data}),
                format!(
                    "Uninstalled {package}{}\n",
                    if keep_data { " (kept data)" } else { "" }
                ),
            )
            .with_optional_session(session_id(session.as_ref()))
            .with_device(device.serial.clone()))
        },
    )
    .await?;
    if let Some(session) = session.as_ref() {
        androidforge_core::session::clear_build(&ctx.paths, &session.id, &package)?;
    }
    Ok(outcome)
}

async fn clear_data(ctx: &CliContext, args: &AppPackageArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    if ctx.dry_run {
        return Ok(success(
            "app.clear-data",
            serde_json::json!({
                "dry_run": true,
                "serial": device.serial,
                "package": args.package,
                "requires_confirmation": true,
            }),
            format!(
                "Would clear {} data on {}; requires --yes when executed\n",
                args.package, device.serial
            ),
        )
        .with_optional_session(session_id(session.as_ref()))
        .with_device(device.serial.clone()));
    }
    if !ctx.yes {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            "clearing app data is destructive; pass --yes",
        )
        .with_suggestion("Back up data first where the application is debuggable.")
        .with_suggestion("Use --dry-run to preview without confirmation."));
    }
    let package = args.package.clone();
    execute_in_run!(
        ctx,
        "app.clear-data",
        Some("app clear data"),
        session.as_ref(),
        Some(&device),
        true,
        |_run| async {
            let app = ctx.app_manager()?;
            let output = app.clear_data(&device.serial, &package).await?;
            Ok(success(
                "app.clear-data",
                serde_json::json!({"package": package, "output": output}),
                format!("Cleared data for {package}\n"),
            )
            .with_optional_session(session_id(session.as_ref()))
            .with_device(device.serial.clone()))
        },
    )
    .await
}

async fn launch(ctx: &CliContext, args: &AppLaunchArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    if ctx.dry_run {
        return Ok(plan(
            "app.launch",
            &device.serial,
            session.as_ref(),
            format!("Would launch {}\n", args.package),
        ));
    }
    let package = args.package.clone();
    let activity = args.activity.clone();
    let wake = !args.no_wake;
    execute_in_run!(
        ctx,
        "app.launch",
        Some("app launch"),
        session.as_ref(),
        Some(&device),
        true,
        |_run| async {
            let app = ctx.app_manager()?;
            let result = app
                .launch(&device.serial, &package, activity.as_deref(), wake)
                .await?;
            let human = format!(
                "Launched {}{} foreground={}\n",
                package,
                result
                    .component
                    .as_ref()
                    .map(|component| format!(" ({component})"))
                    .unwrap_or_default(),
                result.foreground
            );
            Ok(success("app.launch", serde_json::to_value(&result)?, human)
                .with_optional_session(session_id(session.as_ref()))
                .with_device(device.serial.clone()))
        },
    )
    .await
}

async fn force_stop(ctx: &CliContext, args: &AppPackageArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    if ctx.dry_run {
        return Ok(plan(
            "app.force-stop",
            &device.serial,
            session.as_ref(),
            format!("Would force-stop {}\n", args.package),
        ));
    }
    let package = args.package.clone();
    execute_in_run!(
        ctx,
        "app.force-stop",
        Some("app force-stop"),
        session.as_ref(),
        Some(&device),
        true,
        |_run| async {
            let app = ctx.app_manager()?;
            app.force_stop(&device.serial, &package).await?;
            Ok(success(
                "app.force-stop",
                serde_json::json!({"package": package, "stopped": true}),
                format!("Force-stopped {package}\n"),
            )
            .with_optional_session(session_id(session.as_ref()))
            .with_device(device.serial.clone()))
        },
    )
    .await
}

async fn kill(ctx: &CliContext, args: &AppPackageArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    if ctx.dry_run {
        return Ok(plan(
            "app.kill",
            &device.serial,
            session.as_ref(),
            format!("Would send am kill for {}\n", args.package),
        ));
    }
    let package = args.package.clone();
    execute_in_run!(
        ctx,
        "app.kill",
        Some("app kill"),
        session.as_ref(),
        Some(&device),
        true,
        |_run| async {
            let app = ctx.app_manager()?;
            app.kill(&device.serial, &package).await?;
            Ok(success(
                "app.kill",
                serde_json::json!({"package": package, "note": "am kill only terminates background processes"}),
                format!("Sent am kill for {package}; foreground processes are intentionally not killed\n"),
            )
            .with_optional_session(session_id(session.as_ref()))
            .with_device(device.serial.clone()))
        },
    )
    .await
}

async fn status(ctx: &CliContext, args: &AppStatusArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let package = args.package.clone();
    let expectation = match &args.expect {
        Some(value) => Some(parse_expectation(value)?),
        None => None,
    };
    let timeout = ctx.parse_timeout(args.timeout.as_deref(), ctx.config.file.timeouts.ui_secs)?;
    let stable_for = ctx.parse_timeout(args.stable_for.as_deref(), 2)?;
    let assertions = args.assert_task_exists
        || args.assert_task_absent
        || args.assert_process_stable
        || expectation.is_some();
    if !assertions {
        let app = ctx.app_manager()?;
        let states = app.status(&device.serial, &package).await?;
        return Ok(success(
            "app.status",
            serde_json::to_value(&states)?,
            status_human(&states),
        )
        .with_optional_session(session_id(session.as_ref()))
        .with_device(device.serial));
    }
    let expect_value = args.expect.clone();
    execute_in_run!(
        ctx,
        "app.status",
        Some("app status assertion"),
        session.as_ref(),
        Some(&device),
        true,
        |_run| async {
            let app = ctx.app_manager()?;
            let mut states = if let Some(expectation) = expectation {
                app.wait_for_expectation(&device.serial, &package, expectation, timeout)
                    .await?
            } else {
                app.status(&device.serial, &package).await?
            };
            if args.assert_task_exists && !states.task_exists {
                return Err(assertion_error(
                    &device.serial,
                    &states,
                    "task did not exist",
                ));
            }
            if args.assert_task_absent && states.task_exists {
                return Err(assertion_error(
                    &device.serial,
                    &states,
                    "task still exists",
                ));
            }
            if args.assert_process_stable {
                states = app
                    .assert_process_stable(&device.serial, &package, stable_for)
                    .await?;
            }
            Ok(success(
                "app.status",
                serde_json::json!({"expectation": expect_value, "states": states}),
                status_human(&states),
            )
            .with_optional_session(session_id(session.as_ref()))
            .with_device(device.serial.clone()))
        },
    )
    .await
}

fn assertion_error(serial: &str, states: &AppStates, message: &str) -> ForgeError {
    ForgeError::new(ErrorCode::UiAssertionFailed, message)
        .with_context(
            "states",
            serde_json::to_value(states).unwrap_or(serde_json::Value::Null),
        )
        .with_suggestion("Inspect the application with `androidforge app status <package>`.")
        .with_suggestion("Capture evidence with `androidforge diagnostics collect`.")
        .device(serial.to_string())
}

fn status_human(states: &AppStates) -> String {
    format!(
        "{}\ninstalled={} version={:?} process_exists={} process_alive={} pid={:?} task_exists={} activity_exists={} foreground={} resumed={}\n",
        states.package,
        states.installed,
        states.version_name,
        states.process_exists,
        states.process_alive,
        states.process.pid,
        states.task_exists,
        states.activity_exists,
        states.foreground,
        states.resumed
    )
}

fn parse_expectation(value: &str) -> Result<AppStateExpectation> {
    match value {
        "installed" => Ok(AppStateExpectation::Installed),
        "not_installed" | "not-installed" => Ok(AppStateExpectation::NotInstalled),
        "process_running" | "process-running" => Ok(AppStateExpectation::ProcessRunning),
        "process_stopped" | "process-stopped" => Ok(AppStateExpectation::ProcessStopped),
        "process_alive" | "process-alive" => Ok(AppStateExpectation::ProcessAlive),
        "task_exists" | "task-exists" => Ok(AppStateExpectation::TaskExists),
        "task_absent" | "task-absent" => Ok(AppStateExpectation::TaskAbsent),
        "foreground" => Ok(AppStateExpectation::Foreground),
        "background" => Ok(AppStateExpectation::Background),
        "resumed" => Ok(AppStateExpectation::Resumed),
        other => Err(ForgeError::invalid_argument(format!(
            "unknown app expectation '{other}'"
        ))
        .with_suggestion("Use foreground, background, resumed, process_alive, process_stopped, task_exists, task_absent, installed or not_installed.")),
    }
}

async fn info(ctx: &CliContext, args: &AppPackageArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let app = ctx.app_manager()?;
    let info = app.info(&device.serial, &args.package).await?;
    let permissions = androidforge_app::Permissions::new(ctx.adb()?);
    let notification = permissions
        .check(
            &device.serial,
            &args.package,
            "android.permission.POST_NOTIFICATIONS",
        )
        .await
        .ok();
    let result = serde_json::json!({
        "info": info,
        "post_notifications": notification,
    });
    let human = format!(
        "{}\nlauncher: {}\nforeground: {}\n",
        info.states.summary(),
        info.launcher_activity.as_deref().unwrap_or("-"),
        info.states.foreground
    );
    Ok(success("app.info", result, human)
        .with_optional_session(session_id(session.as_ref()))
        .with_device(device.serial))
}

async fn list(ctx: &CliContext, args: &AppListArgs) -> Result<CommandSuccess> {
    let adb = ctx.adb()?;
    let session = ctx.load_session(true)?;
    let serial = ctx.resolve_serial(session.as_ref()).await?;
    let output = adb
        .shell_args(
            &serial,
            &{
                let mut command =
                    vec!["pm".to_string(), "list".to_string(), "packages".to_string()];
                if args.third_party {
                    command.push("-3".to_string());
                }
                command
            },
            Some(Duration::from_secs(30)),
            None,
        )
        .await;
    let output = match output {
        Ok(output) => output,
        Err(error) => return Err(error),
    };
    let packages = output
        .stdout_text()
        .lines()
        .filter_map(|line| line.trim().strip_prefix("package:"))
        .filter(|package| {
            args.filter
                .as_deref()
                .map(|filter| package.contains(filter))
                .unwrap_or(true)
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    Ok(success(
        "app.list",
        serde_json::json!({"packages": packages, "count": packages.len()}),
        packages.join("\n") + "\n",
    )
    .with_optional_session(session_id(session.as_ref()))
    .with_device(serial))
}

pub async fn apk_inspect(ctx: &CliContext, apk: &Path) -> Result<CommandSuccess> {
    let info = ctx.inspector().inspect(apk, false).await?;
    let human = format!(
        "{}\npackage: {}\nversion: {} ({})\nminSdk: {:?} targetSdk: {:?}\nsha256: {}\nsigned: {:?} verified: {:?}\ncert SHA-256: {}\ndebug signed: {:?}\n",
        info.path,
        info.package.as_deref().unwrap_or("-"),
        info.version_name.as_deref().unwrap_or("-"),
        info.version_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "-".to_string()),
        info.min_sdk,
        info.target_sdk,
        info.sha256,
        info.signature.signed,
        info.signature.verified,
        info.signature.cert_sha256.as_deref().unwrap_or("-"),
        info.signature.debug_signed(),
    );
    Ok(success("apk.inspect", serde_json::to_value(&info)?, human))
}

pub async fn apk_verify(ctx: &CliContext, args: &ApkVerifyArgs) -> Result<CommandSuccess> {
    let options = androidforge_app::VerifyOptions {
        expected_cert_sha256: args.expected_cert_sha256.clone(),
        expected_package: args.expected_package.clone(),
        min_version_code: args.min_version_code,
    };
    let report = ctx.inspector().verify(&args.apk, &options).await?;
    if !report.passed {
        let failed = report
            .checks
            .iter()
            .filter(|check| !check.passed)
            .map(|check| check.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ForgeError::new(
            ErrorCode::SignatureVerificationFailed,
            format!("APK verification failed: {failed}"),
        )
        .with_context(
            "checks",
            serde_json::to_value(&report.checks).unwrap_or(serde_json::Value::Null),
        )
        .with_context(
            "apk",
            serde_json::to_value(&report.apk).unwrap_or(serde_json::Value::Null),
        )
        .with_suggestion("Inspect the checks and expected certificate fingerprint.")
        .with_suggestion("AndroidForge never signs or republishes APKs."));
    }
    let human = format!(
        "APK verification passed for {}\npackage: {}\nversion: {}\ncert SHA-256: {}\n",
        report.apk.path,
        report.apk.package.as_deref().unwrap_or("-"),
        report.apk.version_name.as_deref().unwrap_or("-"),
        report.apk.signature.cert_sha256.as_deref().unwrap_or("-")
    );
    Ok(success("apk.verify", serde_json::to_value(&report)?, human))
}

fn local_path(ctx: &CliContext, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        ctx.paths.cwd.join(path)
    }
}

fn plan(command: &str, serial: &str, session: Option<&Session>, human: String) -> CommandSuccess {
    success(
        command,
        serde_json::json!({"dry_run": true, "serial": serial}),
        human,
    )
    .with_device(serial.to_string())
    .with_optional_session(session.map(|session| session.id.clone()))
}

fn session_id(session: Option<&Session>) -> Option<String> {
    session.map(|session| session.id.clone())
}
