use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use androidforge_app::{
    AppFiles, AppStateExpectation, AppStates, InstallOptions, IntentExtra, IntentOutput,
    IntentSpec, inspect_installed,
};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::paths::ForgePaths;
use androidforge_core::session::Session;
use androidforge_core::util::canonicalize_or_self;
use androidforge_ui::{NodeExpectation, UiHierarchy, UiManager};
use serde::{Deserialize, Serialize};

use crate::cli::{
    ApkVerifyArgs, AppBroadcastArgs, AppCommand, AppDataArgs, AppDataCommand, AppDataExportArgs,
    AppDataResetArgs, AppDataSeedArgs, AppInstallArgs, AppLaunchArgs, AppListArgs, AppPackageArgs,
    AppServiceArgs, AppStatusArgs, AppUninstallArgs, AppVerifyInstalledArgs, Cli,
};
use crate::commands::common::success;
use crate::context::{CliContext, Risk};

pub async fn app(cli: &Cli, ctx: &CliContext, command: &AppCommand) -> Result<CommandSuccess> {
    match command {
        AppCommand::Install(args) => install_with_confirmation(cli, ctx, args).await,
        AppCommand::Uninstall(args) => uninstall(ctx, args).await,
        AppCommand::ClearData(args) => clear_data(ctx, args).await,
        AppCommand::Launch(args) => launch(ctx, args).await,
        AppCommand::Broadcast(args) => broadcast(ctx, args).await,
        AppCommand::Service(args) => service(ctx, args).await,
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
                let mut result = serde_json::json!({
                    "dry_run": true,
                    "serial": device.serial,
                    "package": package,
                    "local": local,
                    "remote": args.remote,
                    "requires_confirmation": device.kind == androidforge_core::model::DeviceKind::Physical,
                });
                if device.kind == androidforge_core::model::DeviceKind::Physical {
                    result["requires_physical_mutation"] = serde_json::json!(true);
                }
                return Ok(success(
                    "app.file-push",
                    result,
                    format!(
                        "Would push {} to {} on {}{}\n",
                        local.display(),
                        args.remote,
                        package,
                        if ctx.risk_allowed(Risk::Data) {
                            ""
                        } else if device.kind == androidforge_core::model::DeviceKind::Physical {
                            "; requires --allow-physical-data (or --allow-physical-mutation)"
                        } else {
                            ""
                        }
                    ),
                )
                .with_optional_session(session_id(session.as_ref()))
                .with_device(device.serial.clone()));
            }
            ctx.ensure_risk_allowed(&device, Risk::Data, "pushing files into an app")?;
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
        AppCommand::DataBackup(args) => app_data_export(ctx, args, "app.data-backup").await,
        AppCommand::DataRestore(args) => app_data_import(ctx, args, "app.data-restore").await,
        AppCommand::VerifyInstalled(args) => verify_installed(ctx, args).await,
        AppCommand::Data { command } => match command {
            AppDataCommand::Export(args) => app_data_export(ctx, args, "app.data-export").await,
            AppDataCommand::Import(args) => app_data_import(ctx, args, "app.data-import").await,
            AppDataCommand::Reset(args) => app_data_reset(ctx, args).await,
            AppDataCommand::Seed(args) => app_data_seed(ctx, args).await,
        },
    }
}

// ---------------------------------------------------------------------------
// Safe `app data export` destination resolution
//
// The resolver below is deliberately free of device/ADB state so it can be
// reused by the CLI, MCP and tests. The `app data export` CLI flags
// `--output`, `--allow-in-project` and `--overwrite` map directly onto the
// policy below; the MCP `androidforge_app_data` tool forwards the same flags.
// The default destination lives under the managed artifacts/export area
// instead of a caller-chosen project source path.
// ---------------------------------------------------------------------------

/// What to do when an export destination already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportOverwritePolicy {
    /// Refuse to touch an existing file. This is the default.
    #[default]
    Fail,
    /// Explicitly allow replacing an existing file. Callers must opt in.
    Replace,
}

/// Typed policy for resolving an `app data export` destination.
///
/// `output` is the future explicit `--output` value; `None` selects the safe
/// default under `<artifacts_dir>/export`. A destination inside the project
/// root is refused unless `allow_in_project` is explicitly set (the managed
/// default is exempt). Existing files are never replaced unless `overwrite`
/// is explicitly set to [`ExportOverwritePolicy::Replace`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportDestinationPolicy {
    pub output: Option<PathBuf>,
    pub allow_in_project: bool,
    pub overwrite: ExportOverwritePolicy,
}

impl Default for ExportDestinationPolicy {
    fn default() -> Self {
        Self {
            output: None,
            allow_in_project: false,
            overwrite: ExportOverwritePolicy::Fail,
        }
    }
}

impl ExportDestinationPolicy {
    /// Policy for a caller-supplied output path, with all safety guards on.
    pub fn explicit(path: impl Into<PathBuf>) -> Self {
        Self {
            output: Some(path.into()),
            ..Self::default()
        }
    }
}

/// How the destination was chosen, reported for auditability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportDestinationSource {
    Default,
    Explicit,
}

/// A fully resolved, policy-approved export destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedExportDestination {
    pub path: PathBuf,
    pub source: ExportDestinationSource,
    /// True when the destination is inside the project root.
    pub in_project: bool,
    /// True when the destination is inside the managed artifacts/export area.
    pub in_export_area: bool,
}

/// Resolve an export destination without touching the filesystem except for
/// metadata checks (existence, canonicalization).
///
/// Relative explicit paths are resolved against `paths.cwd`; `..` segments are
/// always rejected so a relative path cannot escape the working directory.
/// `stamp` is caller-supplied (a timestamp or run id) so the default filename
/// is deterministic for tests and reproducible for callers.
pub fn resolve_export_destination(
    paths: &ForgePaths,
    package: &str,
    stamp: &str,
    policy: &ExportDestinationPolicy,
) -> Result<ResolvedExportDestination> {
    let package = export_name_part(package, "package")?;
    let stamp = export_name_part(stamp, "stamp")?;
    let export_area = paths.artifacts_dir.join("export");

    let (path, source) = match policy.output.as_ref() {
        Some(output) if !output.as_os_str().is_empty() => (
            normalize_destination(output, &paths.cwd)?,
            ExportDestinationSource::Explicit,
        ),
        _ => (
            export_area
                .join(&package)
                .join(format!("{package}-{stamp}.tar")),
            ExportDestinationSource::Default,
        ),
    };

    let in_project = path_within(&paths.project_root, &path);
    let in_export_area = path_within(&export_area, &path);
    // The managed default is allowed even when `artifact_dir` is configured
    // inside the project root; an explicit in-project destination is not.
    let managed_default =
        source == ExportDestinationSource::Default && path.starts_with(&export_area);
    if in_project && !policy.allow_in_project && !managed_default {
        return Err(destination_error(
            "destination_inside_project",
            format!(
                "export destination {} is inside the project; pass allow_in_project to override or use the artifacts/export default",
                path.display()
            ),
            &path,
            in_project,
            in_export_area,
        )
        .with_suggestion(
            "Use the default artifacts/export destination, or set allow_in_project explicitly.",
        ));
    }

    if path.is_dir() {
        return Err(destination_error(
            "destination_is_directory",
            format!("export destination {} is a directory", path.display()),
            &path,
            in_project,
            in_export_area,
        ));
    }
    if path.exists() && policy.overwrite != ExportOverwritePolicy::Replace {
        return Err(destination_error(
            "destination_exists",
            format!("export destination {} already exists", path.display()),
            &path,
            in_project,
            in_export_area,
        )
        .with_suggestion("Choose another path or set the explicit overwrite policy."));
    }

    Ok(ResolvedExportDestination {
        path,
        source,
        in_project,
        in_export_area,
    })
}

fn export_name_part(value: &str, field: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return Err(ForgeError::invalid_argument(format!(
            "export {field} must be a non-empty path component"
        )));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains('\0') {
        return Err(destination_error(
            "destination_name_invalid",
            format!("export {field} must not contain path separators: {trimmed}"),
            Path::new(trimmed),
            false,
            false,
        ));
    }
    Ok(trimmed.to_string())
}

/// Resolve an explicit destination against a base directory, rejecting any
/// parent-directory segment before it can be interpreted by the filesystem.
fn normalize_destination(path: &Path, base: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(ForgeError::invalid_argument("export destination is empty"));
    }
    let absolute = path.is_absolute();
    let mut resolved = if absolute {
        PathBuf::new()
    } else {
        base.to_path_buf()
    };
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => resolved.push(part),
            Component::Prefix(prefix) if absolute => resolved.push(prefix.as_os_str()),
            Component::RootDir if absolute => resolved.push(Component::RootDir.as_os_str()),
            Component::ParentDir => {
                return Err(destination_error(
                    "destination_traversal",
                    format!(
                        "export destination {} must not contain '..' segments",
                        path.display()
                    ),
                    path,
                    false,
                    false,
                )
                .with_suggestion("Use an absolute path or a relative path without '..'."));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(destination_error(
                    "destination_not_relative",
                    format!(
                        "export destination {} is not a plain relative path",
                        path.display()
                    ),
                    path,
                    false,
                    false,
                ));
            }
        }
    }
    if resolved.as_os_str().is_empty() {
        return Err(ForgeError::invalid_argument("export destination is empty"));
    }
    Ok(resolved)
}

/// Canonicalize existing parents so symlinks cannot hide containment, while
/// still supporting destinations that do not exist yet.
fn canonicalized_for_compare(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            canonicalize_or_self(parent).join(path.file_name().unwrap_or_default())
        }
        _ => canonicalize_or_self(path),
    }
}

fn path_within(root: &Path, path: &Path) -> bool {
    let root = canonicalize_or_self(root);
    let path = canonicalized_for_compare(path);
    path == root || path.starts_with(&root)
}

fn destination_error(
    reason: &'static str,
    message: String,
    path: &Path,
    in_project: bool,
    in_export_area: bool,
) -> ForgeError {
    ForgeError::new(ErrorCode::InvalidArgument, message)
        .with_context_string("reason", reason)
        .with_context_string("path", path.display().to_string())
        .with_context("in_project", in_project)
        .with_context("in_export_area", in_export_area)
}

