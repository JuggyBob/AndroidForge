use std::path::Path;
use std::time::Duration;

use androidforge_core::config::{
    ForgeConfig, ForgeConfigFile, Severity, default_global_toml, default_project_toml,
};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::paths::ForgePaths;
use androidforge_core::toolchain::Toolchain;
use androidforge_core::util::{is_dir_writable, now_rfc3339};
use androidforge_process::{CommandSpec, Runner};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cli::{Cli, ConfigCommand, InitArgs, SchemaCommand};
use crate::context::CliContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Check {
    name: String,
    status: String,
    detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DoctorReport {
    schema_version: u32,
    status: String,
    generated_at: String,
    host: Value,
    project_root: String,
    project_config: Option<String>,
    global_config: Option<String>,
    checks: Vec<Check>,
    tools: Vec<androidforge_core::model::ToolInfo>,
    avds: Vec<String>,
    sessions: Vec<androidforge_core::session::SessionListEntry>,
}

pub async fn doctor(cli: &Cli) -> Result<CommandSuccess> {
    let paths = ForgePaths::discover(None, cli.project.clone())?;
    let config_result = ForgeConfig::load(&paths);
    let (config, config_error) = match config_result {
        Ok(mut config) => {
            let adjusted = paths.with_artifact_dir(&config.artifact_dir);
            config.paths = adjusted;
            (config, None)
        }
        Err(error) => {
            let paths = paths.with_artifact_dir(&paths.project_root.join(".artifacts"));
            (
                ForgeConfig {
                    artifact_dir: paths.artifacts_dir.clone(),
                    paths,
                    file: ForgeConfigFile::default(),
                    warnings: Vec::new(),
                },
                Some(error),
            )
        }
    };
    let toolchain = Toolchain::discover(&config);
    let mut checks = Vec::new();
    let mut tools = Vec::new();
    checks.push(ok(
        "host",
        format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    ));
    checks.push(ok(
        "androidforge",
        format!("{} (report schema 1)", env!("CARGO_PKG_VERSION")),
    ));
    if let Some(error) = config_error {
        checks.push(error_check(
            "project_config",
            error.message().to_string(),
            None,
        ));
    } else if paths.project_config_path.is_none() {
        checks.push(Check {
            name: "project_config".to_string(),
            status: "warn".to_string(),
            detail: "no project adapter found; using built-in defaults".to_string(),
            path: None,
        });
    } else {
        let validation = config.validate();
        let errors = validation
            .issues
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
            .count();
        let warnings = validation.issues.len().saturating_sub(errors);
        checks.push(Check {
            name: "project_config".to_string(),
            status: if errors > 0 {
                "error".to_string()
            } else if warnings > 0 {
                "warn".to_string()
            } else {
                "ok".to_string()
            },
            detail: format!(
                "{} ({} errors, {} warnings)",
                config.project_name(),
                errors,
                warnings
            ),
            path: paths
                .project_config_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        });
    }
    checks.push(Check {
        name: "sdk".to_string(),
        status: if toolchain.sdk_root.is_some() {
            "ok".to_string()
        } else {
            "error".to_string()
        },
        detail: toolchain
            .sdk_root
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "ANDROID_HOME / ANDROID_SDK_ROOT not found".to_string()),
        path: toolchain
            .sdk_root
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
    });
    for (name, path, version_arg) in [
        ("adb", toolchain.adb.as_ref(), Some("version")),
        ("emulator", toolchain.emulator.as_ref(), Some("-version")),
        ("aapt2", toolchain.aapt2.as_ref(), Some("version")),
        ("java", toolchain.java.as_ref(), Some("-version")),
        ("maestro", toolchain.maestro.as_ref(), Some("--version")),
    ] {
        let info = inspect_tool(name, path, version_arg, cli.trace).await;
        checks.push(Check {
            name: name.to_string(),
            status: if info.available { "ok" } else { "warn" }.to_string(),
            detail: info
                .version
                .clone()
                .or_else(|| info.path.clone())
                .unwrap_or_else(|| "not found".to_string()),
            path: info.path.clone(),
        });
        tools.push(info);
    }
    if let Some(apksigner) = &toolchain.apksigner {
        tools.push(androidforge_core::model::ToolInfo::available(
            "apksigner",
            apksigner.to_string_lossy().into_owned(),
            None,
        ));
    } else {
        tools.push(androidforge_core::model::ToolInfo::missing(
            "apksigner",
            "not found; APK certificate verification is unavailable",
        ));
    }
    checks.push(Check {
        name: "sdk_licenses".to_string(),
        status: if toolchain
            .licenses_dir()
            .map(|dir| dir.is_dir())
            .unwrap_or(false)
        {
            "ok".to_string()
        } else {
            "warn".to_string()
        },
        detail: toolchain
            .licenses_dir()
            .map(|dir| dir.display().to_string())
            .unwrap_or_else(|| "licenses directory unavailable".to_string()),
        path: toolchain
            .licenses_dir()
            .map(|dir| dir.to_string_lossy().into_owned()),
    });
    checks.push(Check {
        name: "artifacts".to_string(),
        status: if is_dir_writable(&config.artifact_dir) {
            "ok".to_string()
        } else {
            "error".to_string()
        },
        detail: config.artifact_dir.display().to_string(),
        path: Some(config.artifact_dir.to_string_lossy().into_owned()),
    });
    let mut avds = Vec::new();
    if toolchain.emulator.is_some() {
        match androidforge_emulator::list_avds(&toolchain, cli.trace).await {
            Ok(list) => {
                avds = list.into_iter().map(|avd| avd.name).collect();
                checks.push(Check {
                    name: "avds".to_string(),
                    status: if avds.is_empty() { "warn" } else { "ok" }.to_string(),
                    detail: format!("{} AVD(s)", avds.len()),
                    path: toolchain
                        .avd_home
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned()),
                });
            }
            Err(error) => checks.push(error_check("avds", error.message().to_string(), None)),
        }
    } else {
        checks.push(Check {
            name: "avds".to_string(),
            status: "warn".to_string(),
            detail: "emulator executable unavailable; AVDs cannot be listed".to_string(),
            path: None,
        });
    }
    if let Some(emulator) = &toolchain.emulator {
        let spec = CommandSpec::new(emulator.to_string_lossy().into_owned())
            .args(["-accel-check"])
            .timeout(Duration::from_secs(20))
            .trace(cli.trace)
            .hide_window(true);
        let accel = Runner::new(cli.trace).run(spec).await;
        let detail = match accel {
            Ok(output) => output.combined_text(),
            Err(error) => error.to_string(),
        };
        let lowered = detail.to_ascii_lowercase();
        checks.push(Check {
            name: "virtualization".to_string(),
            status: if lowered.contains("installed and usable")
                || lowered.contains("accel:") && lowered.contains("\n0")
            {
                "ok".to_string()
            } else {
                "warn".to_string()
            },
            detail: detail.trim().chars().take(2000).collect(),
            path: None,
        });
    }
    let sessions = androidforge_core::session::list_sessions(&paths).unwrap_or_default();
    checks.push(Check {
        name: "sessions".to_string(),
        status: if sessions.iter().any(|session| session.stale) {
            "warn".to_string()
        } else {
            "ok".to_string()
        },
        detail: format!(
            "{} session(s), {} stale",
            sessions.len(),
            sessions.iter().filter(|session| session.stale).count()
        ),
        path: None,
    });
    let writable = is_dir_writable(&config.artifact_dir);
    let free = androidforge_core::util::available_space(&config.artifact_dir);
    let (disk_status, disk_detail) = match free {
        Ok(bytes) if writable && bytes >= 1024 * 1024 * 1024 => (
            "ok".to_string(),
            format!(
                "{} free ({})",
                androidforge_core::util::format_bytes(bytes),
                config.artifact_dir.display()
            ),
        ),
        Ok(bytes) if writable => (
            "warn".to_string(),
            format!(
                "only {} free in {}",
                androidforge_core::util::format_bytes(bytes),
                config.artifact_dir.display()
            ),
        ),
        Ok(_) => (
            "warn".to_string(),
            "artifact directory is not writable".to_string(),
        ),
        Err(error) => (
            "warn".to_string(),
            format!("free-space measurement failed: {error}"),
        ),
    };
    checks.push(Check {
        name: "disk".to_string(),
        status: disk_status,
        detail: disk_detail,
        path: Some(config.artifact_dir.to_string_lossy().into_owned()),
    });
    let status = if checks.iter().any(|check| check.status == "error") {
        "error"
    } else if checks.iter().any(|check| check.status == "warn") {
        "degraded"
    } else {
        "ok"
    };
    let report = DoctorReport {
        schema_version: 1,
        status: status.to_string(),
        generated_at: now_rfc3339(),
        host: serde_json::json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "family": std::env::consts::FAMILY,
        }),
        project_root: paths.project_root.to_string_lossy().into_owned(),
        project_config: paths
            .project_config_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        global_config: if paths.global_config_path.is_file() {
            Some(paths.global_config_path.to_string_lossy().into_owned())
        } else {
            None
        },
        checks: checks.clone(),
        tools,
        avds,
        sessions,
    };
    let mut human = String::new();
    human.push_str(&format!(
        "AndroidForge doctor: {} ({})\n",
        report.status, report.project_root
    ));
    for check in &report.checks {
        human.push_str(&format!(
            "  [{}] {} - {}\n",
            check.status, check.name, check.detail
        ));
    }
    human.push_str(&format!(
        "\nCHECKS: {}  TOOLS: {}  AVDS: {}\n",
        report.checks.len(),
        report.tools.len(),
        report.avds.len()
    ));
    let result = serde_json::to_value(&report)?;
    let mut success = CommandSuccess::new("doctor", result, human);
    if status == "error" {
        success.exit_code = 3;
    }
    Ok(success)
}

