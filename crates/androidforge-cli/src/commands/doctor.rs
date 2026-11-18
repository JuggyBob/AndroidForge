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

use crate::cli::{Cli, ConfigCommand, DoctorFixArgs, InitArgs, SchemaCommand};
use crate::commands::common::success;
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
struct CheckView {
    name: String,
    status: String,
    detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    required: bool,
    blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    impact: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    suggestion: Option<String>,
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
    checks: Vec<CheckView>,
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
    let active_sessions = sessions
        .iter()
        .filter(|session| {
            !matches!(
                session.state,
                androidforge_core::session::SessionState::Released
                    | androidforge_core::session::SessionState::Failed
                    | androidforge_core::session::SessionState::Abandoned
            )
        })
        .collect::<Vec<_>>();
    let stale_active = active_sessions
        .iter()
        .filter(|session| session.stale)
        .count();
    let tombstones = sessions.len().saturating_sub(active_sessions.len());
    checks.push(Check {
        name: "sessions".to_string(),
        status: if stale_active > 0 {
            "warn".to_string()
        } else {
            "ok".to_string()
        },
        detail: format!(
            "{} active session(s), {} stale active, {} tombstone(s)",
            active_sessions.len(),
            stale_active,
            tombstones
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
    let check_views = checks.iter().map(enrich_check).collect::<Vec<_>>();
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
        checks: check_views,
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

fn enrich_check(check: &Check) -> CheckView {
    let (required, blocking, impact, suggestion) = match check.name.as_str() {
        "host" | "androidforge" => (
            true,
            true,
            Some("The host and AndroidForge version anchor every report."),
            None,
        ),
        "sdk" => (
            true,
            true,
            Some("The SDK is required to find ADB, the emulator and build tools."),
            Some("Set ANDROID_HOME or ANDROID_SDK_ROOT, or install the Android SDK."),
        ),
        "adb" => (
            true,
            true,
            Some("ADB is the transport for every device operation."),
            Some("Install Android SDK platform-tools and put adb.exe on PATH."),
        ),
        "emulator" => (
            true,
            false,
            Some(
                "Emulator execution is required to start AVD sessions; attached devices still work.",
            ),
            Some("Install the Android Emulator component from the SDK manager."),
        ),
        "aapt2" => (
            true,
            false,
            Some("APK inspection and package/version identity require aapt2."),
            Some("Install Android SDK build-tools and select a version with aapt2."),
        ),
        "java" => (
            true,
            false,
            Some("apksigner needs a JDK to verify APK signatures and certificates."),
            Some("Install a JDK 17+ or point JAVA_HOME at the SDK-bundled JDK."),
        ),
        "avds" => (
            false,
            false,
            Some("At least one AVD is needed to start an AndroidForge-owned emulator."),
            Some("Create an AVD in Android Studio or with avdmanager, then recheck."),
        ),
        "project_config" => (
            false,
            false,
            Some("A project adapter supplies package, AVD, fixtures and conformance defaults."),
            Some("Run `androidforge init` in the project or select an existing adapter."),
        ),
        "sdk_licenses" => (
            false,
            false,
            Some("Unaccepted licenses can block SDK component installation and updates."),
            Some("Run `sdkmanager --licenses` and accept the required licenses."),
        ),
        "virtualization" => (
            false,
            false,
            Some("Hardware acceleration is required for usable emulator performance."),
            Some("Enable virtualization in firmware/Windows features and recheck."),
        ),
        "maestro" => (
            false,
            false,
            Some("Maestro is optional; AndroidForge automation does not require it."),
            Some("Install Maestro only if you want to run Maestro flows."),
        ),
        "artifacts" => (
            true,
            true,
            Some(
                "Reports, screenshots and manifests cannot be written without a writable artifact directory.",
            ),
            Some("Choose a writable project/artifact directory or fix its permissions."),
        ),
        "sessions" => (
            false,
            false,
            Some("Stale sessions can block a device and hide the current owner."),
            Some("Run `androidforge session gc --reclaim --dry-run` to preview cleanup."),
        ),
        "disk" => (
            false,
            false,
            Some("Low disk space can interrupt evidence capture and report writes."),
            Some("Free space or choose a different artifact directory before long runs."),
        ),
        _ => (false, false, None, None),
    };
    CheckView {
        name: check.name.clone(),
        status: check.status.clone(),
        detail: check.detail.clone(),
        path: check.path.clone(),
        required,
        blocking: blocking && check.status != "ok",
        impact: impact.map(str::to_string),
        suggestion: suggestion.map(str::to_string),
    }
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

#[derive(Debug, Clone, Serialize)]
struct RepairAction {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<Vec<String>>,
    requires_confirmation: bool,
    requires_admin: bool,
    requires_network: bool,
    modifies_environment: bool,
}

#[derive(Debug, Clone, Serialize)]
struct RepairIssue {
    id: String,
    severity: String,
    description: String,
    detected: String,
    component: String,
    reason: String,
    repair: RepairAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
}

fn repair(
    kind: &str,
    package: Option<&str>,
    command: Option<Vec<String>>,
    requires_network: bool,
    modifies_environment: bool,
) -> RepairAction {
    RepairAction {
        kind: kind.to_string(),
        package: package.map(str::to_string),
        command,
        requires_confirmation: modifies_environment || requires_network,
        requires_admin: false,
        requires_network,
        modifies_environment,
    }
}

#[allow(clippy::too_many_arguments)]
fn repair_issue(
    id: &str,
    severity: &str,
    description: impl Into<String>,
    detected: impl Into<String>,
    component: &str,
    reason: impl Into<String>,
    repair: RepairAction,
    notes: Option<String>,
) -> RepairIssue {
    RepairIssue {
        id: id.to_string(),
        severity: severity.to_string(),
        description: description.into(),
        detected: detected.into(),
        component: component.to_string(),
        reason: reason.into(),
        repair,
        notes,
    }
}

fn sdkmanager_command(sdk_root: Option<&Path>, packages: &[&str]) -> Vec<String> {
    let mut argv = vec!["sdkmanager".to_string()];
    if let Some(root) = sdk_root {
        argv.push(format!("--sdk_root={}", root.display()));
    }
    argv.extend(packages.iter().map(|package| (*package).to_string()));
    argv
}

fn env_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os(name)
        .map(std::path::PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// Produce a read-only repair plan. This function never installs packages,
/// never writes configuration and never changes environment variables.
pub async fn fix_plan(cli: &Cli, args: &DoctorFixArgs) -> Result<CommandSuccess> {
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
    let mut issues: Vec<RepairIssue> = Vec::new();

    if let Some(error) = &config_error {
        issues.push(repair_issue(
            "invalid_project_config",
            "blocking",
            "The project configuration could not be loaded.",
            error.message().to_string(),
            "project configuration",
            "AndroidForge cannot apply project defaults until the configuration parses.",
            repair(
                "configuration",
                None,
                Some(vec![
                    "androidforge".to_string(),
                    "config".to_string(),
                    "validate".to_string(),
                    "--project".to_string(),
                    paths.project_root.to_string_lossy().into_owned(),
                    "--json".to_string(),
                ]),
                false,
                false,
            ),
            Some(
                "Fix the reported TOML field, then re-run `androidforge doctor fix --plan`."
                    .to_string(),
            ),
        ));
    } else {
        let validation = config.validate();
        for issue in &validation.issues {
            issues.push(repair_issue(
                "invalid_project_config",
                if issue.severity == Severity::Error {
                    "blocking"
                } else {
                    "recommended"
                },
                format!("Project configuration field '{}' is invalid.", issue.field),
                issue.message.clone(),
                "project configuration",
                "Invalid project configuration produces incorrect or unsafe defaults.",
                repair(
                    "configuration",
                    None,
                    Some(vec![
                        "androidforge".to_string(),
                        "config".to_string(),
                        "validate".to_string(),
                        "--project".to_string(),
                        paths.project_root.to_string_lossy().into_owned(),
                        "--json".to_string(),
                    ]),
                    false,
                    false,
                ),
                None,
            ));
        }
        if paths.project_config_path.is_none() {
            issues.push(repair_issue(
                "missing_project_adapter",
                "optional",
                "No project adapter was found; built-in defaults are in use.",
                paths.project_root.display().to_string(),
                "project configuration",
                "A project adapter supplies the package, AVD, fixtures and timeouts used by agent workflows.",
                repair(
                    "configuration",
                    None,
                    Some(vec![
                        "androidforge".to_string(),
                        "init".to_string(),
                        "--project".to_string(),
                        paths.project_root.to_string_lossy().into_owned(),
                        "--json".to_string(),
                    ]),
                    false,
                    true,
                ),
                Some("This writes .androidforge/project.toml and requires explicit confirmation.".to_string()),
            ));
        }
    }

    if let Some(configured) = &config.file.sdk.root
        && !configured.is_dir()
    {
        issues.push(repair_issue(
            "invalid_sdk_path",
            "blocking",
            "The configured Android SDK path does not exist.",
            configured.display().to_string(),
            "Android SDK",
            "Every ADB, emulator and APK operation depends on a valid SDK root.",
            repair("environment", None, None, false, true),
            Some("Correct sdk.root in .androidforge/project.toml, or remove it and set ANDROID_HOME.".to_string()),
        ));
    }

    let android_home = env_path("ANDROID_HOME");
    let sdk_root_env = env_path("ANDROID_SDK_ROOT");
    for (name, value) in [
        ("ANDROID_HOME", android_home.as_ref()),
        ("ANDROID_SDK_ROOT", sdk_root_env.as_ref()),
    ] {
        if let Some(value) = value
            && !value.is_dir()
        {
            issues.push(repair_issue(
                "invalid_environment_path",
                if toolchain.sdk_root.is_some() {
                    "recommended"
                } else {
                    "blocking"
                },
                format!("{name} points to a path that does not exist."),
                value.display().to_string(),
                "environment",
                "A stale SDK environment variable hides the real SDK or breaks tool discovery.",
                repair("environment", None, None, false, true),
                Some(format!(
                    "Remove {name} or point it at the Android SDK directory, then open a new shell."
                )),
            ));
        }
    }
    if let (Some(home), Some(root)) = (&android_home, &sdk_root_env)
        && home.is_dir()
        && root.is_dir()
        && home != root
    {
        issues.push(repair_issue(
            "conflicting_sdk_environment",
            "recommended",
            "ANDROID_HOME and ANDROID_SDK_ROOT point to different directories.",
            format!("{} vs {}", home.display(), root.display()),
            "environment",
            "Different tools can resolve different SDK installations, producing confusing failures.",
            repair("environment", None, None, false, true),
            Some("Keep one SDK location and set both variables to that same path, or remove one.".to_string()),
        ));
    }

    let sdk_root = toolchain.sdk_root.clone();
    if sdk_root.is_none() {
        issues.push(repair_issue(
            "missing_sdk",
            "blocking",
            "No Android SDK was found.",
            "ANDROID_HOME, ANDROID_SDK_ROOT, LOCALAPPDATA\\Android\\Sdk and sdk.root were not valid",
            "Android SDK",
            "ADB, emulators, APK inspection and installation require the Android SDK.",
            repair("manual", None, None, true, true),
            Some("Install Android Studio or the Android command-line tools, then set ANDROID_HOME and reopen the terminal.".to_string()),
        ));
    }

    if toolchain.adb.is_none() {
        issues.push(repair_issue(
            "missing_platform_tools",
            "blocking",
            "ADB was not found.",
            "no adb executable on PATH or under the discovered SDK",
            "platform-tools",
            "Device and emulator control requires ADB.",
            repair(
                "sdk_package",
                Some("platform-tools"),
                Some(sdkmanager_command(sdk_root.as_deref(), &["platform-tools"])),
                true,
                true,
            ),
            Some(
                "If sdkmanager is not on PATH, install the Android command-line tools first."
                    .to_string(),
            ),
        ));
    }
    if toolchain.emulator.is_none() {
        issues.push(repair_issue(
            "missing_emulator",
            "recommended",
            "The Android emulator binary was not found.",
            "no emulator executable under the discovered SDK",
            "emulator",
            "Emulator sessions and AVD management require the emulator package.",
            repair(
                "sdk_package",
                Some("emulator"),
                Some(sdkmanager_command(sdk_root.as_deref(), &["emulator"])),
                true,
                true,
            ),
            None,
        ));
    }
    if toolchain.aapt2.is_none() || toolchain.apksigner.is_none() {
        issues.push(repair_issue(
            "missing_build_tools",
            "recommended",
            "Android SDK Build-Tools components (aapt2/apksigner) were not found.",
            format!(
                "aapt2={}, apksigner={}",
                toolchain
                    .aapt2
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "missing".to_string()),
                toolchain
                    .apksigner
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "missing".to_string())
            ),
            "Android SDK Build-Tools",
            "APK metadata inspection and certificate verification depend on Build-Tools.",
            repair(
                "sdk_package",
                None,
                Some(sdkmanager_command(sdk_root.as_deref(), &["--list"])),
                true,
                true,
            ),
            Some("Install a current Build-Tools package with the Android Studio SDK Manager. The `--list` command only discovers available versions; choose the version your project requires.".to_string()),
        ));
    }
    if toolchain.java.is_none() {
        issues.push(repair_issue(
            "missing_java",
            "recommended",
            "A Java runtime was not found.",
            "no java executable on PATH or in the SDK",
            "JDK",
            "Some signing, instrumentation and legacy tooling paths require Java.",
            repair("manual", None, None, true, true),
            Some(
                "Install a current JDK (17 or newer for modern Android tooling) and set JAVA_HOME."
                    .to_string(),
            ),
        ));
    }
    if config.file.maestro.enabled && toolchain.maestro.is_none() {
        issues.push(repair_issue(
            "missing_maestro",
            "optional",
            "Maestro was not found; optional Maestro flows cannot run.",
            "no maestro executable on PATH",
            "Maestro",
            "Only Maestro-based flows need it; other AndroidForge features work without it.",
            repair("manual", None, None, true, true),
            Some("Install Maestro separately and ensure it is on PATH, or disable the maestro integration in configuration.".to_string()),
        ));
    }
    if let Some(sdk) = &sdk_root {
        let licenses = sdk.join("licenses");
        if !licenses.is_dir() {
            issues.push(repair_issue(
                "missing_sdk_licenses",
                "recommended",
                "The SDK licenses directory was not found.",
                licenses.display().to_string(),
                "SDK licenses",
                "SDK package installation may refuse to continue until licenses are accepted.",
                repair(
                    "sdk_command",
                    None,
                    Some(sdkmanager_command(Some(sdk), &["--licenses"])),
                    false,
                    true,
                ),
                Some("Review each license before accepting it.".to_string()),
            ));
        }
        let platforms = sdk.join("platforms");
        let platform_count = std::fs::read_dir(&platforms)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| entry.path().is_dir())
                    .count()
            })
            .unwrap_or(0);
        if platform_count == 0 {
            issues.push(repair_issue(
                "missing_sdk_platform",
                "optional",
                "No Android SDK Platform is installed.",
                platforms.display().to_string(),
                "Android SDK Platform",
                "Native builds and some APK inspection paths need at least one platform.",
                repair(
                    "sdk_package",
                    None,
                    Some(sdkmanager_command(Some(sdk), &["--list"])),
                    true,
                    true,
                ),
                Some(
                    "Install the Android SDK Platform that matches your app's compileSdk version."
                        .to_string(),
                ),
            ));
        }
    }

    if let Ok(metadata) = std::fs::metadata(&config.artifact_dir) {
        if !metadata.is_dir() {
            issues.push(repair_issue(
                "invalid_artifact_path",
                "blocking",
                "The artifact path exists but is not a directory.",
                config.artifact_dir.display().to_string(),
                "artifact directory",
                "Reports and evidence cannot be stored.",
                repair("filesystem", None, None, false, true),
                Some("Choose a different artifact_dir in project configuration or remove the conflicting file.".to_string()),
            ));
        } else if metadata.permissions().readonly() {
            issues.push(repair_issue(
                "artifact_directory_read_only",
                "blocking",
                "The artifact directory is read-only.",
                config.artifact_dir.display().to_string(),
                "artifact directory",
                "Reports, logs and evidence cannot be written.",
                repair("filesystem", None, None, false, true),
                Some("Grant write permission or choose another artifact directory.".to_string()),
            ));
        }
    }

    let blocking = issues
        .iter()
        .filter(|issue| issue.severity == "blocking")
        .count();
    let recommended = issues
        .iter()
        .filter(|issue| issue.severity == "recommended")
        .count();
    let optional = issues
        .iter()
        .filter(|issue| issue.severity == "optional")
        .count();
    let mut next_actions = Vec::new();
    if blocking > 0 || recommended > 0 {
        next_actions.push(serde_json::json!({
            "reason": "Re-run the structured environment check after applying repairs.",
            "priority": "normal",
            "command": {"argv": ["androidforge", "doctor", "--json"]},
        }));
    }
    if let Some(command) = issues
        .iter()
        .find(|issue| issue.severity == "blocking")
        .or_else(|| issues.iter().find(|issue| issue.severity == "recommended"))
        .and_then(|issue| issue.repair.command.clone())
    {
        next_actions.push(serde_json::json!({
            "reason": "Primary blocking repair candidate. Review it before running; --plan never executes it.",
            "priority": "high",
            "command": {"argv": command},
            "requires_confirmation": true,
        }));
    }
    if config_error.is_some() {
        next_actions.push(serde_json::json!({
            "reason": "Validate the project configuration syntax.",
            "priority": "high",
            "command": {"argv": ["androidforge", "config", "validate", "--project", paths.project_root.to_string_lossy(), "--json"]},
        }));
    }

    let status = if issues.is_empty() {
        "no_repairs_needed"
    } else {
        "plan_ready"
    };
    let result = serde_json::json!({
        "schema_version": 1,
        "status": status,
        "generated_at": now_rfc3339(),
        "read_only": true,
        "check_only": args.check,
        "mutations_performed": [],
        "project_root": paths.project_root.to_string_lossy(),
        "project_config": paths.project_config_path.as_ref().map(|path| path.to_string_lossy().into_owned()),
        "sdk_root": sdk_root.as_ref().map(|path| path.to_string_lossy().into_owned()),
        "ready": blocking == 0,
        "summary": {
            "blocking": blocking,
            "recommended": recommended,
            "optional": optional,
            "total": issues.len(),
        },
        "issues": issues,
        "next_actions": next_actions,
        "warnings": config.warnings,
    });
    let mut human = String::new();
    if issues.is_empty() {
        human.push_str("No environment repairs are needed.\n");
    } else {
        human.push_str(&format!(
            "Repair plan: {blocking} blocking, {recommended} recommended, {optional} optional. This command is read-only.\n"
        ));
        for issue in &issues {
            human.push_str(&format!(
                "[{}] {} — {}\n",
                issue.severity, issue.id, issue.description
            ));
            if let Some(command) = &issue.repair.command {
                human.push_str(&format!("  {}\n", command.join(" ")));
            }
        }
        human.push_str("No changes were made. Run suggested commands only after reviewing them.\n");
    }
    Ok(success("doctor.fix", result, human))
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
        "session.ensure",
        "supported",
        &["filesystem locks", "optional adb/emulator"],
        "Idempotent reuse/attach/launch with advisory leases and ownership refusal.",
    );
    add(
        "context",
        "supported",
        &["local state", "optional adb"],
        "Compact project/session/device/app/screen/run snapshot with partial results and argv next actions.",
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
        "apk.splits",
        if has_adb && has_aapt2 {
            "supported"
        } else {
            "unsupported"
        },
        &["aapt2", "adb"],
        "Single APKs, explicit split sets, split directories and .apks; no bundletool or device targeting.",
    );
    add(
        "app.install",
        if has_adb { "supported" } else { "unsupported" },
        &["adb"],
        "Signature-aware install diagnostics; split sets use install-multiple; never uninstalls automatically.",
    );
    add(
        "ui.uiautomator",
        if has_adb { "supported" } else { "unsupported" },
        &["adb", "uiautomator"],
        "Compose semantics are visible only when exposed through Android accessibility.",
    );
    add(
        "ui.observation",
        if has_adb { "supported" } else { "unsupported" },
        &["adb", "uiautomator"],
        "Compact semantic observations, session/boot-scoped tokens and semantic diffs.",
    );
    add(
        "ui.postconditions",
        if has_adb { "supported" } else { "unsupported" },
        &["adb", "uiautomator"],
        "One action plus observation polling; no hidden action retries.",
    );
    add(
        "ui.element_refs",
        if has_adb { "supported" } else { "unsupported" },
        &["adb", "uiautomator"],
        "Stable refs for tap, long-press, text, clear, scroll, wait and assert; no text/coordinate fallback.",
    );
    add(
        "diagnosis.explain",
        "supported",
        &["local reports"],
        "Deterministic categories, evidence, fingerprints and exact argv next actions; no LLM.",
    );
    add(
        "repair.plan",
        "supported",
        &["local toolchain"],
        "Strictly read-only environment repair planning.",
    );
    add(
        "agent.pipeline",
        if has_adb { "supported" } else { "unsupported" },
        &["adb", "project adapter"],
        "Transparent ensure/inspect/install/fixture/conformance/explain stages with dry-run.",
    );
    add(
        "agent.brief",
        "supported",
        &["local state", "optional adb"],
        "One small project/session/device/app/screen snapshot with short next commands.",
    );
    add(
        "agent.output_profiles",
        "supported",
        &["none"],
        "Full, compact and brief JSON profiles with minification, explicit caps, counts, summaries and next commands.",
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
        "diagnostics.performance",
        if has_adb {
            "best-effort"
        } else {
            "unsupported"
        },
        &["adb", "dumpsys gfxinfo/meminfo/cpuinfo", "/proc"],
        "Explicit startup, jank, memory and CPU evidence; cold start requires --cold; unknown values carry reasons.",
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
            "best-effort"
        } else {
            "unsupported"
        },
        &["maestro optional"],
        "Optional local Maestro CLI; no Maestro Cloud or accounts. Not end-to-end validated in this release.",
    );
    add(
        "instrumentation",
        if has_adb {
            "best-effort"
        } else {
            "unsupported"
        },
        &["adb", "test APK"],
        "Runs installed instrumentation; requires a test APK and does not replace Gradle. Not end-to-end validated in this release.",
    );
    add(
        "snapshots",
        if has_emulator {
            "best-effort"
        } else {
            "unsupported"
        },
        &["adb", "emulator", "--snapshot-save for manual saves"],
        "Loading always requires --yes. AndroidForge-launched emulators default to -no-snapshot-save; pass --snapshot-save to permit manual snapshots.",
    );
    add(
        "physical.safety",
        "supported",
        &["adb"],
        "Data-changing commands refuse physical devices unless --allow-physical-mutation.",
    );
    add(
        "ui.overlay_dismissal",
        if has_adb {
            "best-effort"
        } else {
            "unsupported"
        },
        &["adb", "uiautomator"],
        "Dismisses only clickable SystemUI Got it/OK/Dismiss/Close overlays; never app content or permission decisions.",
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
        (
            "analysis.schema.json",
            include_str!("../../../../schema/analysis.schema.json"),
        ),
        (
            "maintenance.schema.json",
            include_str!("../../../../schema/maintenance.schema.json"),
        ),
        (
            "context.schema.json",
            include_str!("../../../../schema/context.schema.json"),
        ),
        (
            "observation.schema.json",
            include_str!("../../../../schema/observation.schema.json"),
        ),
        (
            "ui-diff.schema.json",
            include_str!("../../../../schema/ui-diff.schema.json"),
        ),
        (
            "diagnosis.schema.json",
            include_str!("../../../../schema/diagnosis.schema.json"),
        ),
        (
            "session-ensure.schema.json",
            include_str!("../../../../schema/session-ensure.schema.json"),
        ),
        (
            "repair-plan.schema.json",
            include_str!("../../../../schema/repair-plan.schema.json"),
        ),
        (
            "agent-test.schema.json",
            include_str!("../../../../schema/agent-test.schema.json"),
        ),
        (
            "action-receipt.schema.json",
            include_str!("../../../../schema/action-receipt.schema.json"),
        ),
        (
            "recurrence.schema.json",
            include_str!("../../../../schema/recurrence.schema.json"),
        ),
        (
            "apk-resolution.schema.json",
            include_str!("../../../../schema/apk-resolution.schema.json"),
        ),
        (
            "apk-diff.schema.json",
            include_str!("../../../../schema/apk-diff.schema.json"),
        ),
        (
            "bundle.schema.json",
            include_str!("../../../../schema/bundle.schema.json"),
        ),
        (
            "health.schema.json",
            include_str!("../../../../schema/health.schema.json"),
        ),
        (
            "matrix.schema.json",
            include_str!("../../../../schema/matrix.schema.json"),
        ),
        (
            "performance.schema.json",
            include_str!("../../../../schema/performance.schema.json"),
        ),
        (
            "agent-brief.schema.json",
            include_str!("../../../../schema/agent-brief.schema.json"),
        ),
        (
            "wireless.schema.json",
            include_str!("../../../../schema/wireless.schema.json"),
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