async fn app_data_export(
    ctx: &CliContext,
    args: &AppDataExportArgs,
    command: &'static str,
) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    // The named `--output` flag and the legacy positional path are equivalent;
    // an explicit destination is always validated by the safe resolver. With
    // neither, the managed artifacts/export default is selected.
    let overwrite = if args.overwrite {
        ExportOverwritePolicy::Replace
    } else {
        ExportOverwritePolicy::Fail
    };
    let mut policy = match args.output.clone().or_else(|| args.path.clone()) {
        Some(output) => ExportDestinationPolicy::explicit(output),
        None => ExportDestinationPolicy::default(),
    };
    policy.allow_in_project = args.allow_in_project;
    policy.overwrite = overwrite;
    let destination = resolve_export_destination(&ctx.paths, &args.package, &stamp, &policy)?;
    let path = destination.path.clone();
    if ctx.dry_run {
        let source = match destination.source {
            ExportDestinationSource::Default => "default",
            ExportDestinationSource::Explicit => "explicit",
        };
        let result = serde_json::json!({
            "dry_run": true,
            "serial": device.serial,
            "package": args.package,
            "path": path.to_string_lossy().into_owned(),
            "source": source,
            "in_project": destination.in_project,
            "in_export_area": destination.in_export_area,
            "overwrite": args.overwrite,
        });
        return Ok(success(
            command,
            result,
            format!(
                "Would export {} data to {} ({} destination, in_project={}, in_export_area={}){}\n",
                args.package,
                path.display(),
                source,
                destination.in_project,
                destination.in_export_area,
                if args.overwrite { ", overwriting" } else { "" },
            ),
        )
        .with_optional_session(session_id(session.as_ref()))
        .with_device(device.serial.clone()));
    }
    let package = args.package.clone();
    execute_in_run!(
        ctx,
        command,
        Some("app data export"),
        session.as_ref(),
        Some(&device),
        true,
        |run| async {
            let files = AppFiles::new(ctx.adb()?);
            let transfer = files.backup_data(&device.serial, &package, &path).await?;
            let artifact = run.write_text_artifact(
                "logs/app-data-export.json",
                "app_data_export",
                Some("App data export metadata"),
                &serde_json::to_string_pretty(&transfer).unwrap_or_default(),
            )?;
            Ok(success(
                command,
                serde_json::to_value(&transfer)?,
                format!(
                    "Exported {} data to {} ({})\n",
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

async fn app_data_import(
    ctx: &CliContext,
    args: &AppDataArgs,
    command: &'static str,
) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    if ctx.dry_run {
        let mut result = serde_json::json!({
            "dry_run": true,
            "serial": device.serial,
            "package": args.package,
            "path": args.path,
            "requires_confirmation": true,
        });
        if device.kind == androidforge_core::model::DeviceKind::Physical {
            result["requires_physical_mutation"] = serde_json::json!(true);
        }
        return Ok(success(
            command,
            result,
            format!(
                "Would import {} data from {}; requires --yes{}\n",
                args.package,
                args.path.display(),
                if device.kind == androidforge_core::model::DeviceKind::Physical
                    && !ctx.risk_allowed(Risk::Data)
                {
                    " and --allow-physical-data (or --allow-physical-mutation)"
                } else {
                    ""
                }
            ),
        )
        .with_optional_session(session_id(session.as_ref()))
        .with_device(device.serial.clone()));
    }
    if !ctx.yes {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            "importing app data overwrites app-private files; pass --yes",
        )
        .with_suggestion("Back up the current data first where the app is debuggable."));
    }
    ctx.ensure_risk_allowed(&device, Risk::Data, "importing app data")?;
    let package = args.package.clone();
    let path = local_path(ctx, &args.path);
    execute_in_run!(
        ctx,
        command,
        Some("app data import"),
        session.as_ref(),
        Some(&device),
        true,
        |run| async {
            let files = AppFiles::new(ctx.adb()?);
            let backup_path = run.dir.join("data").join("pre-import-backup.tar");
            if let Some(parent) = backup_path.parent() {
                androidforge_core::util::ensure_dir(parent)?;
            }
            let backup = files
                .backup_data(&device.serial, &package, &backup_path)
                .await?;
            let backup_artifact = run.register_artifact(
                &backup_path,
                "app_data_pre_import",
                Some("App-private data captured before the import"),
            )?;
            let transfer = files.restore_data(&device.serial, &package, &path).await?;
            let artifact = run.write_text_artifact(
                "logs/app-data-import.json",
                "app_data_import",
                Some("App data import metadata"),
                &serde_json::to_string_pretty(&transfer).unwrap_or_default(),
            )?;
            Ok(success(
                command,
                serde_json::json!({"transfer": transfer, "backup": backup}),
                format!("Imported {} data from {}\n", package, path.display()),
            )
            .with_artifacts(vec![backup_artifact, artifact])
            .with_optional_session(session_id(session.as_ref()))
            .with_device(device.serial.clone()))
        },
    )
    .await
}

async fn app_data_reset(ctx: &CliContext, args: &AppDataResetArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let backup_path = args
        .backup_current
        .as_ref()
        .map(|path| local_path(ctx, path));
    let baseline_path = args.baseline.as_ref().map(|path| local_path(ctx, path));
    if ctx.dry_run {
        return Ok(plan(
            "app.data-reset",
            &device.serial,
            session.as_ref(),
            format!(
                "Would clear {} data{}{}\n",
                args.package,
                backup_path
                    .as_ref()
                    .map(|path| format!(", save the current data to {}", path.display()))
                    .unwrap_or_default(),
                baseline_path
                    .as_ref()
                    .map(|path| format!(", then restore baseline {}", path.display()))
                    .unwrap_or_default()
            ),
        ));
    }
    if !ctx.yes {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            "resetting app data is destructive; pass --yes",
        )
        .with_suggestion("Use --backup-current <path> to preserve the current data first."));
    }
    ctx.ensure_risk_allowed(&device, Risk::Data, "resetting app data")?;
    let package = args.package.clone();
    execute_in_run!(
        ctx,
        "app.data-reset",
        Some("app data reset"),
        session.as_ref(),
        Some(&device),
        true,
        |_run| async {
            let files = AppFiles::new(ctx.adb()?);
            let mut backup = None;
            if let Some(path) = &backup_path {
                backup = Some(files.backup_data(&device.serial, &package, path).await?);
            }
            let app = ctx.app_manager()?;
            let clear_output = app.clear_data(&device.serial, &package).await?;
            let restore = if let Some(path) = &baseline_path {
                Some(files.restore_data(&device.serial, &package, path).await?)
            } else {
                None
            };
            let mut result = serde_json::json!({
                "package": package,
                "clear_output": clear_output,
            });
            if let Some(backup) = backup {
                result["backup"] = serde_json::to_value(backup)?;
            }
            if let Some(restore) = restore {
                result["restored"] = serde_json::to_value(restore)?;
            }
            Ok(success(
                "app.data-reset",
                result,
                format!("Reset data for {package}\n"),
            )
            .with_optional_session(session_id(session.as_ref()))
            .with_device(device.serial.clone()))
        },
    )
    .await
}

async fn app_data_seed(ctx: &CliContext, args: &AppDataSeedArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let local = local_path(ctx, &args.file);
    if ctx.dry_run {
        return Ok(plan(
            "app.data-seed",
            &device.serial,
            session.as_ref(),
            format!(
                "Would seed {} with {} at {}\n",
                args.package,
                local.display(),
                args.remote
            ),
        ));
    }
    if !ctx.yes {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            "seeding app-private files changes application state; pass --yes",
        )
        .with_suggestion("Use --dry-run to inspect the target path first."));
    }
    ctx.ensure_risk_allowed(&device, Risk::Data, "seeding app data")?;
    let package = args.package.clone();
    let remote = args.remote.clone();
    execute_in_run!(
        ctx,
        "app.data-seed",
        Some("app data seed"),
        session.as_ref(),
        Some(&device),
        true,
        |run| async {
            let files = AppFiles::new(ctx.adb()?);
            let transfer = files
                .push(&device.serial, &package, &local, &remote, false)
                .await?;
            let artifact = run.write_text_artifact(
                "logs/app-data-seed.json",
                "app_data_seed",
                Some("App data seed metadata"),
                &serde_json::to_string_pretty(&transfer).unwrap_or_default(),
            )?;
            Ok(success(
                "app.data-seed",
                serde_json::to_value(&transfer)?,
                format!(
                    "Seeded {} with {} ({})\n",
                    package, remote, transfer.size_bytes
                ),
            )
            .with_artifacts(vec![artifact])
            .with_optional_session(session_id(session.as_ref()))
            .with_device(device.serial.clone()))
        },
    )
    .await
}

async fn verify_installed(
    ctx: &CliContext,
    args: &AppVerifyInstalledArgs,
) -> Result<CommandSuccess> {
    use androidforge_app::compare;
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let source = local_path(ctx, &args.apk);
    let artifact =
        crate::commands::apk::load_artifact_with_splits(ctx, &source, &args.split).await?;
    let package = artifact.base.package_or_error()?.to_string();
    let live = inspect_installed(&ctx.adb()?, &ctx.inspector(), &device.serial, &package).await?;
    let Some(live) = live else {
        return Err(ForgeError::new(
            ErrorCode::PackageNotFound,
            format!("package {package} is not installed on {}", device.serial),
        )
        .with_suggestion("Install the application or verify the package id.")
        .device(device.serial));
    };
    let diff = compare(&artifact.base, &artifact.splits, &live);
    let verified = diff.safe_to_skip_install;
    let result = serde_json::json!({
        "package": package,
        "verified": verified,
        "local": artifact.summary(),
        "installed": live,
        "diff": diff,
    });
    if !verified {
        return Err(ForgeError::new(
            ErrorCode::InstallVerificationFailed,
            format!(
                "installed {package} does not match the local APK: {}",
                diff.summary
            ),
        )
        .with_context("verification", result)
        .with_suggestion(
            "Install the local APK with `androidforge app install <apk>` if that is intended.",
        )
        .with_suggestion(
            "Use `androidforge app status --expect-apk-sha256 <sha>` for a compact assertion.",
        )
        .device(device.serial));
    }
    Ok(success(
        "app.verify-installed",
        result,
        format!(
            "Installed {package} matches the local APK ({})\n",
            artifact.base.sha256
        ),
    )
    .with_optional_session(session_id(session.as_ref()))
    .with_device(device.serial))
}
/// Device-independent description of one install. `build --install` and
/// `app install` both funnel through `install_request`, so identity checks,
/// physical gating and OEM confirmation are shared rather than reimplemented.
#[derive(Debug, Clone)]
pub(crate) struct InstallRequest {
    pub apk: PathBuf,
    pub split: Vec<PathBuf>,
    pub fresh: bool,
    pub downgrade: bool,
    pub no_verify: bool,
    pub expected_package: Option<String>,
    pub expected_cert_sha256: Option<String>,
    pub timeout: Option<String>,
    pub only_if_changed: bool,
}

impl InstallRequest {
    fn from_args(args: &AppInstallArgs) -> Self {
        Self {
            apk: args.apk.clone(),
            split: args.split.clone(),
            fresh: args.fresh,
            downgrade: args.downgrade,
            no_verify: args.no_verify,
            expected_package: args.expected_package.clone(),
            expected_cert_sha256: args.expected_cert_sha256.clone(),
            timeout: args.timeout.clone(),
            only_if_changed: args.only_if_changed,
        }
    }
}

pub(crate) async fn install(ctx: &CliContext, args: &AppInstallArgs) -> Result<CommandSuccess> {
    install_request(ctx, &InstallRequest::from_args(args)).await
}