async fn inspect_tool(
    name: &str,
    path: Option<&std::path::PathBuf>,
    version_arg: Option<&str>,
    trace: bool,
) -> androidforge_core::model::ToolInfo {
    let Some(path) = path else {
        return androidforge_core::model::ToolInfo::missing(name, "not found");
    };
    let version = if let Some(arg) = version_arg {
        let spec = CommandSpec::new(path.to_string_lossy().into_owned())
            .args([arg])
            .timeout(Duration::from_secs(15))
            .trace(trace)
            .hide_window(true);
        Runner::new(trace).run(spec).await.ok().map(|output| {
            output
                .combined_text()
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("unknown")
                .trim()
                .to_string()
        })
    } else {
        None
    };
    androidforge_core::model::ToolInfo::available(
        name,
        path.to_string_lossy().into_owned(),
        version,
    )
}

fn ok(name: &str, detail: impl Into<String>) -> Check {
    Check {
        name: name.to_string(),
        status: "ok".to_string(),
        detail: detail.into(),
        path: None,
    }
}

fn error_check(name: &str, detail: impl Into<String>, path: Option<String>) -> Check {
    Check {
        name: name.to_string(),
        status: "error".to_string(),
        detail: detail.into(),
        path,
    }
}

#[derive(Debug, Serialize)]
struct Capability {
    name: String,
    status: String,
    requires: Vec<String>,
    notes: String,
}