/// CLI entry point. The confirmation flags are global so agent-driven installs
/// keep constructing `AppInstallArgs` unchanged; only this entry point opts in.
pub(crate) async fn install_with_confirmation(
    cli: &Cli,
    ctx: &CliContext,
    args: &AppInstallArgs,
) -> Result<CommandSuccess> {
    // Without the opt-in flag this is a plain, unchanged install. The OEM
    // confirmation path only engages after an install actually fails with a
    // restricted-user/vendor-prompt signature, and it never bypasses the
    // Risk::Install gate enforced by `install_request`.
    if !cli.wait_for_confirmation {
        return install_request(ctx, &InstallRequest::from_args(args)).await;
    }
    let options = OemInstallConfirmationOptions {
        enabled: true,
        confirm_text: cli.confirm_text.clone(),
        timeout: ctx.parse_timeout(cli.confirmation_timeout.as_deref(), 15)?,
        poll_interval: Duration::from_millis(400),
    };
    let first = install_request(ctx, &InstallRequest::from_args(args)).await;
    let error = match first {
        Ok(success) => return Ok(success),
        Err(error) => error,
    };
    if !install_error_needs_oem_confirmation(&error) {
        return Err(error);
    }
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let serial = device.serial.clone();
    let ui = ctx.ui_manager()?;
    let confirmation = handle_oem_install_confirmation(&ui, &serial, &options, &error).await?;
    if confirmation.outcome != "confirmed" {
        return Err(error.with_context(
            "oem_confirmation",
            serde_json::to_value(&confirmation).unwrap_or(serde_json::Value::Null),
        ));
    }
    // A successful confirmation buys exactly one retry. A second failure is
    // surfaced as-is, with the confirmation metadata attached for diagnosis.
    let retry = install_request(ctx, &InstallRequest::from_args(args)).await;
    match retry {
        Ok(mut success) => {
            let metadata = serde_json::to_value(&confirmation).unwrap_or(serde_json::Value::Null);
            if let Some(object) = success.result.as_object_mut() {
                object.insert("oem_confirmation".to_string(), metadata);
            }
            success.human.push_str(&format!(
                "OEM install confirmation: tapped {:?} at ({:?}, {:?}) after {} attempt(s).\n",
                confirmation.label, confirmation.tap_x, confirmation.tap_y, confirmation.attempts
            ));
            Ok(success)
        }
        Err(retry_error) => Err(retry_error.with_context(
            "oem_confirmation",
            serde_json::to_value(&confirmation).unwrap_or(serde_json::Value::Null),
        )),
    }
}

pub(crate) async fn install_request(
    ctx: &CliContext,
    args: &InstallRequest,
) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let source = local_path(ctx, &args.apk);
    let artifact =
        crate::commands::apk::load_artifact_with_splits(ctx, &source, &args.split).await?;
    let mut options = InstallOptions::new(artifact.base_path());
    options.split_apks = artifact.split_paths();
    options.artifact_kind = Some(artifact.kind.as_str().to_string());
    options.archive_sha256 = artifact.archive_sha256.clone();
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
        let mut apk = artifact.base.clone();
        apk.path = source.to_string_lossy().into_owned();
        let package = apk.package.clone();
        let install_mode = if artifact.splits.is_empty() {
            "install"
        } else {
            "install-multiple"
        };
        let mut result = serde_json::json!({
            "dry_run": true,
            "serial": device.serial,
            "apk": apk,
            "artifact": artifact.summary(),
            "install": {
                "mode": install_mode,
                "fresh": options.fresh,
                "allow_downgrade": options.allow_downgrade,
                "verify_installed": options.verify_installed,
                "expected_package": options.expected_package,
                "allowed_packages": options.allowed_packages,
                "expected_cert_sha256": options.expected_cert_sha256,
                "only_if_changed": options.only_if_changed,
                "split_count": artifact.split_count(),
                "splits": artifact.splits,
            }
        });
        if options.fresh && !ctx.yes {
            result["install"]["requires_confirmation"] = serde_json::json!(true);
        }
        if device.kind == androidforge_core::model::DeviceKind::Physical {
            result["install"]["requires_physical_mutation"] =
                serde_json::json!(!ctx.risk_allowed(Risk::Install));
        }
        let mut human = format!(
            "Would install {} on {}{}{}\npackage: {}\nversion: {}\ncertificate: {}\nartifact: {}{}\n",
            source.display(),
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
            artifact.kind.as_str(),
            if artifact.split_count() > 0 {
                format!(
                    ", {} split(s) via adb install-multiple",
                    artifact.split_count()
                )
            } else {
                String::new()
            },
        );
        for warning in &artifact.warnings {
            human.push_str(&format!("warning: {warning}\n"));
        }
        return Ok(success("app.install", result, human)
            .with_optional_session(session_id(session.as_ref()))
            .with_device(device.serial.clone()));
    }
    ctx.ensure_risk_allowed(&device, Risk::Install, "installing an app")?;
    let apk_display = source.display().to_string();
    let outcome = execute_in_run!(
        ctx,
        "app.install",
        Some("app install"),
        session.as_ref(),
        Some(&device),
        true,
        |run| async {
            let app = ctx.app_manager()?;
            let mut result = app.install(&device.serial, &options).await.map_err(|error| {
                error.with_context_string(
                    "apk_path",
                    options.apk_path.to_string_lossy().into_owned(),
                )
            })?;
            let artifact_kind = result.artifact_kind.clone();
            let split_count = result.split_count;
            result.warnings.extend(artifact.warnings.iter().cloned());
            result.apk.path = apk_display.clone();
            result.build.apk_path = Some(apk_display.clone());
            run.set_build(result.build.clone());
            let identity = serde_json::to_string_pretty(&result.build).unwrap_or_default();
            let artifact_file = run.write_text_artifact(
                "build/install-identity.json",
                "build_identity",
                Some("Installed package, APK hash and signing certificate identity"),
                &identity,
            )?;
            let human = if result.skipped {
                format!(
                    "Installation skipped: {}\npackage: {}\nversion: {}\nbase APK SHA-256: {}\nartifact: {}{}\n",
                    result.reason.as_deref().unwrap_or("the installed build already matches"),
                    result.build.package.as_deref().unwrap_or("-"),
                    result.build.version_name.as_deref().unwrap_or("-"),
                    result.build.apk_sha256.as_deref().unwrap_or("-"),
                    artifact_kind.as_deref().unwrap_or("-"),
                    if split_count > 0 {
                        format!(" ({split_count} split(s) matched)")
                    } else {
                        String::new()
                    },
                )
            } else {
                format!(
                    "Installed {}\npackage: {}\nversion: {}\ninstalled APK SHA-256: {}\nsigning cert SHA-256: {}\ndebug signed: {}\nartifact: {}{}\n{}",
                    apk_display,
                    result.build.package.as_deref().unwrap_or("-"),
                    result.build.version_name.as_deref().unwrap_or("-"),
                    result.build.installed_apk_sha256.as_deref().or(result.build.apk_sha256.as_deref()).unwrap_or("-"),
                    result.build.cert_sha256.as_deref().unwrap_or("-"),
                    result.build.debug_signed.map(|value| value.to_string()).unwrap_or_else(|| "-".to_string()),
                    artifact_kind.as_deref().unwrap_or("-"),
                    if split_count > 0 {
                        format!(" ({split_count} split(s) installed via install-multiple)")
                    } else {
                        String::new()
                    },
                    result.reason.as_ref().map(|reason| format!("decision reason: {reason}\n")).unwrap_or_default(),
                )
            };
            let mut human = human;
            for warning in &result.warnings {
                human.push_str(&format!("warning: {warning}\n"));
            }
            Ok(success("app.install", serde_json::to_value(&result)?, human)
                .with_artifacts(vec![artifact_file])
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

// ---------------------------------------------------------------------------
// OEM install confirmation
//
// Some OEM skins show an on-device "allow installation" dialog and answer it
// automatically before `adb install` returns. The install then fails with
// INSTALL_FAILED_USER_RESTRICTED. The handler below watches that dialog through
// the audited UIAutomator snapshot path, taps a single unambiguous confirm
// button through `UiManager` (never a raw ADB bypass), and reports a structured
// error with a stable reason when the dialog is denied, ambiguous, or never
// appears. `app install --wait-for-confirmation` opts into this path and retries
// the verified install once only after an unambiguous tap.
// ---------------------------------------------------------------------------

/// Confirm labels used when the caller supplies no explicit overrides.
/// Matching is case-insensitive and accepts exact text or substring matches.
const DEFAULT_OEM_CONFIRM_LABELS: &[&str] = &[
    "install",
    "install anyway",
    "allow",
    "ok",
    "continue",
    "yes",
    "confirm",
    "安装",
    "仍然安装",
    "允许",
    "确定",
    "继续",
    "确认",
    "instalar",
    "permitir",
    "aceptar",
    "continuar",
    "installer",
    "autoriser",
    "continuer",
    "installieren",
    "zulassen",
    "weiter",
    "インストール",
    "許可",
];

/// Strong signals that the OEM dialog was answered negatively.
const OEM_DENIAL_MARKERS: &[&str] = &[
    "install blocked",
    "installation blocked",
    "install denied",
    "installation denied",
    "install cancelled",
    "installation cancelled",
    "install canceled",
    "installation canceled",
    "install rejected",
    "installation rejected",
    "blocked by admin",
    "blocked by your administrator",
    "安装被拒绝",
    "安装已取消",
    "安装被阻止",
    "已拒绝安装",
    "已取消安装",
    "拒绝安装",
];

const OEM_DENIAL_BUTTONS: &[&str] = &[
    "deny",
    "cancel",
    "reject",
    "don't allow",
    "do not allow",
    "取消",
    "拒绝",
    "不允许",
];

const OEM_DIALOG_MARKERS: &[&str] = &[
    "install",
    "安装",
    "instalar",
    "installer",
    "installieren",
    "permission",
    "权限",
    "unknown source",
    "未知来源",
    "allow",
    "允许",
    "confirm",
    "确认",
    "continue",
    "继续",
];

const OEM_CONFIRMATION_TRIGGERS: &[&str] = &[
    "install_failed_user_restricted",
    "user restricted",
    "user_restricted",
    "confirm the install",
    "confirm installation",
    "allow installation",
    "unknown sources",
    "未知来源",
    "允许安装",
];

/// Typed options for the OEM on-device install confirmation handler.
///
/// `enabled` is kept separate so the install CLI engages this handler only
/// when the caller opts in via `--wait-for-confirmation`; `confirm_text` comes
/// from repeatable `--confirm-text`, and the two durations bound the wait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OemInstallConfirmationOptions {
    pub enabled: bool,
    pub confirm_text: Vec<String>,
    pub timeout: Duration,
    pub poll_interval: Duration,
}

impl Default for OemInstallConfirmationOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            confirm_text: Vec::new(),
            timeout: Duration::from_secs(15),
            poll_interval: Duration::from_millis(400),
        }
    }
}

/// One actionable, labelled node in a dialog snapshot. `path` is used to
/// rebuild a `NodeExpectation` so the tap is identity-checked before dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OemDialogButton {
    /// Exact label from `text` (preferred) or `content-desc`, never lowercased.
    pub label: String,
    /// Stable path in the UI hierarchy, used for audited tap validation.
    pub path: String,
    pub clickable: bool,
}

/// A device-independent view of one UIAutomator snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OemDialogSnapshot {
    /// Exact UI strings from the hierarchy, joined with newlines in node order.
    pub dialog_text: String,
    /// Every visible, actionable labelled node, in hierarchy order.
    pub buttons: Vec<OemDialogButton>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OemDialogDecision {
    Pending,
    /// Index into the observed snapshot's `buttons`.
    Confirm(usize),
    Failed(OemConfirmationFailure),
}

/// All failure context carried into the structured error. `reason` is a stable
/// machine-readable value; the existing error codes are reused unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OemConfirmationFailure {
    pub reason: &'static str,
    pub dialog_text: String,
    pub candidates: Vec<OemDialogButton>,
    pub elapsed_ms: u64,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub cause: Option<String>,
}

impl OemConfirmationFailure {
    fn into_error(self, install_output: &str, serial: &str) -> ForgeError {
        // Dialog/observation failures are install failures; an ADB input error
        // is a command failure. Both codes already exist in the contract.
        let code = if self.reason == "confirmation_tap_failed" {
            ErrorCode::CommandFailed
        } else {
            ErrorCode::InstallFailed
        };
        let mut error = ForgeError::new(
            code,
            format!("OEM install confirmation failed ({})", self.reason),
        )
        .with_context_string("reason", self.reason)
        .with_context_string("dialog_text", self.dialog_text)
        .with_context(
            "candidates",
            serde_json::to_value(&self.candidates).unwrap_or(serde_json::Value::Null),
        )
        .with_context("elapsed_ms", self.elapsed_ms)
        .with_context("attempts", self.attempts)
        .with_context_string("install_output", install_output.trim().to_string())
        .device(serial.to_string())
        .with_suggestion(
            "Pass --wait-for-confirmation and --confirm-text <label> when the device uses a different confirm label.",
        )
        .with_suggestion(
            "Inspect the dialog with `androidforge ui dump` and keep the device screen unlocked during installation.",
        );
        if let Some(last_error) = self.last_error {
            error = error.with_context_string("last_error", last_error);
        }
        if let Some(cause) = self.cause {
            error = error.with_cause(cause);
        }
        error
    }
}

/// Pure dialog state machine. Each observed snapshot advances the state; the
/// driver supplies the same inputs from real UIAutomator dumps.
#[derive(Debug, Clone, Default)]
pub(crate) struct OemConfirmationTracker {
    attempts: u32,
    saw_dialog: bool,
    last_snapshot: Option<OemDialogSnapshot>,
}

impl OemConfirmationTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn observe(
        &mut self,
        snapshot: OemDialogSnapshot,
        options: &OemInstallConfirmationOptions,
        elapsed: Duration,
        timed_out: bool,
    ) -> OemDialogDecision {
        self.attempts += 1;
        if looks_like_oem_dialog(&snapshot) {
            self.saw_dialog = true;
        }
        let lower = snapshot.dialog_text.to_lowercase();
        if OEM_DENIAL_MARKERS
            .iter()
            .any(|marker| lower.contains(marker))
        {
            return self.fail("confirmation_auto_denied", snapshot, elapsed, None);
        }
        let matched = matching_confirm_buttons(&snapshot, options);
        if matched.len() == 1 {
            return OemDialogDecision::Confirm(matched[0]);
        }
        if matched.len() > 1 {
            return self.fail("multiple_confirm_buttons", snapshot, elapsed, None);
        }
        // A dialog whose only actionable choices are negative has already been
        // auto-denied; fail closed instead of waiting out the whole timeout.
        if self.saw_dialog
            && snapshot.buttons.iter().any(|button| {
                let label = button.label.to_lowercase();
                OEM_DENIAL_BUTTONS
                    .iter()
                    .any(|marker| label == *marker || label.contains(marker))
            })
        {
            return self.fail("confirmation_auto_denied", snapshot, elapsed, None);
        }
        if timed_out {
            let reason = if self.saw_dialog {
                "confirm_button_not_found"
            } else {
                "confirmation_dialog_timeout"
            };
            return self.fail(reason, snapshot, elapsed, None);
        }
        self.last_snapshot = Some(snapshot);
        OemDialogDecision::Pending
    }

    // Used by the async driver below, which is exposed ahead of the CLI flags.
    #[allow(dead_code)]
    pub(crate) fn last_snapshot(&self) -> Option<&OemDialogSnapshot> {
        self.last_snapshot.as_ref()
    }

    fn fail(
        &mut self,
        reason: &'static str,
        snapshot: OemDialogSnapshot,
        elapsed: Duration,
        cause: Option<String>,
    ) -> OemDialogDecision {
        let failure = OemConfirmationFailure {
            reason,
            dialog_text: snapshot.dialog_text.clone(),
            candidates: snapshot.buttons.clone(),
            elapsed_ms: elapsed.as_millis().min(u64::MAX as u128) as u64,
            attempts: self.attempts,
            last_error: None,
            cause,
        };
        self.last_snapshot = Some(snapshot);
        OemDialogDecision::Failed(failure)
    }
}

fn matching_confirm_buttons(
    snapshot: &OemDialogSnapshot,
    options: &OemInstallConfirmationOptions,
) -> Vec<usize> {
    let mut needles: Vec<String> = options
        .confirm_text
        .iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    if needles.is_empty() {
        needles = DEFAULT_OEM_CONFIRM_LABELS
            .iter()
            .map(|label| label.to_lowercase())
            .collect();
    }
    snapshot
        .buttons
        .iter()
        .enumerate()
        .filter(|(_, button)| {
            let label = button.label.to_lowercase();
            needles
                .iter()
                .any(|needle| label == needle.as_str() || label.contains(needle.as_str()))
        })
        .map(|(index, _)| index)
        .collect()
}

fn looks_like_oem_dialog(snapshot: &OemDialogSnapshot) -> bool {
    let lower = snapshot.dialog_text.to_lowercase();
    OEM_DIALOG_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
        || snapshot.buttons.iter().any(|button| {
            let label = button.label.to_lowercase();
            OEM_DIALOG_MARKERS
                .iter()
                .any(|marker| label.contains(marker))
        })
}

#[allow(dead_code)]
fn snapshot_from_hierarchy(hierarchy: &UiHierarchy) -> OemDialogSnapshot {
    let mut lines = Vec::new();
    let mut buttons = Vec::new();
    for node in &hierarchy.nodes {
        if let Some(text) = &node.text {
            lines.push(text.clone());
        }
        if let Some(description) = &node.content_desc {
            lines.push(description.clone());
        }
        let label = node.text.clone().or_else(|| node.content_desc.clone());
        if let Some(label) = label
            && node.visible()
            && (node.clickable || hierarchy.clickable_ancestor(node).is_some())
        {
            buttons.push(OemDialogButton {
                label,
                path: node.path.clone(),
                clickable: node.clickable,
            });
        }
    }
    OemDialogSnapshot {
        dialog_text: lines.join("\n"),
        buttons,
    }
}

fn install_error_text(error: &ForgeError) -> String {
    let mut text = error.message().to_string();
    for value in error.body().context.values() {
        if let Some(value) = value.as_str() {
            text.push('\n');
            text.push_str(value);
        }
    }
    text
}

/// True when a failed install looks like an OEM on-device confirmation prompt.
pub(crate) fn install_error_needs_oem_confirmation(error: &ForgeError) -> bool {
    if error.code() != ErrorCode::InstallFailed {
        return false;
    }
    let text = install_error_text(error).to_lowercase();
    OEM_CONFIRMATION_TRIGGERS
        .iter()
        .any(|marker| text.contains(marker))
}

/// Structured outcome attached to the install result or error context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct OemConfirmationResult {
    pub outcome: &'static str,
    pub reason: &'static str,
    pub label: Option<String>,
    pub tap_x: Option<i32>,
    pub tap_y: Option<i32>,
    pub elapsed_ms: u64,
    pub attempts: u32,
}

impl OemConfirmationResult {
    fn disabled() -> Self {
        Self {
            outcome: "disabled",
            reason: "confirmation_not_enabled",
            label: None,
            tap_x: None,
            tap_y: None,
            elapsed_ms: 0,
            attempts: 0,
        }
    }

    fn skipped(reason: &'static str) -> Self {
        Self {
            outcome: "skipped",
            reason,
            label: None,
            tap_x: None,
            tap_y: None,
            elapsed_ms: 0,
            attempts: 0,
        }
    }

    fn confirmed(label: String, x: i32, y: i32, elapsed: Duration, attempts: u32) -> Self {
        Self {
            outcome: "confirmed",
            reason: "confirmation_tapped",
            label: Some(label),
            tap_x: Some(x),
            tap_y: Some(y),
            elapsed_ms: elapsed.as_millis().min(u64::MAX as u128) as u64,
            attempts,
        }
    }
}

/// Watch an OEM confirmation dialog after a failed install and tap exactly one
/// unambiguous confirm button through the audited UI input path.
///
/// Call this from `install()` when `install_error_needs_oem_confirmation(&error)`
/// is true and the caller opted in with `--wait-for-confirmation`. On success
/// the caller should re-run the install once; on failure the returned
/// `ForgeError` already carries the dialog text, candidates, elapsed time and a
/// stable `reason` in its context.
pub(crate) async fn handle_oem_install_confirmation(
    ui: &UiManager,
    serial: &str,
    options: &OemInstallConfirmationOptions,
    install_error: &ForgeError,
) -> Result<OemConfirmationResult> {
    if !options.enabled {
        return Ok(OemConfirmationResult::disabled());
    }
    if !install_error_needs_oem_confirmation(install_error) {
        return Ok(OemConfirmationResult::skipped(
            "trigger_not_oem_confirmation",
        ));
    }
    let trigger = install_error_text(install_error);
    let started = Instant::now();
    let mut tracker = OemConfirmationTracker::new();
    loop {
        let elapsed = started.elapsed();
        let timed_out = elapsed >= options.timeout;
        match ui.dump(serial).await {
            Ok(hierarchy) => {
                let snapshot = snapshot_from_hierarchy(&hierarchy);
                match tracker.observe(snapshot.clone(), options, elapsed, timed_out) {
                    OemDialogDecision::Confirm(index) => {
                        let Some(button) = snapshot.buttons.get(index).cloned() else {
                            return Err(OemConfirmationFailure {
                                reason: "confirmation_button_stale",
                                dialog_text: snapshot.dialog_text,
                                candidates: snapshot.buttons,
                                elapsed_ms: elapsed.as_millis().min(u64::MAX as u128) as u64,
                                attempts: tracker.attempts,
                                last_error: None,
                                cause: None,
                            }
                            .into_error(&trigger, serial));
                        };
                        let Some(node) =
                            hierarchy.nodes.iter().find(|node| node.path == button.path)
                        else {
                            return Err(OemConfirmationFailure {
                                reason: "confirmation_button_stale",
                                dialog_text: snapshot.dialog_text,
                                candidates: snapshot.buttons,
                                elapsed_ms: elapsed.as_millis().min(u64::MAX as u128) as u64,
                                attempts: tracker.attempts,
                                last_error: None,
                                cause: None,
                            }
                            .into_error(&trigger, serial));
                        };
                        // `tap_expected` re-dumps the hierarchy, re-validates the
                        // node identity and dispatches through `adb input tap`.
                        // There is no coordinate or raw-shell bypass here.
                        let expected = NodeExpectation::from_node(node);
                        match ui.tap_expected(serial, &expected).await {
                            Ok(tap) => {
                                return Ok(OemConfirmationResult::confirmed(
                                    button.label,
                                    tap.tap_x,
                                    tap.tap_y,
                                    elapsed,
                                    tracker.attempts,
                                ));
                            }
                            Err(error) => {
                                return Err(OemConfirmationFailure {
                                    reason: "confirmation_tap_failed",
                                    dialog_text: snapshot.dialog_text,
                                    candidates: snapshot.buttons,
                                    elapsed_ms: elapsed.as_millis().min(u64::MAX as u128) as u64,
                                    attempts: tracker.attempts,
                                    last_error: None,
                                    cause: Some(error.message().to_string()),
                                }
                                .into_error(&trigger, serial));
                            }
                        }
                    }
                    OemDialogDecision::Failed(failure) => {
                        return Err(failure.into_error(&trigger, serial));
                    }
                    OemDialogDecision::Pending => {}
                }
            }
            Err(error) => {
                if timed_out {
                    return Err(OemConfirmationFailure {
                        reason: "ui_hierarchy_unavailable",
                        dialog_text: tracker
                            .last_snapshot()
                            .map(|snapshot| snapshot.dialog_text.clone())
                            .unwrap_or_default(),
                        candidates: tracker
                            .last_snapshot()
                            .map(|snapshot| snapshot.buttons.clone())
                            .unwrap_or_default(),
                        elapsed_ms: elapsed.as_millis().min(u64::MAX as u128) as u64,
                        attempts: tracker.attempts,
                        last_error: Some(error.message().to_string()),
                        cause: None,
                    }
                    .into_error(&trigger, serial));
                }
            }
        }
        tokio::time::sleep(options.poll_interval.max(Duration::from_millis(50))).await;
    }
}