pub async fn capabilities(cli: &Cli) -> Result<CommandSuccess> {
    let paths = ForgePaths::discover(None, cli.project.clone())?;
    let config = ForgeConfig::load(&paths).unwrap_or_else(|_| {
        let paths = paths.with_artifact_dir(&paths.project_root.join(".artifacts"));
        ForgeConfig {
            artifact_dir: paths.artifacts_dir.clone(),
            paths,
            file: ForgeConfigFile::default(),
            warnings: Vec::new(),
        }
    });
    let toolchain = Toolchain::discover(&config);
    let has_adb = toolchain.adb.is_some();
    let has_emulator = toolchain.emulator.is_some();
    let has_aapt2 = toolchain.aapt2.is_some();
    let has_apksigner = toolchain.apksigner.is_some();
    let has_maestro = toolchain.maestro.is_some();
    let mut items = Vec::new();
    let mut add = |name: &str, status: &str, requires: &[&str], notes: &str| {
        items.push(Capability {
            name: name.to_string(),
            status: status.to_string(),
            requires: requires.iter().map(|value| value.to_string()).collect(),
            notes: notes.to_string(),
        });
    };
    add(
        "devices.list",
        if has_adb { "supported" } else { "unsupported" },
        &["adb"],
        "Parses adb devices -l and enriches operational devices.",
    );
    add(
        "emulator.lifecycle",
        if has_emulator && has_adb {
            "supported"
        } else {
            "unsupported"
        },
        &["adb", "emulator"],
        "Legacy emulator executable; headless start, boot wait, stop and snapshots.",
    );
    add(
        "session.ownership",
        "supported",
        &["filesystem locks"],
        "Coordinates AndroidForge sessions only; external ADB users are not prevented from touching a device.",
    );
    add(
        "apk.inspect",
        if has_aapt2 {
            "supported"
        } else {
            "unsupported"
        },
        &["aapt2"],
        "Local APK metadata inspection.",
    );
    add(
        "apk.verify",
        if has_aapt2 && has_apksigner {
            "supported"
        } else {
            "unsupported"
        },
        &["aapt2", "apksigner", "java for jar"],
        "Local signature verification; no keystore or signing automation.",
    );
    add(
        "app.install",
        if has_adb { "supported" } else { "unsupported" },
        &["adb"],
        "Signature-aware install diagnostics; never uninstalls automatically.",
    );
    add(
        "ui.uiautomator",
        if has_adb { "supported" } else { "unsupported" },
        &["adb", "uiautomator"],
        "Compose semantics are visible only when exposed through Android accessibility.",
    );
    add(
        "capture.screenshot",
        if has_adb { "supported" } else { "unsupported" },
        &["adb"],
        "Binary PNG via adb exec-out; no console redirection.",
    );
    add(
        "capture.recording",
        if has_adb {
            "best-effort"
        } else {
            "unsupported"
        },
        &["adb", "screenrecord"],
        "Android's standard screenrecord limit is 180s; no audio.",
    );
    add(
        "diagnostics.logcat",
        if has_adb { "supported" } else { "unsupported" },
        &["adb"],
        "Never clears the global buffer by default.",
    );
    add(
        "diagnostics.jobs",
        if has_adb {
            "best-effort"
        } else {
            "unsupported"
        },
        &["adb", "dumpsys jobscheduler"],
        "Parsed output varies; raw evidence is preserved and job IDs are never hardcoded.",
    );
    add(
        "diagnostics.notifications",
        if has_adb {
            "best-effort"
        } else {
            "unsupported"
        },
        &["adb", "dumpsys notification"],
        "NotificationManagerService formatting varies by Android version.",
    );
    add(
        "diagnostics.doze.time",
        if has_adb {
            "best-effort"
        } else {
            "unsupported"
        },
        &["adb", "device privileges"],
        "Time changes usually require root; AndroidForge never roots automatically.",
    );
    add(
        "fixtures",
        "supported",
        &["project adapter"],
        "Explicit project hooks and fixture scripts; never executed automatically.",
    );
    add(
        "maestro",
        if has_maestro {
            "supported"
        } else {
            "unsupported"
        },
        &["maestro optional"],
        "Optional local Maestro CLI; no Maestro Cloud, accounts or API keys.",
    );
    add(
        "instrumentation",
        if has_adb { "supported" } else { "unsupported" },
        &["adb"],
        "Runs installed instrumentation; AndroidForge does not replace Gradle.",
    );
    add(
        "snapshots",
        if has_emulator {
            "best-effort"
        } else {
            "unsupported"
        },
        &["adb", "emulator"],
        "Verified at runtime; loading always requires --yes.",
    );
    add(
        "avd.cloning",
        "unsupported",
        &["none"],
        "Not implemented; do not pretend a renamed profile provides isolation.",
    );
    add(
        "html.reports",
        "unsupported",
        &["none"],
        "JSON and raw artifacts are the primary evidence format.",
    );
    add(
        "visual.diff",
        "unsupported",
        &["none"],
        "Screenshot evidence is supported; automated visual comparison is deferred.",
    );
    add(
        "cloud",
        "unsupported",
        &["none"],
        "AndroidForge is local-first and has no telemetry or cloud upload.",
    );
    let result = serde_json::json!({
        "schema_version": 1,
        "host": {"os": std::env::consts::OS, "arch": std::env::consts::ARCH},
        "toolchain": {
            "sdk": toolchain.sdk_root,
            "adb": toolchain.adb,
            "emulator": toolchain.emulator,
            "aapt2": toolchain.aapt2,
            "apksigner": toolchain.apksigner,
            "java": toolchain.java,
            "maestro": toolchain.maestro,
        },
        "capabilities": items,
        "runtime_verified": false,
        "note": "Capabilities are implementation/toolchain reports, not proof that a real device exercised every feature in this environment."
    });
    let mut human = String::from("AndroidForge capabilities\n");
    for item in &items {
        human.push_str(&format!("  [{:<13}] {}\n", item.status, item.name));
    }
    Ok(CommandSuccess::new("capabilities", result, human))
}

pub fn schema(command: &SchemaCommand) -> Result<CommandSuccess> {
    let schemas = schema_files();
    match command {
        SchemaCommand::List => {
            let names = schemas
                .iter()
                .map(|(name, _)| name.to_string())
                .collect::<Vec<_>>();
            Ok(CommandSuccess::new(
                "schema.list",
                serde_json::json!({ "schemas": names }),
                format!("Available schemas:\n{}\n", names.join("\n")),
            ))
        }
        SchemaCommand::Show { name } => {
            let Some((_, text)) = schemas.iter().find(|(candidate, _)| candidate == name) else {
                return Err(ForgeError::new(
                    ErrorCode::InvalidArgument,
                    format!("schema '{name}' was not found"),
                )
                .with_suggestion(format!(
                    "Available schemas: {}",
                    schemas
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            };
            let value: Value = serde_json::from_str(text).map_err(|error| {
                ForgeError::internal(format!("embedded schema {name} is invalid JSON: {error}"))
            })?;
            Ok(CommandSuccess::new(
                "schema.show",
                value.clone(),
                format!("{}\n", serde_json::to_string_pretty(&value)?),
            )
            .with_artifacts(Vec::new()))
        }
    }
}

fn schema_files() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "androidforge.schema.json",
            include_str!("../../../../schema/androidforge.schema.json"),
        ),
        (
            "adapter.schema.json",
            include_str!("../../../../schema/adapter.schema.json"),
        ),
        (
            "fixture.schema.json",
            include_str!("../../../../schema/fixture.schema.json"),
        ),
        (
            "hook.schema.json",
            include_str!("../../../../schema/hook.schema.json"),
        ),
        (
            "conformance.schema.json",
            include_str!("../../../../schema/conformance.schema.json"),
        ),
    ]
}