async fn uninstall(ctx: &CliContext, args: &AppUninstallArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    if ctx.dry_run {
        let mut result = serde_json::json!({
            "dry_run": true,
            "serial": device.serial,
            "package": args.package,
            "keep_data": args.keep_data,
            "requires_confirmation": true,
        });
        if device.kind == androidforge_core::model::DeviceKind::Physical {
            result["requires_physical_mutation"] =
                serde_json::json!(!ctx.risk_allowed(Risk::Install));
        }
        return Ok(success(
            "app.uninstall",
            result,
            format!(
                "Would uninstall {}{} from {}; requires --yes{}\n",
                args.package,
                if args.keep_data { " (kept data)" } else { "" },
                device.serial,
                if device.kind == androidforge_core::model::DeviceKind::Physical
                    && !ctx.risk_allowed(Risk::Install)
                {
                    " and --allow-physical-install (or --allow-physical-mutation)"
                } else {
                    ""
                }
            ),
        )
        .with_optional_session(session_id(session.as_ref()))
        .with_device(device.serial.clone()));
    }
    if !ctx.yes {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            "uninstalling an app is destructive; pass --yes",
        )
        .with_suggestion("Use --keep-data when the app supports it, or back up data first.")
        .with_suggestion("Use --dry-run to preview without confirmation."));
    }
    ctx.ensure_risk_allowed(&device, Risk::Install, "uninstalling an app")?;
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
        let mut result = serde_json::json!({
            "dry_run": true,
            "serial": device.serial,
            "package": args.package,
            "requires_confirmation": true,
        });
        if device.kind == androidforge_core::model::DeviceKind::Physical {
            result["requires_physical_mutation"] = serde_json::json!(!ctx.risk_allowed(Risk::Data));
        }
        return Ok(success(
            "app.clear-data",
            result,
            format!(
                "Would clear {} data on {}; requires --yes when executed{}\n",
                args.package,
                device.serial,
                if device.kind == androidforge_core::model::DeviceKind::Physical
                    && !ctx.risk_allowed(Risk::Data)
                {
                    " and --allow-physical-data (or --allow-physical-mutation)"
                } else {
                    ""
                }
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
    ctx.ensure_risk_allowed(&device, Risk::Data, "clearing app data")?;
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

/// Device-independent description of one launch. `build --launch/--cold` and
/// `app launch` both funnel through `launch_request`. In particular, `--cold`
/// sets `force_stop` so the fixed `launch_with_force_stop` path is used and the
/// old bare force-stop path cannot silently regress.
#[derive(Debug, Clone)]
pub(crate) struct LaunchRequest {
    pub package: String,
    pub activity: Option<String>,
    pub component: Option<String>,
    pub action: Option<String>,
    pub data: Option<String>,
    pub mime_type: Option<String>,
    pub category: Vec<String>,
    pub extra: Vec<String>,
    pub flag: Vec<String>,
    pub wait: bool,
    pub force_stop: bool,
    /// Explicit cold-start request. Always routes through
    /// `AppManager::launch_with_force_stop`, even when an activity is named.
    pub cold: bool,
    pub no_wake: bool,
}

impl LaunchRequest {
    fn from_args(args: &AppLaunchArgs) -> Self {
        Self {
            package: args.package.clone(),
            activity: args.activity.clone(),
            component: args.component.clone(),
            action: args.action.clone(),
            data: args.data.clone(),
            mime_type: args.mime_type.clone(),
            category: args.category.clone(),
            extra: args.extra.clone(),
            flag: args.flag.clone(),
            wait: args.wait,
            force_stop: args.force_stop,
            cold: false,
            no_wake: args.no_wake,
        }
    }

    pub(crate) fn simple(package: String, activity: Option<String>, cold: bool) -> Self {
        Self {
            package,
            activity,
            component: None,
            action: None,
            data: None,
            mime_type: None,
            category: Vec::new(),
            extra: Vec::new(),
            flag: Vec::new(),
            wait: false,
            force_stop: cold,
            cold,
            no_wake: false,
        }
    }
}

/// Pure classification used by `launch_request` and by build wiring tests.
/// A cold request always stays on the bare-package `launch_with_force_stop`
/// path, even when an activity was supplied.
pub(crate) fn launch_uses_intent(args: &LaunchRequest) -> bool {
    if args.cold {
        return false;
    }
    args.component.is_some()
        || args.activity.is_some()
        || args.action.is_some()
        || args.data.is_some()
        || args.mime_type.is_some()
        || !args.category.is_empty()
        || !args.extra.is_empty()
        || !args.flag.is_empty()
}

async fn launch(ctx: &CliContext, args: &AppLaunchArgs) -> Result<CommandSuccess> {
    launch_request(ctx, &LaunchRequest::from_args(args)).await
}

pub(crate) async fn launch_request(
    ctx: &CliContext,
    args: &LaunchRequest,
) -> Result<CommandSuccess> {
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
    let spec = IntentSpec {
        package: Some(package.clone()),
        component: args.component.clone().or_else(|| {
            activity
                .as_ref()
                .map(|activity| component_for(&package, activity))
        }),
        action: args.action.clone(),
        data: args.data.clone(),
        mime_type: args.mime_type.clone(),
        categories: args.category.clone(),
        extras: parse_extras(&args.extra)?,
        flags: args.flag.clone(),
        receiver_permission: None,
        asynchronous: false,
        wait: args.wait,
        force_stop: args.force_stop || args.cold,
        wake,
    };
    // A cold request is a bare-package force-stop + verified new PID. It must
    // never be diverted into the intent path, even when an activity is named.
    let uses_intent = launch_uses_intent(args);
    ctx.ensure_risk_allowed(&device, Risk::AppControl, "launching an app or intent")?;
    execute_in_run!(
        ctx,
        if uses_intent {
            "app.intent"
        } else {
            "app.launch"
        },
        Some(if uses_intent {
            "app intent start"
        } else {
            "app launch"
        }),
        session.as_ref(),
        Some(&device),
        true,
        |_run| async {
            let app = ctx.app_manager()?;
            let result = if uses_intent {
                // Intent launches keep their existing in-spec force-stop handling.
                app.start_intent(&device.serial, &spec).await?
            } else if spec.force_stop {
                // A bare package previously ignored --force-stop because the
                // flag only reached IntentSpec. Verify a genuinely new PID.
                app.launch_with_force_stop(&device.serial, &package, activity.as_deref(), wake)
                    .await?
            } else {
                app.launch(&device.serial, &package, activity.as_deref(), wake)
                    .await?
            };
            let human = format!(
                "Launched {}{}{} foreground={}{}\n",
                package,
                result
                    .component
                    .as_ref()
                    .map(|component| format!(" ({component})"))
                    .unwrap_or_default(),
                result
                    .action
                    .as_ref()
                    .map(|action| format!(" action={action}"))
                    .unwrap_or_default(),
                result.foreground,
                result
                    .pid
                    .map(|pid| format!(" pid={pid}"))
                    .unwrap_or_default()
            );
            Ok(success(
                if uses_intent {
                    "app.intent"
                } else {
                    "app.launch"
                },
                serde_json::to_value(&result)?,
                human,
            )
            .with_optional_session(session_id(session.as_ref()))
            .with_device(device.serial.clone()))
        },
    )
    .await
}

fn component_for(package: &str, component: &str) -> String {
    if component.contains('/') {
        component.to_string()
    } else {
        format!("{package}/{component}")
    }
}

fn parse_extras(values: &[String]) -> Result<Vec<IntentExtra>> {
    values
        .iter()
        .map(|value| {
            let mut parts = value.splitn(3, ':');
            let key = parts.next().unwrap_or_default();
            let kind = parts.next().unwrap_or_default();
            let value = parts.next().unwrap_or_default();
            let kind = match kind.to_ascii_lowercase().as_str() {
                "s" | "string" => "es",
                "i" | "int" => "ei",
                "l" | "long" => "el",
                "f" | "float" => "ef",
                "z" | "bool" | "boolean" => "ez",
                "esa" | "string-array" => "esa",
                "eia" | "int-array" => "eia",
                other => {
                    return Err(ForgeError::invalid_argument(format!(
                        "unknown intent extra type '{other}' in {value:?}; use s, i, l, f, z, esa or eia"
                    )));
                }
            };
            if key.is_empty() {
                return Err(ForgeError::invalid_argument(format!(
                    "intent extra {value:?} needs key:type:value"
                )));
            }
            Ok(IntentExtra {
                kind: kind.to_string(),
                key: key.to_string(),
                value: value.to_string(),
            })
        })
        .collect()
}

fn broadcast_spec(args: &AppBroadcastArgs) -> Result<IntentSpec> {
    Ok(IntentSpec {
        package: args.package.clone(),
        component: None,
        action: args.action.clone(),
        data: args.data.clone(),
        mime_type: args.mime_type.clone(),
        categories: args.category.clone(),
        extras: parse_extras(&args.extra)?,
        flags: args.flag.clone(),
        receiver_permission: args.receiver_permission.clone(),
        asynchronous: args.asynchronous,
        wait: false,
        force_stop: false,
        wake: false,
    })
}

async fn broadcast(ctx: &CliContext, args: &AppBroadcastArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let spec = broadcast_spec(args)?;
    if ctx.dry_run {
        return Ok(plan(
            "app.broadcast",
            &device.serial,
            session.as_ref(),
            format!("Would broadcast {:?}\n", spec.action),
        ));
    }
    ctx.ensure_risk_allowed(&device, Risk::AppControl, "broadcasting an intent")?;
    execute_in_run!(
        ctx,
        "app.broadcast",
        Some("app broadcast"),
        session.as_ref(),
        Some(&device),
        true,
        |_run| async {
            let app = ctx.app_manager()?;
            let result: IntentOutput = app.broadcast_intent(&device.serial, &spec).await?;
            Ok(success(
                "app.broadcast",
                serde_json::to_value(&result)?,
                format!(
                    "Broadcast {:?}: {}\n",
                    result.action,
                    result.status.as_deref().unwrap_or("completed")
                ),
            )
            .with_optional_session(session_id(session.as_ref()))
            .with_device(device.serial.clone()))
        },
    )
    .await
}

async fn service(ctx: &CliContext, args: &AppServiceArgs) -> Result<CommandSuccess> {
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let component = args
        .component
        .clone()
        .or_else(|| {
            args.service
                .as_ref()
                .zip(args.package.as_ref())
                .map(|(service, package)| component_for(package, service))
        })
        .or_else(|| args.service.clone());
    let spec = IntentSpec {
        package: args.package.clone(),
        component,
        action: args.action.clone(),
        data: args.data.clone(),
        mime_type: None,
        categories: Vec::new(),
        extras: parse_extras(&args.extra)?,
        flags: Vec::new(),
        receiver_permission: None,
        asynchronous: false,
        wait: false,
        force_stop: false,
        wake: false,
    };
    if ctx.dry_run {
        return Ok(plan(
            "app.service",
            &device.serial,
            session.as_ref(),
            format!(
                "Would {} service {:?}\n",
                if args.stop { "stop" } else { "start" },
                spec.component
            ),
        ));
    }
    ctx.ensure_risk_allowed(&device, Risk::AppControl, "starting or stopping a service")?;
    execute_in_run!(
        ctx,
        "app.service",
        Some(if args.stop {
            "app service stop"
        } else {
            "app service start"
        }),
        session.as_ref(),
        Some(&device),
        true,
        |_run| async {
            let app = ctx.app_manager()?;
            let result = app.service_intent(&device.serial, &spec, args.stop).await?;
            Ok(success(
                "app.service",
                serde_json::to_value(&result)?,
                format!(
                    "{} service {:?}: {}\n",
                    if args.stop { "Stopped" } else { "Started" },
                    result.component,
                    result.status.as_deref().unwrap_or("requested")
                ),
            )
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
    ctx.ensure_risk_allowed(&device, Risk::AppControl, "force-stopping an app")?;
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
    ctx.ensure_risk_allowed(&device, Risk::AppControl, "killing an app process")?;
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
        || expectation.is_some()
        || args.expect_version_name.is_some()
        || args.expect_version_code.is_some()
        || args.expect_apk_sha256.is_some();
    let live_expectations = args.expect_version_name.is_some()
        || args.expect_version_code.is_some()
        || args.expect_apk_sha256.is_some();
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
            let mut install_expectations = Vec::new();
            let mut live_install = None;
            if live_expectations {
                let live = inspect_installed(
                    &ctx.adb()?,
                    &ctx.inspector(),
                    &device.serial,
                    &package,
                )
                .await?;
                let Some(live) = live else {
                    return Err(ForgeError::new(
                        ErrorCode::PackageNotFound,
                        format!("package {package} is not installed on {}", device.serial),
                    )
                    .with_suggestion("Install the application before asserting installed build identity.")
                    .device(device.serial.clone()));
                };
                let mut failures = Vec::new();
                if let Some(expected) = &args.expect_version_name {
                    let actual = live.version_name.clone();
                    let passed = actual.as_deref() == Some(expected.as_str());
                    install_expectations.push(serde_json::json!({
                        "name": "version_name",
                        "expected": expected,
                        "actual": actual,
                        "passed": passed,
                    }));
                    if !passed {
                        failures.push("version_name");
                    }
                }
                if let Some(expected) = args.expect_version_code {
                    let actual = live.version_code;
                    let passed = actual == Some(expected);
                    install_expectations.push(serde_json::json!({
                        "name": "version_code",
                        "expected": expected,
                        "actual": actual,
                        "passed": passed,
                    }));
                    if !passed {
                        failures.push("version_code");
                    }
                }
                if let Some(expected) = &args.expect_apk_sha256 {
                    let actual = live.base_apk.sha256.clone();
                    let passed = actual.eq_ignore_ascii_case(expected.trim());
                    install_expectations.push(serde_json::json!({
                        "name": "apk_sha256",
                        "expected": expected.trim(),
                        "actual": actual,
                        "passed": passed,
                    }));
                    if !passed {
                        failures.push("apk_sha256");
                    }
                }
                live_install = Some(serde_json::json!({
                    "package": live.package,
                    "version_name": live.version_name,
                    "version_code": live.version_code,
                    "base_apk_path": live.base_remote_path,
                    "base_apk_sha256": live.base_apk.sha256,
                }));
                if !failures.is_empty() {
                    return Err(ForgeError::new(
                        ErrorCode::InstallVerificationFailed,
                        format!(
                            "installed build assertion failed for {package}: {}",
                            failures.join(", ")
                        ),
                    )
                    .with_context("expectations", install_expectations)
                    .with_context("live_install", live_install)
                    .with_suggestion("Run `androidforge app verify-installed --apk <apk>` for a full byte comparison.")
                    .device(device.serial.clone()));
                }
            }
            Ok(success(
                "app.status",
                serde_json::json!({
                    "expectation": expect_value,
                    "states": states,
                    "install_expectations": install_expectations,
                    "live_install": live_install,
                }),
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
    let artifact = crate::commands::apk::load_artifact_with_splits(ctx, apk, &[]).await?;
    let info = artifact.base.clone();
    let mut value = serde_json::to_value(&info)?;
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "path".to_string(),
            serde_json::Value::String(artifact.source.to_string_lossy().into_owned()),
        );
        object.insert(
            "artifact".to_string(),
            serde_json::to_value(artifact.summary())?,
        );
    }
    let mut human = format!(
        "{}\npackage: {}\nversion: {} ({})\nminSdk: {:?} targetSdk: {:?}\nsha256: {}\nsigned: {:?} verified: {:?}\ncert SHA-256: {}\ndebug signed: {:?}\nartifact: {}\nsplits: {}\n",
        artifact.source.display(),
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
        artifact.kind.as_str(),
        artifact.split_count(),
    );
    if let Some(archive_sha256) = &artifact.archive_sha256 {
        human.push_str(&format!("archive SHA-256: {archive_sha256}\n"));
    }
    for warning in &artifact.warnings {
        human.push_str(&format!("warning: {warning}\n"));
    }
    Ok(success("apk.inspect", value, human))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const STAMP: &str = "20260102T030405Z";

    fn test_paths(cwd: &Path, project_root: &Path, artifacts_dir: &Path) -> ForgePaths {
        ForgePaths {
            cwd: cwd.to_path_buf(),
            project_root: project_root.to_path_buf(),
            project_config_path: None,
            global_config_path: PathBuf::new(),
            global_dir: PathBuf::new(),
            state_dir: PathBuf::new(),
            artifacts_dir: artifacts_dir.to_path_buf(),
        }
    }

    fn failure_reason(error: &ForgeError) -> Option<&str> {
        error
            .body()
            .context
            .get("reason")
            .and_then(|value| value.as_str())
    }

    #[test]
    fn export_default_lives_in_artifacts_export_area_outside_project() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let artifacts = temp.path().join("managed-artifacts");
        std::fs::create_dir_all(&project).unwrap();
        let paths = test_paths(temp.path(), &project, &artifacts);
        let destination = resolve_export_destination(
            &paths,
            "com.example.app",
            STAMP,
            &ExportDestinationPolicy::default(),
        )
        .unwrap();
        assert_eq!(
            destination.path,
            artifacts
                .join("export")
                .join("com.example.app")
                .join(format!("com.example.app-{STAMP}.tar"))
        );
        assert_eq!(destination.source, ExportDestinationSource::Default);
        assert!(destination.in_export_area);
        assert!(!destination.in_project);
    }

    #[test]
    fn export_default_inside_project_artifacts_dir_is_allowed() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let artifacts = project.join(".artifacts");
        std::fs::create_dir_all(&artifacts).unwrap();
        let paths = test_paths(temp.path(), &project, &artifacts);
        let destination = resolve_export_destination(
            &paths,
            "com.example.app",
            STAMP,
            &ExportDestinationPolicy::default(),
        )
        .unwrap();
        assert_eq!(destination.source, ExportDestinationSource::Default);
        assert!(destination.in_project);
        assert!(destination.in_export_area);
    }

    #[test]
    fn export_explicit_absolute_path_is_honoured() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let artifacts = temp.path().join("managed-artifacts");
        let output = temp.path().join("outside.tar");
        std::fs::create_dir_all(&project).unwrap();
        let paths = test_paths(temp.path(), &project, &artifacts);
        let destination = resolve_export_destination(
            &paths,
            "com.example.app",
            STAMP,
            &ExportDestinationPolicy::explicit(&output),
        )
        .unwrap();
        assert_eq!(destination.path, output);
        assert_eq!(destination.source, ExportDestinationSource::Explicit);
        assert!(!destination.in_project);
        assert!(!destination.in_export_area);
    }

    #[test]
    fn export_relative_path_resolves_against_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let artifacts = temp.path().join("managed-artifacts");
        let cwd = temp.path().join("work");
        std::fs::create_dir_all(&cwd).unwrap();
        let paths = test_paths(&cwd, &project, &artifacts);
        let destination = resolve_export_destination(
            &paths,
            "com.example.app",
            STAMP,
            &ExportDestinationPolicy::explicit("exports/./backup.tar"),
        )
        .unwrap();
        assert_eq!(destination.path, cwd.join("exports").join("backup.tar"));
        assert_eq!(destination.source, ExportDestinationSource::Explicit);
    }

    #[test]
    fn export_in_project_path_is_refused_without_override() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let artifacts = temp.path().join("managed-artifacts");
        let inside = project.join("backup.tar");
        std::fs::create_dir_all(&project).unwrap();
        let paths = test_paths(temp.path(), &project, &artifacts);
        let error = resolve_export_destination(
            &paths,
            "com.example.app",
            STAMP,
            &ExportDestinationPolicy::explicit(&inside),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
        assert_eq!(failure_reason(&error), Some("destination_inside_project"));
        assert_eq!(
            error
                .body()
                .context
                .get("in_project")
                .and_then(|value| value.as_bool()),
            Some(true)
        );

        let allowed = ExportDestinationPolicy {
            allow_in_project: true,
            ..ExportDestinationPolicy::explicit(&inside)
        };
        let destination =
            resolve_export_destination(&paths, "com.example.app", STAMP, &allowed).unwrap();
        assert_eq!(destination.path, inside);
        assert!(destination.in_project);
    }

    #[test]
    fn export_explicit_path_inside_default_export_area_still_needs_override() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let artifacts = project.join(".artifacts");
        let inside = artifacts.join("export").join("chosen.tar");
        std::fs::create_dir_all(inside.parent().unwrap()).unwrap();
        let paths = test_paths(temp.path(), &project, &artifacts);
        let error = resolve_export_destination(
            &paths,
            "com.example.app",
            STAMP,
            &ExportDestinationPolicy::explicit(&inside),
        )
        .unwrap_err();
        assert_eq!(failure_reason(&error), Some("destination_inside_project"));
    }

    #[test]
    fn export_rejects_parent_dir_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let artifacts = temp.path().join("managed-artifacts");
        let paths = test_paths(temp.path(), &project, &artifacts);
        for candidate in ["../outside.tar", "backup/../../outside.tar"] {
            let error = resolve_export_destination(
                &paths,
                "com.example.app",
                STAMP,
                &ExportDestinationPolicy::explicit(candidate),
            )
            .unwrap_err();
            assert_eq!(
                failure_reason(&error),
                Some("destination_traversal"),
                "{candidate}"
            );
        }
        let absolute_escape = temp.path().join("work").join("..").join("outside.tar");
        let error = resolve_export_destination(
            &paths,
            "com.example.app",
            STAMP,
            &ExportDestinationPolicy::explicit(&absolute_escape),
        )
        .unwrap_err();
        assert_eq!(failure_reason(&error), Some("destination_traversal"));
    }

    #[test]
    fn export_existing_file_is_refused_until_overwrite_is_enabled() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let artifacts = temp.path().join("managed-artifacts");
        let output = temp.path().join("outside.tar");
        std::fs::write(&output, b"old").unwrap();
        let paths = test_paths(temp.path(), &project, &artifacts);
        let error = resolve_export_destination(
            &paths,
            "com.example.app",
            STAMP,
            &ExportDestinationPolicy::explicit(&output),
        )
        .unwrap_err();
        assert_eq!(failure_reason(&error), Some("destination_exists"));
        assert_eq!(
            error
                .body()
                .context
                .get("path")
                .and_then(|value| value.as_str()),
            Some(output.to_string_lossy().as_ref())
        );
        let replace = ExportDestinationPolicy {
            overwrite: ExportOverwritePolicy::Replace,
            ..ExportDestinationPolicy::explicit(&output)
        };
        let destination =
            resolve_export_destination(&paths, "com.example.app", STAMP, &replace).unwrap();
        assert_eq!(destination.path, output);
    }

    #[test]
    fn export_existing_default_file_is_refused_until_overwrite_is_enabled() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let artifacts = temp.path().join("managed-artifacts");
        let expected = artifacts
            .join("export")
            .join("com.example.app")
            .join(format!("com.example.app-{STAMP}.tar"));
        std::fs::create_dir_all(expected.parent().unwrap()).unwrap();
        std::fs::write(&expected, b"old").unwrap();
        let paths = test_paths(temp.path(), &project, &artifacts);
        let error = resolve_export_destination(
            &paths,
            "com.example.app",
            STAMP,
            &ExportDestinationPolicy::default(),
        )
        .unwrap_err();
        assert_eq!(failure_reason(&error), Some("destination_exists"));
        let replace = ExportDestinationPolicy {
            overwrite: ExportOverwritePolicy::Replace,
            ..ExportDestinationPolicy::default()
        };
        assert_eq!(
            resolve_export_destination(&paths, "com.example.app", STAMP, &replace)
                .unwrap()
                .path,
            expected
        );
    }

    #[test]
    fn export_destination_directory_is_always_refused() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let artifacts = temp.path().join("managed-artifacts");
        let directory = temp.path().join("backup.tar");
        std::fs::create_dir_all(&directory).unwrap();
        let paths = test_paths(temp.path(), &project, &artifacts);
        let replace = ExportDestinationPolicy {
            overwrite: ExportOverwritePolicy::Replace,
            ..ExportDestinationPolicy::explicit(&directory)
        };
        let error =
            resolve_export_destination(&paths, "com.example.app", STAMP, &replace).unwrap_err();
        assert_eq!(failure_reason(&error), Some("destination_is_directory"));

        let bad_package = resolve_export_destination(
            &paths,
            "../escape",
            STAMP,
            &ExportDestinationPolicy::default(),
        )
        .unwrap_err();
        assert_eq!(
            failure_reason(&bad_package),
            Some("destination_name_invalid")
        );
    }

    #[test]
    fn parses_typed_intent_extras() {
        let extras = parse_extras(&[
            "name:s:Ada".to_string(),
            "count:i:7".to_string(),
            "ratio:f:1.5".to_string(),
            "enabled:z:true".to_string(),
            "labels:esa:a,b".to_string(),
        ])
        .unwrap();
        assert_eq!(extras[0].kind, "es");
        assert_eq!(extras[0].key, "name");
        assert_eq!(extras[0].value, "Ada");
        assert_eq!(extras[1].kind, "ei");
        assert_eq!(extras[2].kind, "ef");
        assert_eq!(extras[3].kind, "ez");
        assert_eq!(extras[4].kind, "esa");
    }

    #[test]
    fn intent_extra_keeps_colons_inside_the_value() {
        let extras = parse_extras(&["url:s:https://example.test/a:b".to_string()]).unwrap();
        assert_eq!(extras[0].value, "https://example.test/a:b");
    }

    #[test]
    fn intent_extra_rejects_unknown_types_and_missing_values() {
        let error = parse_extras(&["key:q:value".to_string()]).unwrap_err();
        assert!(error.message().contains("unknown intent extra type"));
        let error = parse_extras(&[":s:value".to_string()]).unwrap_err();
        assert!(error.message().contains("key:type:value"));
    }

    #[test]
    fn component_is_expanded_only_when_needed() {
        assert_eq!(
            component_for("com.example", ".MainActivity"),
            "com.example/.MainActivity"
        );
        assert_eq!(
            component_for("com.example", "com.example/.Other"),
            "com.example/.Other"
        );
    }

    #[test]
    fn broadcast_spec_carries_receiver_options() {
        let spec = broadcast_spec(&AppBroadcastArgs {
            package: Some("com.example".to_string()),
            action: Some("com.example.PING".to_string()),
            data: None,
            mime_type: None,
            category: vec!["com.example.CATEGORY".to_string()],
            extra: vec!["ping:s:1".to_string()],
            flag: vec!["0x10000000".to_string()],
            receiver_permission: Some("com.example.PERMISSION".to_string()),
            asynchronous: true,
            timeout: None,
        })
        .unwrap();
        assert_eq!(spec.action.as_deref(), Some("com.example.PING"));
        assert!(spec.asynchronous);
        assert_eq!(
            spec.receiver_permission.as_deref(),
            Some("com.example.PERMISSION")
        );
        assert_eq!(spec.extras.len(), 1);
        assert_eq!(spec.categories.len(), 1);
        assert_eq!(spec.flags.len(), 1);
    }

    fn dialog(text: &str, buttons: &[(&str, &str)]) -> OemDialogSnapshot {
        OemDialogSnapshot {
            dialog_text: text.to_string(),
            buttons: buttons
                .iter()
                .map(|(label, path)| OemDialogButton {
                    label: (*label).to_string(),
                    path: (*path).to_string(),
                    clickable: true,
                })
                .collect(),
        }
    }

    fn confirmation_options(confirm_text: &[&str]) -> OemInstallConfirmationOptions {
        OemInstallConfirmationOptions {
            enabled: true,
            confirm_text: confirm_text
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            timeout: Duration::from_secs(2),
            poll_interval: Duration::from_millis(100),
        }
    }

    fn failed(decision: OemDialogDecision) -> OemConfirmationFailure {
        match decision {
            OemDialogDecision::Failed(failure) => failure,
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[test]
    fn oem_confirmation_taps_a_single_matching_button() {
        let mut tracker = OemConfirmationTracker::new();
        let options = confirmation_options(&[]);
        let decision = tracker.observe(
            dialog(
                "Install app?\nThis app will be installed.",
                &[("Cancel", "0.1"), ("Install", "0.2")],
            ),
            &options,
            Duration::from_millis(250),
            false,
        );
        let index = match decision {
            OemDialogDecision::Confirm(index) => index,
            other => panic!("expected confirm, got {other:?}"),
        };
        assert_eq!(index, 1);
    }

    #[test]
    fn oem_confirmation_detects_auto_denial() {
        let mut tracker = OemConfirmationTracker::new();
        let options = confirmation_options(&[]);
        assert_eq!(
            tracker.observe(
                dialog("Install app? Checking permissions...", &[]),
                &options,
                Duration::from_millis(120),
                false,
            ),
            OemDialogDecision::Pending
        );
        let failure = failed(tracker.observe(
            dialog(
                "Installation denied by device administrator",
                &[("OK", "1.0")],
            ),
            &options,
            Duration::from_millis(360),
            false,
        ));
        assert_eq!(failure.reason, "confirmation_auto_denied");
        assert_eq!(
            failure.dialog_text,
            "Installation denied by device administrator"
        );
        assert_eq!(failure.candidates.len(), 1);
        assert_eq!(failure.attempts, 2);
        let error = failure.into_error("Failure [INSTALL_FAILED_USER_RESTRICTED]", "emulator-5554");
        assert_eq!(error.code(), ErrorCode::InstallFailed);
        assert_eq!(
            error
                .body()
                .context
                .get("reason")
                .and_then(|value| value.as_str()),
            Some("confirmation_auto_denied")
        );
        assert_eq!(
            error
                .body()
                .context
                .get("elapsed_ms")
                .and_then(|value| value.as_u64()),
            Some(360)
        );
    }

    #[test]
    fn oem_confirmation_times_out_without_a_dialog() {
        let mut tracker = OemConfirmationTracker::new();
        let options = confirmation_options(&[]);
        assert_eq!(
            tracker.observe(dialog("", &[]), &options, Duration::from_millis(900), false),
            OemDialogDecision::Pending
        );
        let failure =
            failed(tracker.observe(dialog("", &[]), &options, Duration::from_millis(2000), true));
        assert_eq!(failure.reason, "confirmation_dialog_timeout");
        assert_eq!(failure.elapsed_ms, 2000);
        assert_eq!(failure.attempts, 2);
        assert!(failure.candidates.is_empty());
    }

    #[test]
    fn oem_confirmation_reports_missing_button_with_dialog_text() {
        let mut tracker = OemConfirmationTracker::new();
        let options = confirmation_options(&[]);
        let failure = failed(tracker.observe(
            dialog(
                "Install app? Unknown source is disabled",
                &[("Remind me later", "0.3"), ("Settings", "0.4")],
            ),
            &options,
            Duration::from_millis(2000),
            true,
        ));
        assert_eq!(failure.reason, "confirm_button_not_found");
        assert_eq!(
            failure.dialog_text,
            "Install app? Unknown source is disabled"
        );
        assert_eq!(failure.candidates.len(), 2);
        let error = failure.into_error("INSTALL_FAILED_USER_RESTRICTED", "serial-1");
        assert_eq!(
            error
                .body()
                .context
                .get("reason")
                .and_then(|value| value.as_str()),
            Some("confirm_button_not_found")
        );
        assert!(error.body().context.contains_key("candidates"));
        assert_eq!(error.body().device_serial.as_deref(), Some("serial-1"));
    }

    #[test]
    fn oem_confirmation_rejects_multiple_candidates() {
        let mut tracker = OemConfirmationTracker::new();
        let options = confirmation_options(&[]);
        let failure = failed(tracker.observe(
            dialog(
                "Install app?",
                &[
                    ("Install", "0.1"),
                    ("Install anyway", "0.2"),
                    ("Cancel", "0.3"),
                ],
            ),
            &options,
            Duration::from_millis(300),
            false,
        ));
        assert_eq!(failure.reason, "multiple_confirm_buttons");
        assert_eq!(failure.candidates.len(), 3);
        let error = failure.into_error("install failed", "serial-2");
        assert_eq!(error.code(), ErrorCode::InstallFailed);
        assert_eq!(
            error
                .body()
                .context
                .get("reason")
                .and_then(|value| value.as_str()),
            Some("multiple_confirm_buttons")
        );
    }

    #[test]
    fn oem_confirmation_matches_localized_and_uppercase_labels() {
        let mut tracker = OemConfirmationTracker::new();
        let options = confirmation_options(&[]);
        let decision = tracker.observe(
            dialog("安装应用？请确认", &[("取消", "0.1"), ("允许", "0.2")]),
            &options,
            Duration::from_millis(150),
            false,
        );
        let index = match decision {
            OemDialogDecision::Confirm(index) => index,
            other => panic!("expected localized confirm, got {other:?}"),
        };
        assert_eq!(index, 1);

        let mut tracker = OemConfirmationTracker::new();
        let decision = tracker.observe(
            dialog("Install?", &[("ALLOW INSTALLATION", "1.1")]),
            &options,
            Duration::from_millis(150),
            false,
        );
        assert_eq!(decision, OemDialogDecision::Confirm(0));
    }

    #[test]
    fn oem_confirmation_builds_snapshot_from_synthetic_hierarchy() {
        let xml = r#"<hierarchy rotation="0">
  <node index="0" text="Install app?" resource-id="" class="android.widget.TextView" package="com.android.packageinstaller" content-desc="" clickable="false" enabled="true" focused="false" scrollable="false" bounds="[0,0][1080,200]" />
  <node index="1" text="Cancel" resource-id="com.android.packageinstaller:id/cancel" class="android.widget.Button" package="com.android.packageinstaller" content-desc="" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[100,400][400,500]" />
  <node index="2" text="允许安装" resource-id="com.android.packageinstaller:id/ok" class="android.widget.Button" package="com.android.packageinstaller" content-desc="" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[500,400][900,500]" />
</hierarchy>"#;
        let hierarchy = UiHierarchy::parse(xml).unwrap();
        let snapshot = snapshot_from_hierarchy(&hierarchy);
        assert!(snapshot.dialog_text.contains("Install app?"));
        assert_eq!(snapshot.buttons.len(), 2);
        assert_eq!(snapshot.buttons[1].label, "允许安装");
        assert_eq!(snapshot.buttons[1].path, "2");
        let mut tracker = OemConfirmationTracker::new();
        let options = confirmation_options(&[]);
        let decision = tracker.observe(snapshot, &options, Duration::from_millis(100), false);
        assert_eq!(decision, OemDialogDecision::Confirm(1));
    }

    #[test]
    fn explicit_confirm_text_ignores_default_labels() {
        let mut tracker = OemConfirmationTracker::new();
        let options = confirmation_options(&["Continue"]);
        let decision = tracker.observe(
            dialog("Install app?", &[("Install", "0.1"), ("Continue", "0.2")]),
            &options,
            Duration::from_millis(150),
            false,
        );
        assert_eq!(decision, OemDialogDecision::Confirm(1));
    }

    #[test]
    fn only_install_failures_with_oem_markers_engage_the_handler() {
        let error = ForgeError::new(
            ErrorCode::InstallFailed,
            "installation of com.example failed: INSTALL_FAILED_USER_RESTRICTED",
        )
        .with_context_string("install_output", "Failure [INSTALL_FAILED_USER_RESTRICTED]");
        assert!(install_error_needs_oem_confirmation(&error));
        let other = ForgeError::new(ErrorCode::InstallFailed, "INSTALL_FAILED_VERSION_DOWNGRADE");
        assert!(!install_error_needs_oem_confirmation(&other));
        let wrong_code = ForgeError::new(
            ErrorCode::CommandFailed,
            "INSTALL_FAILED_USER_RESTRICTED from an unrelated command",
        );
        assert!(!install_error_needs_oem_confirmation(&wrong_code));
    }

    // The driver below is exercised against a fake ADB script, not a device.
    // It proves that the audited UI path is used: dump -> state machine ->
    // identity-validated tap through UiManager -> structured outcome/error.
    fn oem_ui_adb(dir: &Path, xml: &str) -> PathBuf {
        let xml_path = dir.join("oem-ui.xml");
        std::fs::write(&xml_path, xml).expect("fake OEM dialog XML");
        #[cfg(windows)]
        {
            let path = dir.join("fake-adb.cmd");
            std::fs::write(
                &path,
                "@echo off\r\n\
                 setlocal enabledelayedexpansion\r\n\
                 set \"all=%*\"\r\n\
                 echo !all! | findstr /I \"uiautomator\" >nul\r\n\
                 if !errorlevel! equ 0 (\r\n\
                 echo OK\r\n\
                 exit /b 0\r\n\
                 )\r\n\
                 echo !all! | findstr /I \"exec-out\" >nul\r\n\
                 if !errorlevel! equ 0 (\r\n\
                 type \"%~dp0oem-ui.xml\"\r\n\
                 exit /b 0\r\n\
                 )\r\n\
                 echo !all! | findstr /I \"wm size\" >nul\r\n\
                 if !errorlevel! equ 0 (\r\n\
                 echo Physical display size: 1080x2400\r\n\
                 exit /b 0\r\n\
                 )\r\n\
                 echo !all! | findstr /I \"input tap\" >nul\r\n\
                 if !errorlevel! equ 0 (\r\n\
                 echo OK\r\n\
                 exit /b 0\r\n\
                 )\r\n\
                 exit /b 0\r\n",
            )
            .expect("fake adb");
            path
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join("fake-adb");
            std::fs::write(
                &path,
                "#!/bin/sh\n\
                 case \"$*\" in\n\
                   *uiautomator*) echo OK ;;\n\
                   *exec-out*) cat \"$(dirname \"$0\")/oem-ui.xml\" ;;\n\
                   *wm\\ size*) echo 'Physical display size: 1080x2400' ;;\n\
                   *input\\ tap*) echo OK ;;\n\
                 esac\n\
                 exit 0\n",
            )
            .expect("fake adb");
            let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).expect("permissions");
            path
        }
    }

    fn oem_ui_manager(adb: &Path) -> UiManager {
        UiManager::new(
            androidforge_adb::AdbClient::new(adb.to_path_buf(), false, Duration::from_secs(5)),
            Duration::from_secs(2),
            Duration::from_millis(50),
            Duration::from_millis(50),
            true,
            false,
        )
    }

    fn oem_install_error() -> ForgeError {
        ForgeError::new(
            ErrorCode::InstallFailed,
            "installation of com.example failed: INSTALL_FAILED_USER_RESTRICTED",
        )
        .with_context_string("install_output", "Failure [INSTALL_FAILED_USER_RESTRICTED]")
    }

    #[tokio::test]
    async fn oem_confirmation_driver_taps_the_single_confirm_button_with_fake_adb() {
        let temp = tempfile::tempdir().expect("temp");
        let xml = r#"<hierarchy rotation="0">
  <node index="0" text="Install app?" class="android.widget.TextView" clickable="false" enabled="true" focused="false" scrollable="false" bounds="[0,0][1080,200]" />
  <node index="1" text="Cancel" class="android.widget.Button" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[100,400][400,500]" />
  <node index="2" text="Install" class="android.widget.Button" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[500,400][900,500]" />
</hierarchy>"#;
        let adb = oem_ui_adb(temp.path(), xml);
        let ui = oem_ui_manager(&adb);
        let options = OemInstallConfirmationOptions {
            enabled: true,
            confirm_text: Vec::new(),
            timeout: Duration::from_secs(2),
            poll_interval: Duration::from_millis(50),
        };
        let result =
            handle_oem_install_confirmation(&ui, "emulator-5554", &options, &oem_install_error())
                .await
                .expect("confirmation");
        assert_eq!(result.outcome, "confirmed");
        assert_eq!(result.label.as_deref(), Some("Install"));
        assert!(result.tap_x.is_some() && result.tap_y.is_some());
        assert_eq!(result.attempts, 1);
    }

    #[tokio::test]
    async fn oem_confirmation_driver_reports_auto_denial_with_exact_dialog_text() {
        let temp = tempfile::tempdir().expect("temp");
        let xml = r#"<hierarchy rotation="0">
  <node index="0" text="Install app?" class="android.widget.TextView" clickable="false" enabled="true" focused="false" scrollable="false" bounds="[0,0][1080,200]" />
  <node index="1" text="Installation denied by device administrator" class="android.widget.TextView" clickable="false" enabled="true" focused="false" scrollable="false" bounds="[0,200][1080,400]" />
</hierarchy>"#;
        let adb = oem_ui_adb(temp.path(), xml);
        let ui = oem_ui_manager(&adb);
        let options = OemInstallConfirmationOptions {
            enabled: true,
            confirm_text: Vec::new(),
            timeout: Duration::from_secs(1),
            poll_interval: Duration::from_millis(50),
        };
        let error =
            handle_oem_install_confirmation(&ui, "emulator-5554", &options, &oem_install_error())
                .await
                .expect_err("auto-denial");
        assert_eq!(error.code(), ErrorCode::InstallFailed);
        assert_eq!(
            error.body().context.get("reason").and_then(Value::as_str),
            Some("confirmation_auto_denied")
        );
        assert_eq!(
            error
                .body()
                .context
                .get("dialog_text")
                .and_then(Value::as_str),
            Some("Install app?\nInstallation denied by device administrator")
        );
    }

    #[tokio::test]
    async fn oem_confirmation_driver_times_out_without_a_dialog() {
        let temp = tempfile::tempdir().expect("temp");
        let xml = r#"<hierarchy rotation="0">
  <node index="0" text="Waiting for device" class="android.widget.TextView" clickable="false" enabled="true" focused="false" scrollable="false" bounds="[0,0][1080,200]" />
</hierarchy>"#;
        let adb = oem_ui_adb(temp.path(), xml);
        let ui = oem_ui_manager(&adb);
        let options = OemInstallConfirmationOptions {
            enabled: true,
            confirm_text: Vec::new(),
            timeout: Duration::ZERO,
            poll_interval: Duration::from_millis(50),
        };
        let error =
            handle_oem_install_confirmation(&ui, "emulator-5554", &options, &oem_install_error())
                .await
                .expect_err("timeout");
        assert_eq!(
            error.body().context.get("reason").and_then(Value::as_str),
            Some("confirmation_dialog_timeout")
        );
        assert_eq!(
            error
                .body()
                .context
                .get("elapsed_ms")
                .and_then(Value::as_u64),
            Some(0)
        );
    }
}