pub fn init(cli: &Cli, args: &InitArgs) -> Result<CommandSuccess> {
    let paths = ForgePaths::discover(None, cli.project.clone())?;
    if args.global {
        let path = paths.global_config_path.clone();
        if path.exists() && !args.force {
            return Err(ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!("{} already exists", path.display()),
            )
            .with_suggestion("Pass --force to overwrite the global configuration."));
        }
        if !cli.dry_run {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    ForgeError::new(
                        ErrorCode::ArtifactWriteFailed,
                        format!("could not create {}: {error}", parent.display()),
                    )
                })?;
            }
            std::fs::write(
                &path,
                default_global_toml(
                    std::env::var("ANDROID_HOME")
                        .ok()
                        .map(std::path::PathBuf::from)
                        .as_deref(),
                ),
            )
            .map_err(|error| {
                ForgeError::new(
                    ErrorCode::ArtifactWriteFailed,
                    format!("could not write {}: {error}", path.display()),
                )
            })?;
        }
        let human = format!(
            "{} global AndroidForge configuration at {}\n",
            if cli.dry_run {
                "Would create"
            } else {
                "Created"
            },
            path.display()
        );
        return Ok(CommandSuccess::new(
            "init",
            serde_json::json!({"global_config": path, "dry_run": cli.dry_run}),
            human,
        ));
    }
    if paths.project_config_path.is_some() && !args.force {
        return Err(ForgeError::new(
            ErrorCode::ConfigInvalid,
            "this project already has .androidforge/project.toml",
        )
        .with_suggestion("Pass --force to overwrite it.")
        .with_suggestion("Inspect the current adapter with `androidforge config show`."));
    }
    let name = args.name.clone().unwrap_or_else(|| {
        paths
            .project_root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Android project".to_string())
    });
    let content = default_project_toml(&name, args.package.as_deref(), args.avd.as_deref());
    let config_path = paths
        .project_root
        .join(".androidforge")
        .join("project.toml");
    if !cli.dry_run {
        std::fs::create_dir_all(paths.project_root.join(".androidforge").join("fixtures"))
            .map_err(|error| {
                ForgeError::new(
                    ErrorCode::ArtifactWriteFailed,
                    format!("could not create .androidforge: {error}"),
                )
            })?;
        std::fs::create_dir_all(paths.project_root.join(".androidforge").join("conformance"))
            .map_err(|error| {
                ForgeError::new(
                    ErrorCode::ArtifactWriteFailed,
                    format!("could not create conformance directory: {error}"),
                )
            })?;
        std::fs::write(&config_path, content).map_err(|error| {
            ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!("could not write {}: {error}", config_path.display()),
            )
        })?;
    }
    let human = format!(
        "{} AndroidForge adapter at {}\n\nRecommended .gitignore:\n  .artifacts/\n  .androidforge/current-session\n\nThe next commands:\n  androidforge doctor\n  androidforge session start --avd <AVD>\n  androidforge config validate\n",
        if cli.dry_run {
            "Would create"
        } else {
            "Created"
        },
        config_path.display()
    );
    Ok(CommandSuccess::new(
        "init",
        serde_json::json!({
            "project_root": paths.project_root,
            "project_config": config_path,
            "dry_run": cli.dry_run,
            "gitignore": [".artifacts/", ".androidforge/current-session"],
        }),
        human,
    ))
}

pub async fn config(cli: &Cli, command: &ConfigCommand) -> Result<CommandSuccess> {
    match command {
        ConfigCommand::Path => {
            let paths = ForgePaths::discover(None, cli.project.clone())?;
            let result = serde_json::json!({
                "project_root": paths.project_root,
                "project_config": paths.project_config_path,
                "global_config": paths.global_config_path,
                "state_dir": paths.state_dir,
                "default_artifacts_dir": paths.artifacts_dir,
            });
            let human = format!(
                "Project root: {}\nProject config: {}\nGlobal config: {}\nState: {}\n",
                paths.project_root.display(),
                paths
                    .project_config_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "(none)".to_string()),
                paths.global_config_path.display(),
                paths.state_dir.display()
            );
            Ok(CommandSuccess::new("config.path", result, human))
        }
        ConfigCommand::Show => {
            let ctx = CliContext::new(cli)?;
            let result = serde_json::to_value(&ctx.config.file)?;
            let human = format!(
                "{} ({})\n{}\n",
                ctx.config.project_name(),
                ctx.config.paths.project_root.display(),
                serde_json::to_string_pretty(&result)?
            );
            Ok(CommandSuccess::new("config.show", result, human))
        }
        ConfigCommand::Validate => {
            let ctx = CliContext::new(cli)?;
            let report = ctx.config.validate();
            let valid = report.valid;
            let result = serde_json::to_value(&report)?;
            let mut human = format!(
                "Configuration {} for {}\n",
                if valid { "is valid" } else { "has errors" },
                report.project_root
            );
            for issue in &report.issues {
                human.push_str(&format!(
                    "  [{}] {}: {}\n",
                    if issue.severity == Severity::Error {
                        "error"
                    } else {
                        "warning"
                    },
                    issue.field,
                    issue.message
                ));
            }
            let mut success = CommandSuccess::new("config.validate", result, human);
            if !valid {
                success.exit_code = 8;
            }
            Ok(success)
        }
    }
}

#[allow(dead_code)]
fn path_hint(path: Option<&Path>) -> Option<String> {
    path.map(|path| path.to_string_lossy().into_owned())
}
