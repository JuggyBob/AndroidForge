//! Local MCP stdio server and function-calling tool catalog.
//!
//! The server deliberately shells out to the same `androidforge` binary. That
//! keeps the MCP surface, the CLI and the JSON contract on one implementation
//! instead of creating a second command engine.

use std::io::{BufRead, Cursor, Read, Write};
use std::process::Command;

use androidforge_core::config::ForgeConfig;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::maestro;
use androidforge_core::output::CommandSuccess;
use androidforge_core::paths::ForgePaths;
use androidforge_core::toolchain::Toolchain;
use androidforge_core::util::sha256_hex;
use serde_json::{Value, json};

use crate::cli::{Cli, McpCommand, ToolsCommand, ToolsSchemaArgs};

pub fn mcp(command: &McpCommand) -> Result<CommandSuccess> {
    match command {
        McpCommand::Serve => Err(ForgeError::new(
            ErrorCode::UnsupportedOperation,
            "mcp serve is a long-running stdio command and must be started directly",
        )
        .with_suggestion("Run `androidforge mcp serve` as a child process.")),
    }
}

pub fn tools(cli: &Cli, command: &ToolsCommand) -> Result<CommandSuccess> {
    match command {
        ToolsCommand::Schema(args) => tools_schema(args),
        ToolsCommand::Catalog => {
            let catalog = crate::commands::catalog::catalog();
            let count = catalog.commands.len();
            Ok(CommandSuccess::new(
                "tools.catalog",
                serde_json::to_value(&catalog)?,
                format!("{count} CLI commands.\n"),
            ))
        }
        ToolsCommand::Install { tool, force } => install_tool(cli, tool, *force),
        ToolsCommand::Status { tool } => tool_status(cli, tool.as_deref()),
        ToolsCommand::Uninstall { tool } => uninstall_tool(cli, tool),
    }
}

fn install_tool(cli: &Cli, tool: &str, force: bool) -> Result<CommandSuccess> {
    if !tool.eq_ignore_ascii_case("maestro") {
        return Err(
            ForgeError::invalid_argument(format!("unknown optional tool '{tool}'"))
                .with_suggestion("Use `androidforge tools install maestro`."),
        );
    }
    if !cfg!(windows) {
        return Err(ForgeError::new(
            ErrorCode::UnsupportedOperation,
            "automatic Maestro installation is currently supported on Windows only",
        )
        .with_suggestion("Install Maestro manually and set [sdk].maestro or [maestro].binary.")
        .with_suggestion("Native `androidforge ui` commands do not require Maestro."));
    }
    let paths = ForgePaths::discover(None, cli.project.clone())?;
    let config = ForgeConfig::load(&paths)?;
    let version = maestro::MAESTRO_PINNED_VERSION;
    let target = paths.tools_dir().join("maestro").join(version);
    let launcher = target
        .join("maestro")
        .join("bin")
        .join(maestro::maestro_launcher_name());
    if launcher.is_file() && !force {
        return Ok(tool_success(
            "tools.install",
            json!({
                "tool": "maestro",
                "version": version,
                "installed": true,
                "already_installed": true,
                "path": launcher.to_string_lossy(),
                "managed_dir": target.to_string_lossy(),
                "sha256": maestro::MAESTRO_PINNED_ZIP_SHA256,
            }),
            format!(
                "Maestro {version} is already installed at {}.\n",
                launcher.display()
            ),
        ));
    }
    if cli.dry_run {
        return Ok(tool_success(
            "tools.install",
            json!({
                "dry_run": true,
                "tool": "maestro",
                "version": version,
                "url": maestro::MAESTRO_PINNED_ZIP_URL,
                "sha256": maestro::MAESTRO_PINNED_ZIP_SHA256,
                "target": target.to_string_lossy(),
                "requires_confirmation": true,
            }),
            format!(
                "Would download Maestro {version} from {}\ntarget: {}\nverify SHA-256: {}\n",
                maestro::MAESTRO_PINNED_ZIP_URL,
                target.display(),
                maestro::MAESTRO_PINNED_ZIP_SHA256
            ),
        ));
    }
    if Toolchain::discover(&config).java.is_none() {
        return Err(ForgeError::new(
            ErrorCode::ToolNotFound,
            "Java is required to run Maestro, but no Java runtime was found",
        )
        .with_suggestion("Install a JDK 17 or newer, or set [sdk].java / JAVA_HOME.")
        .with_suggestion("Run `androidforge doctor --json` to inspect Java discovery."));
    }
    if !cli.yes {
        return Err(ForgeError::invalid_argument(
            "installing Maestro downloads about 350 MB; pass --yes to confirm",
        )
        .with_suggestion("Use `--dry-run` to inspect the pinned URL, hash and target first."));
    }
    eprintln!(
        "Downloading Maestro {} (about 350 MB) from {}",
        version,
        maestro::MAESTRO_PINNED_ZIP_URL
    );
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(900)))
        .build()
        .into();
    let mut response = agent
        .get(maestro::MAESTRO_PINNED_ZIP_URL)
        .header(
            "User-Agent",
            format!("AndroidForge/{}", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|error| {
            ForgeError::new(
                ErrorCode::CommandFailed,
                format!("could not download Maestro: {error}"),
            )
            .with_suggestion("Check network access and try again.")
        })?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(420 * 1024 * 1024)
        .read_to_vec()
        .map_err(|error| {
            ForgeError::new(
                ErrorCode::CommandFailed,
                format!("could not read the Maestro download: {error}"),
            )
        })?;
    let actual = sha256_hex(&bytes);
    if actual != maestro::MAESTRO_PINNED_ZIP_SHA256 {
        return Err(ForgeError::new(
            ErrorCode::SignatureVerificationFailed,
            format!(
                "Maestro download checksum mismatch: expected {}, got {actual}",
                maestro::MAESTRO_PINNED_ZIP_SHA256
            ),
        )
        .with_suggestion(
            "Do not run the downloaded file. Retry the install, and report this if it repeats.",
        ));
    }
    let staging = paths
        .tools_dir()
        .join("maestro")
        .join(format!(".install-{version}-{}", std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|error| {
            ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!("could not clear {}: {error}", staging.display()),
            )
        })?;
    }
    androidforge_core::util::ensure_dir(&staging)?;
    if let Err(error) = extract_maestro_zip(&bytes, &staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    let staged_launcher = staging
        .join("maestro")
        .join("bin")
        .join(maestro::maestro_launcher_name());
    if !staged_launcher.is_file() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            "the Maestro archive did not contain maestro/bin/maestro.bat",
        ));
    }
    if target.exists()
        && let Err(error) = std::fs::remove_dir_all(&target)
    {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            format!("could not replace {}: {error}", target.display()),
        ));
    }
    if let Some(parent) = target.parent() {
        androidforge_core::util::ensure_dir(parent)?;
    }
    std::fs::rename(&staging, &target).map_err(|error| {
        let _ = std::fs::remove_dir_all(&staging);
        ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            format!(
                "could not activate Maestro at {}: {error}",
                target.display()
            ),
        )
    })?;
    let installed_launcher = target
        .join("maestro")
        .join("bin")
        .join(maestro::maestro_launcher_name());
    let metadata = json!({
        "tool": "maestro",
        "version": version,
        "url": maestro::MAESTRO_PINNED_ZIP_URL,
        "sha256": maestro::MAESTRO_PINNED_ZIP_SHA256,
        "archive_bytes": bytes.len(),
        "installed_at": androidforge_core::util::now_rfc3339(),
        "launcher": installed_launcher.to_string_lossy(),
    });
    androidforge_core::util::write_json_atomic(&target.join("install.json"), &metadata)?;
    Ok(tool_success(
        "tools.install",
        json!({
            "tool": "maestro",
            "version": version,
            "installed": true,
            "already_installed": false,
            "path": installed_launcher.to_string_lossy(),
            "managed_dir": target.to_string_lossy(),
            "sha256": maestro::MAESTRO_PINNED_ZIP_SHA256,
            "verified": true,
        }),
        format!(
            "Installed Maestro {version} at {}.\nVerified SHA-256 {}.\n",
            installed_launcher.display(),
            maestro::MAESTRO_PINNED_ZIP_SHA256
        ),
    ))
}

fn extract_maestro_zip(bytes: &[u8], destination: &std::path::Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| {
        ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            format!("the Maestro download is not a valid ZIP: {error}"),
        )
    })?;
    if archive.len() > 20_000 {
        return Err(ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            "the Maestro archive has an unexpected number of entries",
        ));
    }
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!("could not read Maestro archive entry {index}: {error}"),
            )
        })?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!(
                    "Maestro archive entry '{}' escapes the install directory",
                    entry
                        .name()
                        .map(|name| name.into_owned())
                        .unwrap_or_else(|_| "<invalid>".to_string())
                ),
            ));
        };
        total = total.saturating_add(entry.size());
        if total > 600 * 1024 * 1024 {
            return Err(ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                "the Maestro archive expands beyond the 600 MB safety limit",
            ));
        }
        let path = destination.join(&relative);
        if entry.is_dir() {
            androidforge_core::util::ensure_dir(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            androidforge_core::util::ensure_dir(parent)?;
        }
        let mut output = std::fs::File::create(&path).map_err(|error| {
            ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!("could not create {}: {error}", path.display()),
            )
        })?;
        std::io::copy(&mut entry, &mut output).map_err(|error| {
            ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!("could not extract {}: {error}", path.display()),
            )
        })?;
    }
    Ok(())
}

fn tool_status(cli: &Cli, requested: Option<&str>) -> Result<CommandSuccess> {
    if let Some(tool) = requested
        && !tool.eq_ignore_ascii_case("maestro")
    {
        return Err(
            ForgeError::invalid_argument(format!("unknown optional tool '{tool}'"))
                .with_suggestion("Use `androidforge tools status maestro`."),
        );
    }
    let paths = ForgePaths::discover(None, cli.project.clone())?;
    let config = ForgeConfig::load(&paths)?;
    let toolchain = Toolchain::discover(&config);
    let managed = maestro::latest_installed();
    let path_on_disk = toolchain.maestro.clone();
    let managed_path = managed.as_ref().map(|install| install.launcher.clone());
    let installed = path_on_disk.is_some();
    let managed_active = match (&path_on_disk, &managed_path) {
        (Some(active), Some(managed)) => active == managed,
        _ => false,
    };
    let source = match (&path_on_disk, managed_active) {
        (Some(_), true) => "androidforge-managed",
        (Some(_), false) => "external",
        (None, _) => "not-installed",
    };
    let value = json!({
        "tool": "maestro",
        "installed": installed,
        "path": path_on_disk.clone().map(|path| path.to_string_lossy().into_owned()),
        "source": source,
        "managed": managed_active,
        "pinned_version": maestro::MAESTRO_PINNED_VERSION,
        "managed_version": managed.as_ref().map(|install| install.version.clone()),
        "managed_dir": managed.as_ref().and_then(|install| install.root.parent().map(|parent| parent.to_string_lossy().into_owned())),
        "java": toolchain.java.as_ref().map(|path| path.to_string_lossy().into_owned()),
        "install_command": "androidforge tools install maestro --yes",
    });
    let human = if let Some(path) = &path_on_disk {
        format!(
            "maestro installed ({})\npath: {}\nmanaged: {}\nversion: {}\n",
            source,
            path.display(),
            managed_active,
            managed
                .as_ref()
                .map(|install| install.version.as_str())
                .unwrap_or("-")
        )
    } else {
        format!(
            "maestro not installed\ninstall: androidforge tools install maestro --yes\npinned version: {}\n",
            maestro::MAESTRO_PINNED_VERSION
        )
    };
    Ok(tool_success("tools.status", value, human))
}

fn uninstall_tool(cli: &Cli, tool: &str) -> Result<CommandSuccess> {
    if !tool.eq_ignore_ascii_case("maestro") {
        return Err(
            ForgeError::invalid_argument(format!("unknown optional tool '{tool}'"))
                .with_suggestion("Use `androidforge tools uninstall maestro`."),
        );
    }
    if cli.dry_run {
        return Ok(tool_success(
            "tools.uninstall",
            json!({"dry_run": true, "tool": "maestro"}),
            "Would remove the AndroidForge-managed Maestro installation.\n".to_string(),
        ));
    }
    if !cli.yes {
        return Err(ForgeError::invalid_argument(
            "uninstalling Maestro removes the managed installation; pass --yes",
        ));
    }
    let paths = ForgePaths::discover(None, cli.project.clone())?;
    let root = paths.tools_dir().join("maestro");
    let managed = maestro::latest_installed();
    let Some(install) = managed else {
        return Err(ForgeError::new(
            ErrorCode::ToolNotFound,
            "no AndroidForge-managed Maestro installation was found",
        )
        .with_suggestion(
            "Only manager-installed copies can be removed here; remove external installs manually.",
        ));
    };
    let version_dir = install
        .root
        .parent()
        .ok_or_else(|| ForgeError::internal("invalid managed Maestro path"))?
        .to_path_buf();
    if !version_dir.starts_with(&root) {
        return Err(ForgeError::new(
            ErrorCode::Internal,
            "refusing to remove a Maestro path outside the managed tools directory",
        ));
    }
    std::fs::remove_dir_all(&version_dir).map_err(|error| {
        ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            format!("could not remove {}: {error}", version_dir.display()),
        )
    })?;
    Ok(tool_success(
        "tools.uninstall",
        json!({
            "tool": "maestro",
            "removed": true,
            "version": install.version,
            "path": version_dir.to_string_lossy(),
        }),
        format!("Removed managed Maestro {}.\n", install.version),
    ))
}

fn tool_success(command: &str, value: Value, human: String) -> CommandSuccess {
    CommandSuccess::new(command, value, human)
}

fn tools_schema(args: &ToolsSchemaArgs) -> Result<CommandSuccess> {
    let defs = tool_definitions();
    let format = args.format.to_ascii_lowercase();
    let value = match format.as_str() {
        "mcp" | "json" => json!({
            "schema_version": 1,
            "tools": defs,
        }),
        "openai" => json!({
            "schema_version": 1,
            "tools": defs
                .iter()
                .map(|tool| json!({
                    "type": "function",
                    "function": {
                        "name": tool["name"],
                        "description": tool["description"],
                        "parameters": tool["inputSchema"],
                    }
                }))
                .collect::<Vec<_>>(),
        }),
        other => {
            return Err(ForgeError::invalid_argument(format!(
                "unsupported tool schema format '{other}'"
            ))
            .with_suggestion("Use --format mcp, openai or json."));
        }
    };
    let human = serde_json::to_string_pretty(&value)? + "\n";
    Ok(CommandSuccess::new("tools.schema", value, human))
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn selector_properties() -> Value {
    json!({
        "text": {"type": "string", "description": "Exact visible text."},
        "contains": {"type": "string", "description": "Substring of visible text."},
        "content_desc": {"type": "string"},
        "resource_id": {"type": "string"},
        "test_tag": {"type": "string", "description": "Compose testTag exposed as an accessibility resource id in debug builds."},
        "class": {"type": "string"},
        "role": {"type": "string", "description": "Semantic role such as button, edit_text, tab, checkbox."},
        "text_regex": {"type": "string", "description": "Regular expression matched against text/content description."},
        "within_text": {"type": "string", "description": "Restrict candidates to the bounds of a node with this text."},
        "above_text": {"type": "string"},
        "below_text": {"type": "string"},
        "left_of_text": {"type": "string"},
        "right_of_text": {"type": "string"},
        "region": {"type": "string", "description": "x1,y1,x2,y2 device-pixel bounds."},
        "index": {"type": "integer", "minimum": 0},
        "clickable": {"type": "boolean"},
        "enabled": {"type": "boolean"},
        "visible": {"type": "boolean"},
        "ocr_fallback": {"type": "boolean", "description": "Try local OCR when UIAutomator has no match."},
    })
}

fn merge(base: &mut serde_json::Map<String, Value>, extra: Value) {
    if let Some(extra) = extra.as_object() {
        base.extend(extra.clone());
    }
}

fn common_selector(required: &[&str]) -> Value {
    let mut props = selector_properties()
        .as_object()
        .cloned()
        .unwrap_or_default();
    props.insert("project".to_string(), json!({"type": "string"}));
    props.insert("session".to_string(), json!({"type": "string"}));
    props.insert("ref".to_string(), json!({"type": "string"}));
    props.insert("expect_token".to_string(), json!({"type": "string"}));
    schema(Value::Object(props), required)
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    })
}

pub fn tool_definitions() -> Vec<Value> {
    let mut tools = Vec::new();
    tools.push(tool(
        "androidforge_version",
        "Return the AndroidForge CLI version.",
        schema(json!({}), &[]),
    ));
    tools.push(tool(
        "androidforge_doctor",
        "Inspect the host, SDK, ADB, emulator, build tools and sessions. Read-only.",
        schema(json!({}), &[]),
    ));
    tools.push(tool(
        "androidforge_doctor_capacity",
        "Report conservative host RAM/commit/disk capacity and per-AVD commit estimates. Unknown values stay null/unknown with a reason. Read-only.",
        schema(
            json!({
                "project": {"type": "string", "description": "Project directory containing .androidforge/project.toml."},
                "output_profile": {"enum": ["full", "compact", "brief"]},
            }),
            &[],
        ),
    ));
    tools.push(tool(
        "androidforge_build",
        "Run the configured host build command for a variant and resolve the produced APK. Host-only: bounded local log capture, no session or device access. Install/launch flags are deliberately omitted; use androidforge_app_install / androidforge_app_launch, which keep their physical-device gates.",
        schema(
            json!({
                "project": {"type": "string", "description": "Project directory containing .androidforge/project.toml."},
                "variant": {"type": "string", "description": "Variant in [build.commands.<variant>]. Defaults to [build].default_variant."},
                "module": {"type": "string", "description": "Restrict APK resolution to one module directory."},
                "timeout": {"type": "string", "description": "Build timeout override, for example 20m."},
                "expected_package": {"type": "string", "description": "Require the built APK to declare this package."},
                "dry_run": {"type": "boolean", "description": "Plan without executing the build or writing artifacts."},
            }),
            &[],
        ),
    ));
    tools.push(tool(
        "androidforge_capabilities",
        "Return the implemented, best-effort and unsupported capability matrix.",
        schema(json!({}), &[]),
    ));
    tools.push(tool(
        "androidforge_devices_list",
        "List connected physical devices and emulators.",
        schema(json!({}), &[]),
    ));
    tools.push(tool(
        "androidforge_context",
        "Return a compact project/session/device/app/screen snapshot with next actions.",
        schema(
            json!({
                "project": {"type": "string"},
                "session": {"type": "string"},
                "output_profile": {"enum": ["full", "compact", "brief"]},
                "refresh_screen": {"type": "boolean"},
                "no_screen": {"type": "boolean"},
            }),
            &[],
        ),
    ));
    tools.push(tool(
        "androidforge_session_ensure",
        "Idempotently acquire, attach or launch a device session. Supports waiting for a busy device.",
        schema(
            json!({
                "avd": {"type": "string"},
                "serial": {"type": "string"},
                "owner": {"type": "string"},
                "ttl": {"type": "string"},
                "headless": {"type": "boolean"},
                "wait": {"type": "boolean"},
                "timeout": {"type": "string"},
                "project": {"type": "string"},
            }),
            &[],
        ),
    ));
    tools.push(tool(
        "androidforge_ui_observe",
        "Capture a semantic UI observation and return a session-scoped token.",
        schema(
            json!({
                "session": {"type": "string"},
                "project": {"type": "string"},
                "package": {"type": "string"},
                "full": {"type": "boolean"},
                "semantics": {"type": "boolean"},
                "retries": {"type": "integer", "minimum": 0, "maximum": 5},
            }),
            &["session"],
        ),
    ));
    tools.push(tool(
        "androidforge_ui_find",
        "Find UI nodes with selectors, relative bounds, regex/role matching or OCR fallback.",
        common_selector(&["session"]),
    ));
    tools.push(tool(
        "androidforge_ui_bounds",
        "Read the eight bounds fields of one semantic UI element. Read-only; no physical gate.",
        common_selector(&["session"]),
    ));
    tools.push(tool(
        "androidforge_ui_assert_bounds",
        "Assert bottom_y with eq/gt/lt and tolerance. Read-only; failures are E_UI_ASSERTION_FAILED with comparison context.",
        merge_common_selector(
            common_selector(&["session"]),
            json!({
                "bottom_y": {"type": "integer", "description": "Expected bottom edge for equality."},
                "bottom_y_gt": {"type": "integer", "description": "Assert bottom_y is greater than this value plus tolerance."},
                "bottom_y_lt": {"type": "integer", "description": "Assert bottom_y is less than this value minus tolerance."},
                "tolerance": {"type": "integer", "minimum": 0, "default": 0, "description": "Dead-band in device pixels."},
            }),
        ),
    ));
    tools.push(tool(
        "androidforge_ui_tap",
        "Tap a UI element by selector or stable ref, optionally waiting for a postcondition.",
        merge_common_selector(
            common_selector(&["session"]),
            json!({
                "wait_for_text": {"type": "string"},
                "wait_for_absent_text": {"type": "string"},
                "wait_for_package": {"type": "string"},
                "wait_for_activity": {"type": "string"},
                "timeout": {"type": "string"},
                "x": {"type": "integer"},
                "y": {"type": "integer"},
            }),
        ),
    ));
    tools.push(tool(
        "androidforge_ui_text",
        "Type text into an editable field, optionally clearing it first and hiding the keyboard.",
        merge_common_selector(
            common_selector(&["session", "text"]),
            json!({
                "text": {"type": "string"},
                "clear": {"type": "boolean"},
                "hide_keyboard": {"type": "boolean"},
            }),
        ),
    ));
    tools.push(tool(
        "androidforge_ui_dismiss_overlays",
        "Dismiss known SystemUI Got it/OK/Dismiss/Close overlays. Never taps app content.",
        schema(
            json!({
                "session": {"type": "string"},
                "project": {"type": "string"},
                "max": {"type": "integer", "minimum": 1, "maximum": 10},
                "dry_run": {"type": "boolean"},
                "confirm": {"type": "boolean", "description": "Required unless dry_run is true."},
            }),
            &["session"],
        ),
    ));
    tools.push(tool(
        "androidforge_app_verify_installed",
        "Compare the live installed base APK bytes/hash with a local APK without installing it.",
        schema(
            json!({
                "apk": {"type": "string"},
                "split": {"type": "array", "items": {"type": "string"}},
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["apk", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_app_status",
        "Read live package state and assert expected installed version or base APK SHA-256.",
        schema(
            json!({
                "package": {"type": "string"},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "expect_version_name": {"type": "string"},
                "expect_version_code": {"type": "integer"},
                "expect_apk_sha256": {"type": "string"},
                "assert_task_exists": {"type": "boolean"},
                "assert_task_absent": {"type": "boolean"},
                "assert_process_stable": {"type": "boolean"},
                "stable_for": {"type": "string"},
                "timeout": {"type": "string"},
            }),
            &["package", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_app_data",
        "Export, import, reset or seed app-private data. Export defaults to the managed artifacts/export destination; --output selects an explicit path (in-project and overwrite need explicit opt-in). Import/reset/seed require confirm=true.",
        schema(
            json!({
                "action": {"type": "string", "enum": ["export", "import", "reset", "seed"]},
                "package": {"type": "string"},
                "path": {"type": "string", "description": "Legacy positional destination for export/import; use output for the named export form."},
                "output": {"type": "string", "description": "Explicit export destination (--output). Conflicts with path."},
                "allow_in_project": {"type": "boolean", "description": "Allow an explicit export destination inside the project root."},
                "overwrite": {"type": "boolean", "description": "Replace an existing export destination instead of refusing it."},
                "baseline": {"type": "string"},
                "backup_current": {"type": "string"},
                "file": {"type": "string"},
                "remote": {"type": "string"},
                "confirm": {"type": "boolean"},
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["action", "package", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_maintenance_clean",
        "Prune old terminal sessions and run artifacts. Host-only and destructive: confirm=true is required to delete; keep_tag protects runs carrying any listed tag, and using it without confirm is a dry run. Running, active and protected runs are never deleted.",
        schema(
            json!({
                "project": {"type": "string"},
                "older_than": {"type": "string", "description": "Age threshold, for example 30d. Default 30d."},
                "keep_last": {"type": "integer", "minimum": 0, "description": "Preserve the newest N runs. Default 10."},
                "keep_tag": {"type": "array", "items": {"type": "string"}, "description": "Protect runs carrying any of these tags. Repeatable."},
                "sessions": {"type": "boolean", "description": "Include terminal session tombstones."},
                "runs": {"type": "boolean", "description": "Include terminal run directories."},
                "dry_run": {"type": "boolean", "description": "Preview without deleting anything."},
                "confirm": {"type": "boolean", "description": "Required to actually delete."},
            }),
            &[],
        ),
    ));
    tools.push(tool(
        "androidforge_ui_drag",
        "Perform a press-move-release drag using coordinates or semantic endpoints.",
        schema(
            json!({
                "session": {"type": "string"},
                "project": {"type": "string"},
                "from_x": {"type": "integer"},
                "from_y": {"type": "integer"},
                "to_x": {"type": "integer"},
                "to_y": {"type": "integer"},
                "from_text": {"type": "string"},
                "from_id": {"type": "string"},
                "to_text": {"type": "string"},
                "to_id": {"type": "string"},
                "from": {"type": "string"},
                "to": {"type": "string"},
                "duration_ms": {"type": "integer", "minimum": 1},
                "hold_before_ms": {"type": "integer", "minimum": 0},
                "hold_after_ms": {"type": "integer", "minimum": 0},
                "capture_mid": {"type": "string"},
            }),
            &["session"],
        ),
    ));
    tools.push(tool(
        "androidforge_ui_wait",
        "Wait for a selector, for text to disappear, for an idle hierarchy, for a stable hierarchy, or for stable element bounds.",
        merge_common_selector(
            common_selector(&["session"]),
            json!({
                "not_exists": {"type": "boolean"},
                "for_text_gone": {"type": "array", "items": {"type": "string"}},
                "for_idle": {"type": "boolean"},
                "for_stable": {"type": "boolean"},
                "for_bounds_stable": {"type": "boolean", "description": "Re-resolve the selector until its bounds stay unchanged for stable_for. Requires a semantic selector."},
                "stable_for": {"type": "string"},
                "timeout": {"type": "string"},
            }),
        ),
    ));
    tools.push(tool(
        "androidforge_ui_assert_no_text",
        "Assert that matching visible text or content descriptions are absent.",
        schema(
            json!({
                "session": {"type": "string"},
                "project": {"type": "string"},
                "text": {"type": "string"},
                "contains": {"type": "string"},
                "timeout": {"type": "string"},
            }),
            &["session"],
        ),
    ));
    tools.push(tool(
        "androidforge_network_reverse",
        "List, add, remove or clear adb reverse mappings for this session.",
        schema(
            json!({
                "action": {"type": "string", "enum": ["add", "list", "remove", "clear"]},
                "remote": {"type": "string"},
                "local": {"type": "string"},
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["action", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_network_mock_serve",
        "Run a bounded local mock HTTP(S) server. Raw routes need max_requests or duration so the call returns; built-in scenarios carry their own bounds. Scenarios are loopback-only, redacted and host-only; --reverse is not allowed with them.",
        schema(
            json!({
                "scenario": {"type": "string", "enum": ["newer-release", "offline", "bad-digest", "no-APK"], "description": "Built-in scenario name; routes and bounds come from its fixture."},
                "port": {"type": "integer", "minimum": 1, "maximum": 65535},
                "bind": {"type": "string"},
                "routes_file": {"type": "string"},
                "route": {"type": "array", "items": {"type": "string"}},
                "default_status": {"type": "integer"},
                "default_body": {"type": "string"},
                "delay_ms": {"type": "integer", "minimum": 0},
                "tls_cert": {"type": "string"},
                "tls_key": {"type": "string"},
                "max_requests": {"type": "integer", "minimum": 0},
                "duration": {"type": "string"},
                "reverse": {"type": "boolean"},
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &[],
        ),
    ));
    tools.push(tool(
        "androidforge_network_mock_scenarios",
        "List built-in network mock scenarios with mode and fixture file. Read-only: no session, device or network change.",
        schema(json!({}), &[]),
    ));
    tools.push(tool(
        "androidforge_session_list",
        "List sessions with classification, age and optional active/mine filters.",
        schema(
            json!({
                "active": {"type": "boolean"},
                "mine": {"type": "boolean"},
                "owner": {"type": "string"},
                "project": {"type": "string"},
            }),
            &[],
        ),
    ));
    tools.push(tool(
        "androidforge_capture_screenshot",
        "Capture a PNG screenshot and return its artifact path, size and hash.",
        schema(
            json!({
                "session": {"type": "string"},
                "project": {"type": "string"},
                "label": {"type": "string"},
                "output": {"type": "string"},
                "compare": {"type": "string"},
                "region": {"type": "string"},
                "max_diff_ratio": {"type": "number"},
                "diff_threshold": {"type": "integer", "minimum": 0, "maximum": 255},
                "diff_output": {"type": "string"},
            }),
            &["session"],
        ),
    ));
    tools.push(tool(
        "androidforge_diagnostics_frames",
        "Collect gfxinfo frame timing into frames/report.json, with optional baseline thresholds. Read-only; exit 11 on a measured regression.",
        schema(
            json!({
                "package": {"type": "string", "description": "Package whose gfxinfo counters are collected."},
                "reset": {"type": "boolean", "description": "Reset gfxinfo counters before collecting."},
                "duration": {"type": "string", "description": "Explicit sampling window, for example 5s."},
                "timeout": {"type": "string", "description": "Per-command timeout, for example 30s."},
                "baseline": {"type": "string", "description": "Baseline frames/report.json to compare against."},
                "max_janky": {"type": "number", "minimum": 0, "description": "Allowed increase in jank percentage points."},
                "max_p99": {"type": "number", "minimum": 0, "description": "Allowed increase in p99 frame time in ms."},
                "max_frame_ms": {"type": "number", "minimum": 0, "description": "Allowed increase in max frame time in ms."},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "serial": {"type": "string"},
                "output_profile": {"enum": ["full", "compact", "brief"]},
                "dry_run": {"type": "boolean"},
            }),
            &["package"],
        ),
    ));
    tools.push(tool(
        "androidforge_capture_assert_color",
        "Check the colour of a fresh, host-only screenshot against #RRGGBB. Read-only; mismatch is E_UI_ASSERTION_FAILED.",
        schema(
            json!({
                "rgb": {"type": "string", "description": "Expected opaque colour in #RRGGBB form."},
                "at": {"type": "string", "description": "Sample one device pixel at x,y."},
                "region": {"type": "string", "description": "Device-pixel region x,y,w,h to sample."},
                "majority": {"type": "boolean", "description": "Use the most frequent colour of the region."},
                "tolerance": {"type": "integer", "minimum": 0, "maximum": 255, "description": "Max per-channel difference (default 16)."},
                "timeout": {"type": "string", "description": "Screenshot timeout, for example 10s."},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "serial": {"type": "string"},
                "output_profile": {"enum": ["full", "compact", "brief"]},
                "dry_run": {"type": "boolean", "description": "Plan without a device session; no device state is changed."},
            }),
            &["rgb"],
        ),
    ));
    tools.push(tool(
        "androidforge_capture_launch",
        "Launch a package and sample bounded screencap frames to assert the first app frame. Launches by design; uses existing scoped app-control consent, no physical mutation flag.",
        schema(
            json!({
                "package": {"type": "string", "description": "Package to launch and observe."},
                "activity": {"type": "string"},
                "fps": {"type": "integer", "minimum": 1, "maximum": 30, "description": "Samples per second (default 10)."},
                "duration": {"type": "string", "description": "Sampling window, for example 5s."},
                "ui_dump_package": {"type": "boolean", "description": "Fall back to uiautomator dump /dev/tty when focus does not match."},
                "expect_first_frame": {"type": "boolean"},
                "no_splash": {"type": "boolean"},
                "expected_color": {"type": "string", "description": "Expected #RRGGBB colour of the first app frame."},
                "color_sample": {"enum": ["centre", "dominant"], "description": "Which colour to check against expected_color."},
                "tolerance": {"type": "integer", "minimum": 0, "maximum": 255},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "serial": {"type": "string"},
                "output_profile": {"enum": ["full", "compact", "brief"]},
                "dry_run": {"type": "boolean"},
            }),
            &["package"],
        ),
    ));
    tools.push(tool(
        "androidforge_app_install",
        "Install an APK or split set. `fresh` requires confirm=true. `wait_for_confirmation` watches for an OEM on-device prompt and retries once only after an unambiguous audited tap; install consent is unchanged.",
        schema(
            json!({
                "apk": {"type": "string"},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "fresh": {"type": "boolean"},
                "only_if_changed": {"type": "boolean"},
                "allow_downgrade": {"type": "boolean"},
                "wait_for_confirmation": {"type": "boolean", "description": "Wait for an OEM install confirmation dialog after a restricted install failure."},
                "confirm_text": {"type": "array", "items": {"type": "string"}, "description": "Confirm button labels; overrides the built-in localized list."},
                "confirmation_timeout": {"type": "string", "description": "Maximum dialog wait, for example 15s."},
                "confirm": {"type": "boolean"},
            }),
            &["apk", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_app_uninstall",
        "Uninstall a package. Requires confirm=true and passes --yes.",
        schema(
            json!({
                "package": {"type": "string"},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "keep_data": {"type": "boolean"},
                "confirm": {"type": "boolean"},
            }),
            &["package", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_fixture_run",
        "Run a project fixture. Destructive fixtures require confirm=true.",
        schema(
            json!({
                "name": {"type": "string"},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "phases": {"type": "array", "items": {"type": "string"}},
                "vars": {"type": "array", "items": {"type": "string"}},
                "apk": {"type": "string"},
                "confirm": {"type": "boolean"},
            }),
            &["name", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_fixture_apply",
        "Apply only the declarative app state of a fixture (prefs, history, installed_version). Debug-only app-private data mutation; requires confirm=true and the physical-data gate where applicable.",
        schema(
            json!({
                "name": {"type": "string", "description": "Fixture pack name."},
                "package": {"type": "string", "description": "Defaults to the project adapter package."},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "confirm": {"type": "boolean"},
            }),
            &["name", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_fixture_reset",
        "Remove only the app-private files declared by a fixture's state block. Destructive; requires confirm=true and the physical-data gate where applicable.",
        schema(
            json!({
                "name": {"type": "string", "description": "Fixture pack name."},
                "package": {"type": "string", "description": "Defaults to the project adapter package."},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "confirm": {"type": "boolean"},
            }),
            &["name", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_scenario_run",
        "Run an allowlisted declarative scenario manifest (.json/.toml) through the audited app/UI/fixture operations. Mutating steps require confirm=true or dry_run=true; physical-device capability flags are never forwarded over MCP, so a physical device stays gated by its scoped flags.",
        schema(
            json!({
                "file": {"type": "string", "description": "Scenario manifest path (.json or .toml)."},
                "repeat": {"type": "integer", "minimum": 1, "description": "Override the manifest repeat count."},
                "dry_run": {"type": "boolean", "description": "Plan steps without dispatch; no consent or device is required."},
                "confirm": {"type": "boolean", "description": "Required for a real run with device-mutating steps."},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "output_profile": {"enum": ["full", "compact", "brief"]},
            }),
            &["file"],
        ),
    ));
    tools.push(tool(
        "androidforge_scenario_validate",
        "Parse and validate an allowlisted scenario manifest without touching a device. Read-only.",
        schema(
            json!({
                "file": {"type": "string", "description": "Scenario manifest path (.json or .toml)."},
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["file"],
        ),
    ));
    tools.push(tool(
        "androidforge_app_launch",
        "Launch an app, deep link or explicit intent. Supports actions, data URIs, categories, typed extras and flags.",
        schema(
            json!({
                "package": {"type": "string"},
                "activity": {"type": "string"},
                "component": {"type": "string", "description": "Explicit component, for example com.example/.MainActivity."},
                "action": {"type": "string"},
                "data": {"type": "string", "description": "Intent data URI, for example myapp://checkout/42."},
                "mime_type": {"type": "string"},
                "categories": {"type": "array", "items": {"type": "string"}},
                "extras": {"type": "array", "items": {"type": "string"}, "description": "key:type:value, type is s, i, l, f, z, esa or eia."},
                "flags": {"type": "array", "items": {"type": "string"}},
                "wait": {"type": "boolean"},
                "force_stop": {"type": "boolean"},
                "no_wake": {"type": "boolean"},
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["package", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_app_broadcast",
        "Send a broadcast intent with optional extras, categories and receiver permission.",
        schema(
            json!({
                "package": {"type": "string"},
                "action": {"type": "string"},
                "data": {"type": "string"},
                "mime_type": {"type": "string"},
                "categories": {"type": "array", "items": {"type": "string"}},
                "extras": {"type": "array", "items": {"type": "string"}},
                "flags": {"type": "array", "items": {"type": "string"}},
                "receiver_permission": {"type": "string"},
                "asynchronous": {"type": "boolean"},
                "timeout": {"type": "string"},
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["action", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_app_service",
        "Start or stop a service intent.",
        schema(
            json!({
                "package": {"type": "string"},
                "service": {"type": "string", "description": "Service class, for example .SyncService."},
                "component": {"type": "string"},
                "action": {"type": "string"},
                "data": {"type": "string"},
                "extras": {"type": "array", "items": {"type": "string"}},
                "stop": {"type": "boolean"},
                "timeout": {"type": "string"},
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["session"],
        ),
    ));
    // Only the read-only shell form is exposed over MCP. Physical flags are
    // never advertised or forwarded; `read_only` is forced by tool_argv.
    tools.push(tool(
        "androidforge_devices_shell",
        "Run an allowlisted read-only `devices shell` command through the typed ADB path (the 1.0.0 `device` alias is retained). MCP always passes --read-only; the mutating form and physical consent flags are not exposed. Reuse diagnostics dump commands such as dumpsys, getprop and settings get.",
        schema(
            json!({
                "command": {"type": "array", "items": {"type": "string"}, "description": "Exact remote argv after `--`; each item is quoted separately."},
                "allow": {"type": "array", "items": {"type": "string"}, "description": "Optional read-only command prefixes; replaces the conservative default list."},
                "deny": {"type": "array", "items": {"type": "string"}, "description": "Refuse commands starting with or containing these prefixes."},
                "timeout": {"type": "string", "description": "Per-command timeout, for example 30s."},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "dry_run": {"type": "boolean", "description": "Print the exact argv without resolving a device."},
            }),
            &["command", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_devices_input_tap",
        "Tap exact device-pixel coordinates through `devices input tap` and the audited UiManager::tap_xy path, returning the display size. Physical devices keep the narrow Risk::Input consent gate; MCP never forwards physical flags.",
        schema(
            json!({
                "x": {"type": "integer", "description": "Device-pixel X coordinate."},
                "y": {"type": "integer", "description": "Device-pixel Y coordinate."},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "dry_run": {"type": "boolean", "description": "Print the coordinates without resolving a device."},
            }),
            &["x", "y", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_diagnostics_logcat_query",
        "Read a bounded logcat window and return structured, filtered records. Read-only.",
        schema(
            json!({
                "session": {"type": "string"},
                "project": {"type": "string"},
                "package": {"type": "string"},
                "lines": {"type": "integer", "minimum": 1, "maximum": 20000},
                "level": {"type": "string", "description": "verbose, debug, info, warn, error or fatal."},
                "since": {"type": "string"},
                "until": {"type": "string"},
                "grep": {"type": "string", "description": "Regular expression applied to tag and message."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 20000},
                "retries": {"type": "integer", "minimum": 0, "maximum": 5},
                "redact_patterns": {"type": "array", "items": {"type": "string"}, "description": "Literal token prefixes redacted before output or storage."},
            }),
            &["session"],
        ),
    ));
    tools.push(tool(
        "androidforge_diagnostics_logcat_watch",
        "Stream logcat, crash and ANR records as NDJSON for a bounded duration.",
        schema(
            json!({
                "session": {"type": "string"},
                "project": {"type": "string"},
                "package": {"type": "string"},
                "level": {"type": "string"},
                "grep": {"type": "string"},
                "crash": {"type": "boolean"},
                "anr": {"type": "boolean"},
                "timeout": {"type": "string"},
                "max_events": {"type": "integer", "minimum": 1, "maximum": 10000},
                "redact_patterns": {"type": "array", "items": {"type": "string"}, "description": "Literal token prefixes redacted before each NDJSON record is emitted."},
            }),
            &["session"],
        ),
    ));
    tools.push(tool(
        "androidforge_network_proxy_get",
        "Read the device global HTTP proxy. Read-only.",
        schema(
            json!({
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["session"],
        ),
    ));
    tools.push(tool(
        "androidforge_network_proxy_set",
        "Set the device global HTTP proxy and record the previous value for restore.",
        schema(
            json!({
                "proxy": {"type": "string", "description": "For example http://127.0.0.1:8080."},
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["proxy", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_network_proxy_clear",
        "Clear the device global HTTP proxy and record the previous value for restore.",
        schema(
            json!({
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["session"],
        ),
    ));
    tools.push(tool(
        "androidforge_network_profile_list",
        "List built-in network profiles: default, offline, wifi-only, data-only and no-proxy.",
        schema(json!({}), &[]),
    ));
    tools.push(tool(
        "androidforge_network_profile_apply",
        "Apply a network profile and record the previous network state in the session for restore.",
        schema(
            json!({
                "name": {"type": "string", "enum": ["default", "offline", "wifi-only", "data-only", "no-proxy"]},
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["name", "session"],
        ),
    ));
    tools.push(tool(
        "androidforge_network_restore",
        "Restore network and proxy state recorded in the current session.",
        schema(
            json!({
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["session"],
        ),
    ));
    tools.push(tool(
        "androidforge_run_checkpoint",
        "Record a named resume point in a run.",
        schema(
            json!({
                "name": {"type": "string"},
                "note": {"type": "string"},
                "run": {"type": "string"},
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["name"],
        ),
    ));
    tools.push(tool(
        "androidforge_run_resume",
        "Reopen a terminal run from a checkpoint and continue appending to it.",
        schema(
            json!({
                "run": {"type": "string"},
                "from_checkpoint": {"type": "string"},
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["run"],
        ),
    ));
    tools.push(tool(
        "androidforge_run_events",
        "Return the run's NDJSON event stream as text. Does not follow.",
        schema(
            json!({
                "run": {"type": "string"},
                "session": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["run"],
        ),
    ));
    tools.push(tool(
        "androidforge_report_repro",
        "Emit a copyable reproduction script from a run's recorded exact argv.",
        schema(
            json!({
                "run": {"type": "string"},
                "output": {"type": "string"},
                "project": {"type": "string"},
            }),
            &["run"],
        ),
    ));
    tools.push(tool(
        "androidforge_report_explain",
        "Explain a run or the latest failure with diagnosis, evidence and next commands. latest/latest-failure can be narrowed by session, device, tags and since.",
        schema(
            json!({
                "run": {"type": "string", "description": "Run id, latest or latest-failure."},
                "tag": {"type": "array", "items": {"type": "string"}, "description": "Require all of these run tags when selecting latest/latest-failure."},
                "since": {"type": "string", "description": "Only runs started at or after this RFC3339 instant or duration ago, for example 30d."},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "output_profile": {"enum": ["full", "compact", "brief"]},
            }),
            &["run"],
        ),
    ));
    tools.push(tool(
        "androidforge_diagnostics_performance",
        "Measure startup, jank, memory and CPU and write performance/report.json. A real run launches the app (`am start -W`; --cold also force-stops it); physical devices need the app-control grant outside MCP, which never forwards physical capability flags. Classified device-mutation: requires confirm=true unless dry_run=true.",
        schema(
            json!({
                "package": {"type": "string", "description": "Defaults to [project].package."},
                "activity": {"type": "string", "description": "Explicit launcher activity, for example com.example/.MainActivity."},
                "cold": {"type": "boolean", "description": "Explicit cold-start force-stop request."},
                "settle": {"type": "string", "description": "Delay after launch before sampling, for example 2s."},
                "timeout": {"type": "string", "description": "Per-command timeout, for example 30s."},
                "wake": {"type": "boolean", "description": "Explicit opt-in to wake the screen. Never implicit."},
                "no_screen_check": {"type": "boolean", "description": "Opt out of the screen-on preflight."},
                "min_frames": {"type": "integer", "minimum": 1, "default": 30, "description": "Minimum rendered frames before a jank percentage is reported."},
                "frame_window": {"type": "string", "description": "Frame sample window duration, for example 5s."},
                "warmup": {"type": "string", "description": "Additional warmup before frame sampling, for example 2s."},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "serial": {"type": "string"},
                "output_profile": {"enum": ["full", "compact", "brief"]},
                "dry_run": {"type": "boolean"},
                "confirm": {"type": "boolean", "description": "Required unless dry_run is true."},
            }),
            &[],
        ),
    ));
    // Every remaining CLI leaf command gets a generated MCP tool. Curated tools
    // above stay authoritative for the high-traffic, safety-sensitive paths.
    let mut existing = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_string))
        .collect::<std::collections::HashSet<_>>();
    for command in crate::commands::catalog::command_catalog() {
        if command.safety == "long-running" || command.safety == "unsafe" {
            continue;
        }
        if !existing.insert(command.tool_name.clone()) {
            continue;
        }
        tools.push(generated_tool(&command));
    }
    tools
}

/// Global flags that MCP tools must never advertise or forward. Physical
/// capability flags are intentionally in this list: an MCP caller cannot
/// self-approve a physical-device mutation.
fn is_stripped_global(key: &str) -> bool {
    matches!(
        key,
        "session"
            | "project"
            | "serial"
            | "output_profile"
            | "confirm"
            | "wait_for_confirmation"
            | "confirm_text"
            | "confirmation_timeout"
            | "allow_physical_mutation"
            | "allow_physical_app_control"
            | "allow_physical_data"
            | "allow_physical_network"
            | "allow_physical_install"
            | "allow_physical_input"
            | "dry_run"
    )
}

fn generated_tool(command: &crate::commands::catalog::CatalogCommand) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert("session".to_string(), json!({"type": "string"}));
    properties.insert("project".to_string(), json!({"type": "string"}));
    if !serial_is_array(command) {
        properties.insert("serial".to_string(), json!({"type": "string"}));
    }
    properties.insert(
        "output_profile".to_string(),
        json!({"type": "string", "enum": ["full", "compact", "brief"]}),
    );
    properties.insert("dry_run".to_string(), json!({"type": "boolean"}));
    if command.safety != "read-only" {
        properties.insert(
            "confirm".to_string(),
            json!({"type": "boolean", "description": "Required for mutating commands."}),
        );
    }
    let mut required = Vec::new();
    for argument in &command.arguments {
        let key = argument.long.clone().unwrap_or_else(|| argument.id.clone());
        if is_stripped_global(&key) && !(key == "serial" && serial_is_array(command)) {
            continue;
        }
        let schema = match argument.kind.as_str() {
            "boolean" => json!({"type": "boolean"}),
            "enum" => json!({
                "type": "string",
                "enum": argument.possible_values,
            }),
            "array" => json!({
                "type": "array",
                "items": {"type": "string"},
            }),
            _ => json!({"type": "string"}),
        };
        let schema = match &argument.help {
            Some(help) if !help.is_empty() => {
                let mut value = schema;
                value["description"] = Value::String(help.clone());
                value
            }
            _ => schema,
        };
        if argument.required {
            required.push(Value::String(key.clone()));
        }
        properties.insert(key, schema);
    }
    let description = if command.arguments.is_empty() {
        format!(
            "{} Safety: {}. Run `androidforge {} --help` for the complete contract.",
            command.summary, command.safety, command.path
        )
    } else {
        format!(
            "{} Safety: {}. Arguments mirror the CLI. Run `androidforge {} --help` for details.",
            command.summary, command.safety, command.path
        )
    };
    tool(
        &command.tool_name,
        &description,
        schema(
            Value::Object(properties),
            &required
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
        ),
    )
}

fn serial_is_array(command: &crate::commands::catalog::CatalogCommand) -> bool {
    command
        .arguments
        .iter()
        .any(|argument| argument.id == "serial" && argument.kind == "array")
}

fn merge_common_selector(base: Value, extra: Value) -> Value {
    let mut props = base
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    merge(&mut props, extra);
    let required = base
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    schema(
        Value::Object(props),
        &required
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>(),
    )
}

fn resource_definitions() -> Vec<Value> {
    let mut resources = vec![
        json!({
            "uri": "androidforge://tools",
            "name": "AndroidForge tool catalog",
            "description": "MCP/function-calling definitions for the current build.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "androidforge://catalog",
            "name": "AndroidForge CLI command catalog",
            "description": "Every CLI leaf command, its arguments, safety class and examples.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "androidforge://schemas",
            "name": "AndroidForge schema index",
            "description": "Names of embedded JSON schemas.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "androidforge://capabilities",
            "name": "AndroidForge capabilities",
            "description": "Implemented, best-effort and unsupported capabilities.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "androidforge://context",
            "name": "AndroidForge context",
            "description": "Current project/session/device/app/screen context.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "androidforge://version",
            "name": "AndroidForge version",
            "description": "Current build version and report schema.",
            "mimeType": "application/json"
        }),
    ];
    for (name, _) in crate::commands::doctor::schema_files() {
        resources.push(json!({
            "uri": format!("androidforge://schema/{name}"),
            "name": format!("Schema {name}"),
            "description": "Embedded AndroidForge JSON Schema.",
            "mimeType": "application/json"
        }));
    }
    resources
}

#[derive(Debug, PartialEq, Eq)]
enum RequestLine {
    Eof,
    Ready,
    Oversized,
}

/// Read one newline-delimited MCP request with a hard bound on retained bytes.
///
/// `take` keeps the allocation at `max_line + 1` bytes. If the newline was not
/// part of the retained bytes, the remainder is consumed in small buffered
/// chunks so the next read starts on the following line instead of the middle
/// of the oversized request.
fn read_request_line<R: BufRead>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
    max_line: usize,
) -> Result<RequestLine> {
    buffer.clear();
    let read = reader
        .by_ref()
        .take(max_line as u64 + 1)
        .read_until(b'\n', buffer)
        .map_err(|error| ForgeError::internal(format!("could not read MCP stdin: {error}")))?;
    if read == 0 {
        return Ok(RequestLine::Eof);
    }
    if buffer.len() > max_line {
        if buffer.last() != Some(&b'\n') {
            loop {
                let chunk = reader.fill_buf().map_err(|error| {
                    ForgeError::internal(format!("could not read MCP stdin: {error}"))
                })?;
                if chunk.is_empty() {
                    break;
                }
                if let Some(index) = chunk.iter().position(|byte| *byte == b'\n') {
                    reader.consume(index + 1);
                    break;
                }
                let length = chunk.len();
                reader.consume(length);
            }
        }
        buffer.clear();
        return Ok(RequestLine::Oversized);
    }
    Ok(RequestLine::Ready)
}

pub fn serve_stdio() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut reader = std::io::BufReader::new(stdin.lock());
    let mut buffer = Vec::new();
    const MAX_REQUEST_LINE: usize = 16 * 1024 * 1024;
    loop {
        match read_request_line(&mut reader, &mut buffer, MAX_REQUEST_LINE)? {
            RequestLine::Eof => break,
            RequestLine::Oversized => {
                write_response(
                    &mut stdout,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": Value::Null,
                        "error": {"code": -32700, "message": "MCP request line exceeded the 16 MiB limit"}
                    }),
                )?;
                continue;
            }
            RequestLine::Ready => {}
        }
        let line = match std::str::from_utf8(&buffer) {
            Ok(line) => line,
            Err(error) => {
                write_response(
                    &mut stdout,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": Value::Null,
                        "error": {"code": -32700, "message": format!("MCP request was not UTF-8: {error}")}
                    }),
                )?;
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => {
                write_response(
                    &mut stdout,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": Value::Null,
                        "error": {"code": -32700, "message": format!("parse error: {error}")}
                    }),
                )?;
                continue;
            }
        };
        if let Some(response) = handle_request(&request)? {
            let shutdown = response
                .get("_shutdown")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let mut response = response;
            if let Some(object) = response.as_object_mut() {
                object.remove("_shutdown");
            }
            write_response(&mut stdout, &response)?;
            if shutdown {
                break;
            }
        }
    }
    Ok(())
}

fn write_response(stdout: &mut std::io::Stdout, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *stdout, value)?;
    stdout
        .write_all(b"\n")
        .map_err(|error| ForgeError::internal(format!("could not write MCP stdout: {error}")))?;
    stdout
        .flush()
        .map_err(|error| ForgeError::internal(format!("could not flush MCP stdout: {error}")))
}

fn handle_request(request: &Value) -> Result<Option<Value>> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    if method.is_empty() {
        return Ok(Some(error_response(id, -32600, "invalid JSON-RPC request")));
    }
    let is_notification = request.get("id").is_none();
    if is_notification {
        return Ok(None);
    }
    let response = match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2024-11-05");
            result_response(
                id,
                json!({
                    "protocolVersion": requested,
                    "capabilities": {
                        "tools": {},
                        "resources": {"subscribe": false, "listChanged": false}
                    },
                    "serverInfo": {
                        "name": "androidforge",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "instructions": "AndroidForge controls Android emulators and physical devices. Prefer context, observe and report resources over raw coordinates. Destructive tools require confirm=true."
                }),
            )
        }
        "ping" => result_response(id, json!({})),
        "tools/list" => result_response(id, json!({"tools": tool_definitions()})),
        "tools/call" => call_tool(id, &params),
        "resources/list" => result_response(id, json!({"resources": resource_definitions()})),
        "resources/read" => read_resource(id, &params),
        "shutdown" => {
            let mut response = result_response(id, json!({}));
            if let Some(object) = response.as_object_mut() {
                object.insert("_shutdown".to_string(), json!(true));
            }
            response
        }
        other => error_response(id, -32601, &format!("method not found: {other}")),
    };
    Ok(Some(response))
}

fn result_response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn call_tool(id: Value, params: &Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return error_response(id, -32602, "tools/call requires a tool name");
    };
    // The advertised tool list is the dispatch boundary. Without this check a
    // caller can invoke a hidden catalog entry (for example the unsafe `adb` or
    // `run exec` wrappers) by name even though `tools/list` omits it.
    if !tool_definitions()
        .iter()
        .any(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
    {
        return result_response(
            id,
            json!({
                "content": [{"type": "text", "text": format!("unknown AndroidForge tool: {name}")}],
                "isError": true,
            }),
        );
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    match tool_argv(name, &arguments) {
        Ok(argv) => {
            let (success, stdout, stderr) = run_cli(&argv);
            let mut text = stdout.trim().to_string();
            if !stderr.trim().is_empty() {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(stderr.trim());
            }
            if text.is_empty() {
                text = format!("androidforge exited with code {success}");
            }
            let mut result = json!({
                "content": [{"type": "text", "text": text}],
                "isError": !success,
            });
            if let Ok(value) = serde_json::from_str::<Value>(stdout.trim()) {
                result["structuredContent"] = value;
            }
            result_response(id, result)
        }
        Err(message) => result_response(
            id,
            json!({
                "content": [{"type": "text", "text": message}],
                "isError": true,
            }),
        ),
    }
}

fn run_cli(argv: &[String]) -> (bool, String, String) {
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            return (
                false,
                String::new(),
                format!("could not resolve CLI: {error}"),
            );
        }
    };
    let mut child = match Command::new(exe)
        .args(argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return (false, String::new(), format!("could not run CLI: {error}")),
    };
    let pid = child.id();
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || read_limited(stdout_pipe, 32 * 1024 * 1024));
    let stderr_reader = std::thread::spawn(move || read_limited(stderr_pipe, 8 * 1024 * 1024));
    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < Duration::from_secs(600) => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                timed_out = true;
                androidforge_process::kill_tree(pid);
                let _ = child.wait();
                break None;
            }
            Err(error) => {
                androidforge_process::kill_tree(pid);
                let _ = child.wait();
                return (
                    false,
                    String::new(),
                    format!("could not wait for CLI: {error}"),
                );
            }
        }
    };
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    if timed_out {
        return (
            false,
            stdout,
            format!(
                "{stderr}\nMCP tool execution exceeded the 600 second limit and was terminated."
            ),
        );
    }
    (
        status.is_some_and(|status| status.success()),
        stdout,
        stderr,
    )
}

fn read_limited<R: std::io::Read>(pipe: Option<R>, limit: u64) -> String {
    let Some(pipe) = pipe else {
        return String::new();
    };
    let mut bytes = Vec::new();
    let _ = pipe.take(limit).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn arg_str(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn arg_bool(arguments: &Value, key: &str) -> bool {
    arguments.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn arg_strings(arguments: &Value, key: &str) -> Vec<String> {
    arguments
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn push_global(argv: &mut Vec<String>, arguments: &Value) {
    if let Some(project) = arg_str(arguments, "project") {
        argv.push("--project".to_string());
        argv.push(project);
    }
    if let Some(session) = arg_str(arguments, "session") {
        argv.push("--session".to_string());
        argv.push(session);
    }
    if let Some(profile) = arg_str(arguments, "output_profile") {
        argv.push("--output-profile".to_string());
        argv.push(profile);
    }
}

fn push_selector(argv: &mut Vec<String>, arguments: &Value) {
    for (key, flag) in [
        ("text", "--text"),
        ("contains", "--contains"),
        ("content_desc", "--content-desc"),
        ("resource_id", "--resource-id"),
        ("test_tag", "--test-tag"),
        ("class", "--class"),
        ("role", "--role"),
        ("text_regex", "--text-regex"),
        ("within_text", "--within"),
        ("above_text", "--above"),
        ("below_text", "--below"),
        ("left_of_text", "--left-of"),
        ("right_of_text", "--right-of"),
        ("region", "--region"),
        ("index", "--index"),
        ("ref", "--ref"),
        ("expect_token", "--expect-token"),
    ] {
        if let Some(value) = arguments.get(key) {
            let value = value
                .as_str()
                .map(str::to_string)
                .or_else(|| value.as_u64().map(|number| number.to_string()));
            if let Some(value) = value {
                argv.push(flag.to_string());
                argv.push(value);
            }
        }
    }
    for (key, flag) in [
        ("clickable", "--clickable"),
        ("enabled", "--enabled"),
        ("visible", "--visible"),
        ("ocr_fallback", "--ocr-fallback"),
        ("clear", "--clear"),
        ("hide_keyboard", "--hide-keyboard"),
    ] {
        if arguments.get(key).and_then(Value::as_bool).unwrap_or(false) {
            argv.push(flag.to_string());
        }
    }
}

fn tool_argv(name: &str, arguments: &Value) -> std::result::Result<Vec<String>, String> {
    let mut argv = Vec::new();
    match name {
        "androidforge_version" => {
            argv.push("--version".to_string());
            return Ok(argv);
        }
        "androidforge_doctor" => argv.extend(["doctor".to_string(), "--json".to_string()]),
        "androidforge_doctor_capacity" => {
            argv.extend(["doctor".to_string(), "--capacity".to_string()]);
            push_global(&mut argv, arguments);
            argv.push("--json".to_string());
        }
        // MCP build stays host-only: no --session/--serial and no
        // --install/--launch/--cold. Device mutations go through the existing
        // curated app tools, which retain their physical consent checks.
        "androidforge_build" => {
            argv.extend(["build".to_string(), "--json".to_string()]);
            if let Some(project) = arg_str(arguments, "project") {
                argv.push("--project".to_string());
                argv.push(project);
            }
            if arg_bool(arguments, "dry_run") {
                argv.push("--dry-run".to_string());
            }
            for (key, flag) in [
                ("variant", "--variant"),
                ("module", "--module"),
                ("timeout", "--timeout"),
                ("expected_package", "--expected-package"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
        }
        "androidforge_capabilities" => {
            argv.extend(["capabilities".to_string(), "--json".to_string()])
        }
        "androidforge_devices_list" => argv.extend([
            "devices".to_string(),
            "list".to_string(),
            "--json".to_string(),
        ]),
        "androidforge_context" => {
            argv.extend(["context".to_string(), "--json".to_string()]);
            push_global(&mut argv, arguments);
            if arg_bool(arguments, "refresh_screen") {
                argv.push("--refresh-screen".to_string());
            }
            if arg_bool(arguments, "no_screen") {
                argv.push("--no-screen".to_string());
            }
        }
        "androidforge_session_ensure" => {
            argv.extend([
                "session".to_string(),
                "ensure".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            for (key, flag) in [
                ("avd", "--avd"),
                ("serial", "--serial"),
                ("owner", "--owner"),
                ("ttl", "--ttl"),
                ("timeout", "--timeout"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
            if arg_bool(arguments, "headless") {
                argv.push("--headless".to_string());
            }
            if arg_bool(arguments, "wait") {
                argv.push("--wait".to_string());
            }
        }
        "androidforge_ui_observe" => {
            argv.extend([
                "ui".to_string(),
                "observe".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if arg_bool(arguments, "full") {
                argv.push("--full".to_string());
            }
            if arg_bool(arguments, "semantics") {
                argv.push("--semantics".to_string());
            }
            if let Some(package) = arg_str(arguments, "package") {
                argv.push("--package".to_string());
                argv.push(package);
            }
            if let Some(retries) = arguments.get("retries").and_then(Value::as_u64) {
                argv.push("--retries".to_string());
                argv.push(retries.to_string());
            }
        }
        "androidforge_ui_find" => {
            argv.extend(["ui".to_string(), "find".to_string(), "--json".to_string()]);
            push_global(&mut argv, arguments);
            push_selector(&mut argv, arguments);
        }
        "androidforge_ui_bounds" => {
            argv.extend(["ui".to_string(), "bounds".to_string(), "--json".to_string()]);
            push_global(&mut argv, arguments);
            push_selector(&mut argv, arguments);
        }
        "androidforge_ui_assert_bounds" => {
            argv.extend([
                "ui".to_string(),
                "assert-bounds".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            push_selector(&mut argv, arguments);
            for (key, flag) in [
                ("bottom_y", "--bottom-y"),
                ("bottom_y_gt", "--bottom-y-gt"),
                ("bottom_y_lt", "--bottom-y-lt"),
                ("tolerance", "--tolerance"),
            ] {
                if let Some(value) = arguments.get(key) {
                    let value = value
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| value.as_i64().map(|number| number.to_string()));
                    if let Some(value) = value {
                        argv.push(flag.to_string());
                        argv.push(value);
                    }
                }
            }
        }
        "androidforge_ui_tap" => {
            argv.extend(["ui".to_string(), "tap".to_string(), "--json".to_string()]);
            push_global(&mut argv, arguments);
            push_selector(&mut argv, arguments);
            for (key, flag) in [
                ("wait_for_text", "--wait-for-text"),
                ("wait_for_absent_text", "--wait-for-absent-text"),
                ("wait_for_package", "--wait-for-package"),
                ("wait_for_activity", "--wait-for-activity"),
                ("timeout", "--timeout"),
                ("x", "--x"),
                ("y", "--y"),
            ] {
                if let Some(value) = arguments.get(key) {
                    let value = value
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| value.as_i64().map(|number| number.to_string()));
                    if let Some(value) = value {
                        argv.push(flag.to_string());
                        argv.push(value);
                    }
                }
            }
        }
        "androidforge_ui_text" => {
            let text = arg_str(arguments, "text")
                .ok_or_else(|| "androidforge_ui_text requires text".to_string())?;
            argv.extend([
                "ui".to_string(),
                "text".to_string(),
                text,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            push_selector(&mut argv, arguments);
        }
        "androidforge_ui_dismiss_overlays" => {
            let dry_run = arg_bool(arguments, "dry_run");
            if !dry_run && !arg_bool(arguments, "confirm") {
                return Err(
                    "androidforge_ui_dismiss_overlays is a device-mutation command and requires confirm=true"
                        .to_string(),
                );
            }
            argv.extend([
                "ui".to_string(),
                "dismiss-overlays".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if !dry_run {
                argv.push("--yes".to_string());
            }
            if let Some(max) = arguments.get("max").and_then(Value::as_u64) {
                argv.push("--max".to_string());
                argv.push(max.to_string());
            }
            if dry_run {
                argv.push("--dry-run".to_string());
            }
        }
        "androidforge_capture_screenshot" => {
            argv.extend([
                "capture".to_string(),
                "screenshot".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            for (key, flag) in [
                ("label", "--label"),
                ("output", "--output"),
                ("compare", "--compare"),
                ("region", "--region"),
                ("diff_output", "--diff-output"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
            if let Some(value) = arguments.get("max_diff_ratio").and_then(Value::as_f64) {
                argv.push("--max-diff-ratio".to_string());
                argv.push(value.to_string());
            }
            if let Some(value) = arguments.get("diff_threshold").and_then(Value::as_u64) {
                argv.push("--diff-threshold".to_string());
                argv.push(value.to_string());
            }
        }
        "androidforge_diagnostics_frames" => {
            let package = arg_str(arguments, "package")
                .ok_or_else(|| "androidforge_diagnostics_frames requires package".to_string())?;
            argv.extend([
                "diagnostics".to_string(),
                "frames".to_string(),
                package,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if let Some(serial) = arg_str(arguments, "serial") {
                argv.extend(["--serial".to_string(), serial]);
            }
            for (key, flag) in [
                ("duration", "--duration"),
                ("timeout", "--timeout"),
                ("baseline", "--baseline"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
            for (key, flag) in [
                ("max_janky", "--max-janky"),
                ("max_p99", "--max-p99"),
                ("max_frame_ms", "--max-frame-ms"),
            ] {
                if let Some(value) = arguments.get(key).and_then(Value::as_f64) {
                    argv.push(flag.to_string());
                    argv.push(value.to_string());
                }
            }
            if arg_bool(arguments, "reset") {
                argv.push("--reset".to_string());
            }
            if arg_bool(arguments, "dry_run") {
                argv.push("--dry-run".to_string());
            }
        }
        "androidforge_capture_assert_color" => {
            let rgb = arg_str(arguments, "rgb")
                .ok_or_else(|| "androidforge_capture_assert_color requires rgb".to_string())?;
            argv.extend([
                "capture".to_string(),
                "assert-color".to_string(),
                "--rgb".to_string(),
                rgb,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if let Some(serial) = arg_str(arguments, "serial") {
                argv.extend(["--serial".to_string(), serial]);
            }
            for (key, flag) in [
                ("at", "--at"),
                ("region", "--region"),
                ("timeout", "--timeout"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
            if let Some(tolerance) = arguments.get("tolerance").and_then(Value::as_u64) {
                argv.extend(["--tolerance".to_string(), tolerance.to_string()]);
            }
            if arg_bool(arguments, "majority") {
                argv.push("--majority".to_string());
            }
            if arg_bool(arguments, "dry_run") {
                argv.push("--dry-run".to_string());
            }
        }
        "androidforge_capture_launch" => {
            let package = arg_str(arguments, "package")
                .ok_or_else(|| "androidforge_capture_launch requires package".to_string())?;
            argv.extend([
                "capture".to_string(),
                "launch".to_string(),
                package,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if let Some(serial) = arg_str(arguments, "serial") {
                argv.extend(["--serial".to_string(), serial]);
            }
            for (key, flag) in [
                ("activity", "--activity"),
                ("duration", "--duration"),
                ("expected_color", "--expected-color"),
                ("color_sample", "--color-sample"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
            for (key, flag) in [("fps", "--fps"), ("tolerance", "--tolerance")] {
                if let Some(value) = arguments.get(key).and_then(Value::as_u64) {
                    argv.push(flag.to_string());
                    argv.push(value.to_string());
                }
            }
            for (key, flag) in [
                ("ui_dump_package", "--ui-dump-package"),
                ("expect_first_frame", "--expect-first-frame"),
                ("no_splash", "--no-splash"),
                ("dry_run", "--dry-run"),
            ] {
                if arg_bool(arguments, key) {
                    argv.push(flag.to_string());
                }
            }
        }
        "androidforge_app_install" => {
            let apk = arg_str(arguments, "apk")
                .ok_or_else(|| "androidforge_app_install requires apk".to_string())?;
            if arg_bool(arguments, "fresh") && !arg_bool(arguments, "confirm") {
                return Err("fresh installs require confirm=true".to_string());
            }
            argv.extend([
                "app".to_string(),
                "install".to_string(),
                apk,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if arg_bool(arguments, "fresh") {
                argv.push("--fresh".to_string());
                argv.push("--yes".to_string());
            }
            if arg_bool(arguments, "only_if_changed") {
                argv.push("--only-if-changed".to_string());
            }
            if arg_bool(arguments, "allow_downgrade") {
                argv.push("--downgrade".to_string());
            }
            if arg_bool(arguments, "wait_for_confirmation") {
                argv.push("--wait-for-confirmation".to_string());
            }
            for text in arg_strings(arguments, "confirm_text") {
                argv.push("--confirm-text".to_string());
                argv.push(text);
            }
            if let Some(timeout) = arg_str(arguments, "confirmation_timeout") {
                argv.extend(["--confirmation-timeout".to_string(), timeout]);
            }
        }
        "androidforge_app_verify_installed" => {
            let apk = arg_str(arguments, "apk")
                .ok_or_else(|| "androidforge_app_verify_installed requires apk".to_string())?;
            argv.extend([
                "app".to_string(),
                "verify-installed".to_string(),
                "--apk".to_string(),
                apk,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            for split in arg_strings(arguments, "split") {
                argv.push("--split".to_string());
                argv.push(split);
            }
        }
        "androidforge_app_status" => {
            let package = arg_str(arguments, "package")
                .ok_or_else(|| "androidforge_app_status requires package".to_string())?;
            argv.extend([
                "app".to_string(),
                "status".to_string(),
                package,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            for (key, flag) in [
                ("expect_version_name", "--expect-version-name"),
                ("expect_version_code", "--expect-version-code"),
                ("expect_apk_sha256", "--expect-apk-sha256"),
                ("stable_for", "--stable-for"),
                ("timeout", "--timeout"),
            ] {
                if let Some(value) = arguments.get(key) {
                    let value = value
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| value.as_i64().map(|number| number.to_string()));
                    if let Some(value) = value {
                        argv.push(flag.to_string());
                        argv.push(value);
                    }
                }
            }
            for (key, flag) in [
                ("assert_task_exists", "--assert-task-exists"),
                ("assert_task_absent", "--assert-task-absent"),
                ("assert_process_stable", "--assert-process-stable"),
            ] {
                if arg_bool(arguments, key) {
                    argv.push(flag.to_string());
                }
            }
        }
        "androidforge_app_data" => {
            let action = arg_str(arguments, "action")
                .ok_or_else(|| "androidforge_app_data requires action".to_string())?;
            let package = arg_str(arguments, "package")
                .ok_or_else(|| "androidforge_app_data requires package".to_string())?;
            argv.extend([
                "app".to_string(),
                "data".to_string(),
                action.clone(),
                package,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            match action.as_str() {
                "export" => {
                    // Named --output wins over the legacy positional path; both
                    // go through the same safe resolver in the CLI.
                    if let Some(output) = arg_str(arguments, "output") {
                        argv.extend(["--output".to_string(), output]);
                    } else if let Some(path) = arg_str(arguments, "path") {
                        argv.push(path);
                    }
                    if arg_bool(arguments, "allow_in_project") {
                        argv.push("--allow-in-project".to_string());
                    }
                    if arg_bool(arguments, "overwrite") {
                        argv.push("--overwrite".to_string());
                    }
                }
                "import" => {
                    if let Some(path) = arg_str(arguments, "path") {
                        argv.push(path);
                    }
                }
                "reset" => {
                    if let Some(baseline) = arg_str(arguments, "baseline") {
                        argv.push("--baseline".to_string());
                        argv.push(baseline);
                    }
                    if let Some(backup) = arg_str(arguments, "backup_current") {
                        argv.push("--backup-current".to_string());
                        argv.push(backup);
                    }
                }
                "seed" => {
                    let file = arg_str(arguments, "file")
                        .ok_or_else(|| "seed requires file".to_string())?;
                    let remote = arg_str(arguments, "remote")
                        .ok_or_else(|| "seed requires remote".to_string())?;
                    argv.extend(["--file".to_string(), file, "--remote".to_string(), remote]);
                }
                other => return Err(format!("unsupported app data action '{other}'")),
            }
            if action != "export" {
                if !arg_bool(arguments, "confirm") {
                    return Err(format!("app data {action} requires confirm=true"));
                }
                argv.push("--yes".to_string());
            }
        }
        "androidforge_maintenance_clean" => {
            let keep_tags = arg_strings(arguments, "keep_tag");
            let dry_run = arg_bool(arguments, "dry_run");
            let confirm = arg_bool(arguments, "confirm");
            // A tag-protected call without confirmation is a deliberate dry
            // run; every other destructive call needs an explicit confirm.
            if !dry_run && !confirm && keep_tags.is_empty() {
                return Err(
                    "androidforge_maintenance_clean requires confirm=true or dry_run=true"
                        .to_string(),
                );
            }
            argv.extend([
                "maintenance".to_string(),
                "clean".to_string(),
                "--json".to_string(),
            ]);
            if let Some(project) = arg_str(arguments, "project") {
                argv.extend(["--project".to_string(), project]);
            }
            if let Some(older_than) = arg_str(arguments, "older_than") {
                argv.extend(["--older-than".to_string(), older_than]);
            }
            if let Some(keep_last) = arguments.get("keep_last").and_then(Value::as_u64) {
                argv.extend(["--keep-last".to_string(), keep_last.to_string()]);
            }
            for tag in keep_tags {
                argv.push("--keep-tag".to_string());
                argv.push(tag);
            }
            for (key, flag) in [("sessions", "--sessions"), ("runs", "--runs")] {
                if arg_bool(arguments, key) {
                    argv.push(flag.to_string());
                }
            }
            if confirm {
                argv.push("--yes".to_string());
            }
            if dry_run {
                argv.push("--dry-run".to_string());
            }
        }
        "androidforge_ui_drag" => {
            argv.extend(["ui".to_string(), "drag".to_string(), "--json".to_string()]);
            push_global(&mut argv, arguments);
            for (key, flag) in [
                ("from_x", "--from-x"),
                ("from_y", "--from-y"),
                ("to_x", "--to-x"),
                ("to_y", "--to-y"),
                ("from_text", "--from-text"),
                ("to_text", "--to-text"),
                ("from_id", "--from-id"),
                ("to_id", "--to-id"),
                ("from", "--from"),
                ("to", "--to"),
                ("duration_ms", "--duration"),
                ("hold_before_ms", "--hold-before"),
                ("hold_after_ms", "--hold-after"),
                ("capture_mid", "--capture-mid"),
            ] {
                if let Some(value) = arguments.get(key) {
                    let value = value
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| value.as_i64().map(|number| number.to_string()));
                    if let Some(value) = value {
                        argv.push(flag.to_string());
                        argv.push(value);
                    }
                }
            }
        }
        "androidforge_ui_wait" => {
            argv.extend(["ui".to_string(), "wait".to_string(), "--json".to_string()]);
            push_global(&mut argv, arguments);
            push_selector(&mut argv, arguments);
            if arg_bool(arguments, "not_exists") {
                argv.push("--not-exists".to_string());
            }
            for text in arg_strings(arguments, "for_text_gone") {
                argv.push("--for-text-gone".to_string());
                argv.push(text);
            }
            if arg_bool(arguments, "for_idle") {
                argv.push("--for-idle".to_string());
            }
            if arg_bool(arguments, "for_stable") {
                argv.push("--for-stable".to_string());
            }
            if arg_bool(arguments, "for_bounds_stable") {
                argv.push("--for-bounds-stable".to_string());
            }
            for (key, flag) in [("stable_for", "--stable-for"), ("timeout", "--timeout")] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
        }
        "androidforge_ui_assert_no_text" => {
            argv.extend([
                "ui".to_string(),
                "assert-no-text".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            for (key, flag) in [
                ("text", "--text"),
                ("contains", "--contains"),
                ("timeout", "--timeout"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
        }
        "androidforge_network_reverse" => {
            let action = arg_str(arguments, "action")
                .ok_or_else(|| "androidforge_network_reverse requires action".to_string())?;
            argv.extend([
                "network".to_string(),
                "reverse".to_string(),
                action.clone(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if action == "add" {
                let remote = arg_str(arguments, "remote")
                    .ok_or_else(|| "network reverse add requires remote".to_string())?;
                argv.push(remote);
                if let Some(local) = arg_str(arguments, "local") {
                    argv.push("--local".to_string());
                    argv.push(local);
                }
            } else if action == "remove" {
                let remote = arg_str(arguments, "remote")
                    .ok_or_else(|| "network reverse remove requires remote".to_string())?;
                argv.push(remote);
            }
        }
        "androidforge_network_mock_serve" => {
            let scenario = arg_str(arguments, "scenario");
            let max_requests = arguments
                .get("max_requests")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if scenario.is_none() && max_requests == 0 && arg_str(arguments, "duration").is_none() {
                return Err(
                    "androidforge_network_mock_serve requires max_requests > 0 or duration unless a scenario is set"
                        .to_string(),
                );
            }
            argv.extend([
                "network".to_string(),
                "mock".to_string(),
                "serve".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if let Some(scenario) = scenario {
                argv.push("--scenario".to_string());
                argv.push(scenario);
            }
            for (key, flag) in [
                ("bind", "--bind"),
                ("routes_file", "--routes"),
                ("default_body", "--default-body"),
                ("tls_cert", "--tls-cert"),
                ("tls_key", "--tls-key"),
                ("duration", "--duration"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
            for (key, flag) in [
                ("port", "--port"),
                ("default_status", "--default-status"),
                ("delay_ms", "--delay-ms"),
                ("max_requests", "--max-requests"),
            ] {
                if let Some(value) = arguments.get(key).and_then(Value::as_u64) {
                    argv.push(flag.to_string());
                    argv.push(value.to_string());
                }
            }
            for route in arg_strings(arguments, "route") {
                argv.push("--route".to_string());
                argv.push(route);
            }
            if arg_bool(arguments, "reverse") {
                argv.push("--reverse".to_string());
            }
        }
        "androidforge_network_mock_scenarios" => {
            argv.extend([
                "network".to_string(),
                "mock".to_string(),
                "scenarios".to_string(),
                "--json".to_string(),
            ]);
        }
        "androidforge_fixture_apply" | "androidforge_fixture_reset" => {
            let fixture =
                arg_str(arguments, "name").ok_or_else(|| format!("{name} requires name"))?;
            if !arg_bool(arguments, "confirm") {
                return Err(format!("{name} requires confirm=true"));
            }
            let action = if name == "androidforge_fixture_apply" {
                "apply"
            } else {
                "reset"
            };
            argv.extend([
                "fixture".to_string(),
                action.to_string(),
                fixture,
                "--yes".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if let Some(package) = arg_str(arguments, "package") {
                argv.push("--package".to_string());
                argv.push(package);
            }
        }
        "androidforge_scenario_run" => {
            let file = arg_str(arguments, "file")
                .ok_or_else(|| "androidforge_scenario_run requires file".to_string())?;
            let dry_run = arg_bool(arguments, "dry_run");
            if !dry_run && !arg_bool(arguments, "confirm") {
                return Err(
                    "androidforge_scenario_run is a device-mutation command and requires confirm=true or dry_run=true"
                        .to_string(),
                );
            }
            argv.extend(["scenario".to_string(), "run".to_string(), file]);
            push_global(&mut argv, arguments);
            if let Some(repeat) = arguments.get("repeat").and_then(Value::as_u64) {
                argv.extend(["--repeat".to_string(), repeat.to_string()]);
            }
            if !dry_run {
                argv.push("--yes".to_string());
            }
            if dry_run {
                argv.push("--dry-run".to_string());
            }
            argv.push("--json".to_string());
        }
        "androidforge_scenario_validate" => {
            let file = arg_str(arguments, "file")
                .ok_or_else(|| "androidforge_scenario_validate requires file".to_string())?;
            argv.extend([
                "scenario".to_string(),
                "validate".to_string(),
                file,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
        }
        "androidforge_session_list" => {
            argv.extend([
                "session".to_string(),
                "list".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            for (key, flag) in [("active", "--active"), ("mine", "--mine")] {
                if arg_bool(arguments, key) {
                    argv.push(flag.to_string());
                }
            }
            if let Some(owner) = arg_str(arguments, "owner") {
                argv.push("--owner".to_string());
                argv.push(owner);
            }
        }
        "androidforge_app_uninstall" => {
            let package = arg_str(arguments, "package")
                .ok_or_else(|| "androidforge_app_uninstall requires package".to_string())?;
            if !arg_bool(arguments, "confirm") {
                return Err("uninstall requires confirm=true".to_string());
            }
            argv.extend([
                "app".to_string(),
                "uninstall".to_string(),
                package,
                "--yes".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if arg_bool(arguments, "keep_data") {
                argv.push("--keep-data".to_string());
            }
        }
        "androidforge_fixture_run" => {
            let fixture = arg_str(arguments, "name")
                .ok_or_else(|| "androidforge_fixture_run requires name".to_string())?;
            argv.extend([
                "fixture".to_string(),
                "run".to_string(),
                fixture,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if arg_bool(arguments, "confirm") {
                argv.push("--yes".to_string());
            }
            if let Some(phases) = arguments.get("phases").and_then(Value::as_array) {
                for phase in phases {
                    if let Some(phase) = phase.as_str() {
                        argv.push("--phase".to_string());
                        argv.push(phase.to_string());
                    }
                }
            }
            if let Some(vars) = arguments.get("vars").and_then(Value::as_array) {
                for value in vars {
                    if let Some(value) = value.as_str() {
                        argv.push("--var".to_string());
                        argv.push(value.to_string());
                    }
                }
            }
            if let Some(apk) = arg_str(arguments, "apk") {
                argv.push("--apk".to_string());
                argv.push(apk);
            }
        }
        "androidforge_app_launch" => {
            let package = arg_str(arguments, "package")
                .ok_or_else(|| "androidforge_app_launch requires package".to_string())?;
            argv.extend([
                "app".to_string(),
                "launch".to_string(),
                package,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if let Some(activity) = arg_str(arguments, "activity") {
                argv.push("--activity".to_string());
                argv.push(activity);
            }
            for (key, flag) in [
                ("component", "--component"),
                ("action", "--action"),
                ("data", "--data"),
                ("mime_type", "--mime-type"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
            for key in ["categories", "extras", "flags"] {
                if let Some(values) = arguments.get(key).and_then(Value::as_array) {
                    let flag = match key {
                        "categories" => "--category",
                        "extras" => "--extra",
                        _ => "--flag",
                    };
                    for value in values {
                        if let Some(value) = value.as_str() {
                            argv.push(flag.to_string());
                            argv.push(value.to_string());
                        }
                    }
                }
            }
            for (key, flag) in [
                ("wait", "--wait"),
                ("force_stop", "--force-stop"),
                ("no_wake", "--no-wake"),
            ] {
                if arg_bool(arguments, key) {
                    argv.push(flag.to_string());
                }
            }
        }
        "androidforge_app_broadcast" => {
            let action = arg_str(arguments, "action")
                .ok_or_else(|| "androidforge_app_broadcast requires action".to_string())?;
            argv.extend([
                "app".to_string(),
                "broadcast".to_string(),
                "--action".to_string(),
                action,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if let Some(package) = arg_str(arguments, "package") {
                argv.push("--package".to_string());
                argv.push(package);
            }
            for (key, flag) in [
                ("data", "--data"),
                ("mime_type", "--mime-type"),
                ("receiver_permission", "--receiver-permission"),
                ("timeout", "--timeout"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
            for key in ["categories", "extras", "flags"] {
                if let Some(values) = arguments.get(key).and_then(Value::as_array) {
                    let flag = match key {
                        "categories" => "--category",
                        "extras" => "--extra",
                        _ => "--flag",
                    };
                    for value in values {
                        if let Some(value) = value.as_str() {
                            argv.push(flag.to_string());
                            argv.push(value.to_string());
                        }
                    }
                }
            }
            if arg_bool(arguments, "asynchronous") {
                argv.push("--asynchronous".to_string());
            }
        }
        "androidforge_app_service" => {
            argv.extend([
                "app".to_string(),
                "service".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            for (key, flag) in [
                ("package", "--package"),
                ("service", "--service"),
                ("component", "--component"),
                ("action", "--action"),
                ("data", "--data"),
                ("timeout", "--timeout"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
            if let Some(values) = arguments.get("extras").and_then(Value::as_array) {
                for value in values {
                    if let Some(value) = value.as_str() {
                        argv.push("--extra".to_string());
                        argv.push(value.to_string());
                    }
                }
            }
            if arg_bool(arguments, "stop") {
                argv.push("--stop".to_string());
            }
        }
        "androidforge_diagnostics_performance" => {
            let dry_run = arg_bool(arguments, "dry_run");
            if !dry_run && !arg_bool(arguments, "confirm") {
                return Err(
                    "androidforge_diagnostics_performance requires confirm=true or dry_run=true"
                        .to_string(),
                );
            }
            argv.extend(["diagnostics".to_string(), "performance".to_string()]);
            // `package` is an optional positional, so it must precede flags.
            if let Some(package) = arg_str(arguments, "package") {
                argv.push(package);
            }
            argv.push("--json".to_string());
            push_global(&mut argv, arguments);
            if let Some(serial) = arg_str(arguments, "serial") {
                argv.push("--serial".to_string());
                argv.push(serial);
            }
            if dry_run {
                argv.push("--dry-run".to_string());
            }
            for (key, flag) in [
                ("activity", "--activity"),
                ("settle", "--settle"),
                ("timeout", "--timeout"),
                ("frame_window", "--frame-window"),
                ("warmup", "--warmup"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
            if let Some(min_frames) = arguments.get("min_frames").and_then(Value::as_u64) {
                argv.push("--min-frames".to_string());
                argv.push(min_frames.to_string());
            }
            for (key, flag) in [
                ("cold", "--cold"),
                ("wake", "--wake"),
                ("no_screen_check", "--no-screen-check"),
            ] {
                if arg_bool(arguments, key) {
                    argv.push(flag.to_string());
                }
            }
        }
        "androidforge_devices_shell" => {
            argv.extend(["devices".to_string(), "shell".to_string()]);
            push_global(&mut argv, arguments);
            // Defense in depth: the read-only form is the only MCP surface.
            argv.push("--read-only".to_string());
            for (key, flag) in [("allow", "--allow"), ("deny", "--deny")] {
                for value in arg_strings(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
            if let Some(timeout) = arg_str(arguments, "timeout") {
                argv.push("--timeout".to_string());
                argv.push(timeout);
            }
            if arg_bool(arguments, "dry_run") {
                argv.push("--dry-run".to_string());
            }
            let command = arg_strings(arguments, "command");
            if command.is_empty() {
                return Err(
                    "androidforge_devices_shell requires a non-empty command array".to_string(),
                );
            }
            // `--json` must precede `--`; everything after it is the exact
            // remote argv and must not be polluted by global flags.
            argv.push("--json".to_string());
            argv.push("--".to_string());
            argv.extend(command);
        }
        "androidforge_devices_input_tap" => {
            argv.extend([
                "devices".to_string(),
                "input".to_string(),
                "tap".to_string(),
            ]);
            push_global(&mut argv, arguments);
            let x = arguments
                .get("x")
                .and_then(Value::as_i64)
                .ok_or_else(|| "androidforge_devices_input_tap requires x".to_string())?;
            let y = arguments
                .get("y")
                .and_then(Value::as_i64)
                .ok_or_else(|| "androidforge_devices_input_tap requires y".to_string())?;
            argv.push(x.to_string());
            argv.push(y.to_string());
            if arg_bool(arguments, "dry_run") {
                argv.push("--dry-run".to_string());
            }
            argv.push("--json".to_string());
        }
        "androidforge_diagnostics_logcat_query" => {
            argv.extend([
                "diagnostics".to_string(),
                "logcat".to_string(),
                "query".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            for (key, flag) in [
                ("package", "--package"),
                ("level", "--level"),
                ("since", "--since"),
                ("until", "--until"),
                ("grep", "--grep"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
            for (key, flag) in [
                ("lines", "--lines"),
                ("limit", "--limit"),
                ("retries", "--retries"),
            ] {
                if let Some(value) = arguments.get(key).and_then(Value::as_u64) {
                    argv.push(flag.to_string());
                    argv.push(value.to_string());
                }
            }
            for value in arg_strings(arguments, "redact_patterns") {
                argv.push("--redact-patterns".to_string());
                argv.push(value);
            }
        }
        "androidforge_diagnostics_logcat_watch" => {
            argv.extend([
                "diagnostics".to_string(),
                "logcat".to_string(),
                "watch".to_string(),
            ]);
            push_global(&mut argv, arguments);
            for (key, flag) in [
                ("package", "--package"),
                ("level", "--level"),
                ("grep", "--grep"),
                ("timeout", "--timeout"),
            ] {
                if let Some(value) = arg_str(arguments, key) {
                    argv.push(flag.to_string());
                    argv.push(value);
                }
            }
            if let Some(value) = arguments.get("max_events").and_then(Value::as_u64) {
                argv.push("--max-events".to_string());
                argv.push(value.to_string());
            }
            for (key, flag) in [("crash", "--crash"), ("anr", "--anr")] {
                if arg_bool(arguments, key) {
                    argv.push(flag.to_string());
                }
            }
            for value in arg_strings(arguments, "redact_patterns") {
                argv.push("--redact-patterns".to_string());
                argv.push(value);
            }
        }
        "androidforge_network_proxy_get" => {
            argv.extend([
                "network".to_string(),
                "proxy".to_string(),
                "get".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
        }
        "androidforge_network_proxy_set" => {
            let proxy = arg_str(arguments, "proxy")
                .ok_or_else(|| "androidforge_network_proxy_set requires proxy".to_string())?;
            argv.extend([
                "network".to_string(),
                "proxy".to_string(),
                "set".to_string(),
                proxy,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
        }
        "androidforge_network_proxy_clear" => {
            argv.extend([
                "network".to_string(),
                "proxy".to_string(),
                "clear".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
        }
        "androidforge_network_profile_list" => {
            argv.extend([
                "network".to_string(),
                "profile".to_string(),
                "list".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
        }
        "androidforge_network_profile_apply" => {
            let name = arg_str(arguments, "name")
                .ok_or_else(|| "androidforge_network_profile_apply requires name".to_string())?;
            argv.extend([
                "network".to_string(),
                "profile".to_string(),
                "apply".to_string(),
                name,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
        }
        "androidforge_network_restore" => {
            argv.extend([
                "network".to_string(),
                "restore".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
        }
        "androidforge_run_checkpoint" => {
            let name = arg_str(arguments, "name")
                .ok_or_else(|| "androidforge_run_checkpoint requires name".to_string())?;
            argv.extend([
                "run".to_string(),
                "checkpoint".to_string(),
                name,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if let Some(note) = arg_str(arguments, "note") {
                argv.push("--note".to_string());
                argv.push(note);
            }
            if let Some(run) = arg_str(arguments, "run") {
                argv.push("--run".to_string());
                argv.push(run);
            }
        }
        "androidforge_run_resume" => {
            let run = arg_str(arguments, "run")
                .ok_or_else(|| "androidforge_run_resume requires run".to_string())?;
            argv.extend([
                "run".to_string(),
                "resume".to_string(),
                run,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if let Some(checkpoint) = arg_str(arguments, "from_checkpoint") {
                argv.push("--from-checkpoint".to_string());
                argv.push(checkpoint);
            }
        }
        "androidforge_run_events" => {
            let run = arg_str(arguments, "run")
                .ok_or_else(|| "androidforge_run_events requires run".to_string())?;
            argv.extend([
                "run".to_string(),
                "events".to_string(),
                run,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
        }
        "androidforge_report_repro" => {
            let run = arg_str(arguments, "run")
                .ok_or_else(|| "androidforge_report_repro requires run".to_string())?;
            argv.extend([
                "report".to_string(),
                "repro".to_string(),
                run,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if let Some(output) = arg_str(arguments, "output") {
                argv.push("--output".to_string());
                argv.push(output);
            }
        }
        "androidforge_report_explain" => {
            let run = arg_str(arguments, "run")
                .ok_or_else(|| "androidforge_report_explain requires run".to_string())?;
            argv.extend([
                "report".to_string(),
                "explain".to_string(),
                run,
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            for tag in arg_strings(arguments, "tag") {
                argv.push("--tag".to_string());
                argv.push(tag);
            }
            if let Some(since) = arg_str(arguments, "since") {
                argv.push("--since".to_string());
                argv.push(since);
            }
        }
        other => {
            let catalog = crate::commands::catalog::command_catalog();
            let Some(command) = catalog.iter().find(|command| command.tool_name == other) else {
                return Err(format!("unknown AndroidForge tool: {other}"));
            };
            // Defense in depth: even if a hidden name reaches this fallback,
            // never build argv for an unsafe or long-running catalog entry.
            if command.safety == "unsafe" || command.safety == "long-running" {
                return Err(format!(
                    "AndroidForge tool '{other}' is not exposed over MCP ({})",
                    command.safety
                ));
            }
            append_generated_argv(&mut argv, arguments, command)?;
        }
    }
    Ok(argv)
}

fn append_generated_argv(
    argv: &mut Vec<String>,
    arguments: &Value,
    command: &crate::commands::catalog::CatalogCommand,
) -> std::result::Result<(), String> {
    argv.extend(command.path.split_whitespace().map(str::to_string));
    push_global(argv, arguments);
    if !serial_is_array(command)
        && let Some(serial) = arg_str(arguments, "serial")
    {
        argv.push("--serial".to_string());
        argv.push(serial);
    }
    let mut positionals = Vec::new();
    for argument in &command.arguments {
        let key = argument.long.clone().unwrap_or_else(|| argument.id.clone());
        if is_stripped_global(&key) && !(key == "serial" && serial_is_array(command)) {
            continue;
        }
        let value = arguments.get(&key);
        let required = argument.required;
        if value.is_none() {
            if required {
                return Err(format!("{} requires '{}'", command.tool_name, key));
            }
            continue;
        }
        let flag = if argument.positional {
            None
        } else if let Some(long) = &argument.long {
            Some(format!("--{long}"))
        } else {
            argument.short.map(|short| format!("-{short}"))
        };
        match argument.kind.as_str() {
            "boolean" => {
                if value.and_then(Value::as_bool).unwrap_or(false)
                    && let Some(flag) = flag
                {
                    argv.push(flag);
                }
            }
            "array" => {
                let values = match value {
                    Some(Value::Array(values)) => values.clone(),
                    Some(value) => vec![value.clone()],
                    None => Vec::new(),
                };
                for value in values {
                    let text = value_to_arg(&value).ok_or_else(|| {
                        format!(
                            "{} argument '{}' must be a string, number or boolean",
                            command.tool_name, key
                        )
                    })?;
                    if let Some(flag) = &flag {
                        argv.push(flag.clone());
                    }
                    if argument.positional {
                        positionals.push(text);
                    } else {
                        argv.push(text);
                    }
                }
            }
            _ => {
                let Some(value) = value else { continue };
                let text = value_to_arg(value).ok_or_else(|| {
                    format!(
                        "{} argument '{}' must be a string, number or boolean",
                        command.tool_name, key
                    )
                })?;
                if argument.positional {
                    positionals.push(text);
                } else if let Some(flag) = flag {
                    argv.push(flag);
                    argv.push(text);
                } else {
                    positionals.push(text);
                }
            }
        }
    }
    if command.safety == "destructive" || command.safety == "device-mutation" {
        if !arg_bool(arguments, "confirm") {
            return Err(format!(
                "{} is a {} command and requires confirm=true",
                command.tool_name, command.safety
            ));
        }
        argv.push("--yes".to_string());
    }
    if arg_bool(arguments, "dry_run") {
        argv.push("--dry-run".to_string());
    }
    argv.push("--json".to_string());
    if !positionals.is_empty() {
        // Keep a positional that starts with '-' from being interpreted as a
        // global flag. Flags are emitted before this delimiter.
        argv.push("--".to_string());
        argv.extend(positionals);
    }
    Ok(())
}

fn value_to_arg(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn read_resource(id: Value, params: &Value) -> Value {
    let Some(uri) = params.get("uri").and_then(Value::as_str) else {
        return error_response(id, -32602, "resources/read requires a uri");
    };
    let (mime, text) = match uri {
        "androidforge://tools" => (
            "application/json",
            serde_json::to_string_pretty(&json!({"tools": tool_definitions()})).unwrap_or_default(),
        ),
        "androidforge://catalog" => (
            "application/json",
            serde_json::to_string_pretty(&crate::commands::catalog::catalog()).unwrap_or_default(),
        ),
        "androidforge://schemas" => {
            let names = crate::commands::doctor::schema_files()
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>();
            (
                "application/json",
                serde_json::to_string_pretty(&json!({"schemas": names})).unwrap_or_default(),
            )
        }
        "androidforge://capabilities" => {
            let (_, stdout, stderr) = run_cli(&["capabilities".to_string(), "--json".to_string()]);
            if stdout.trim().is_empty() {
                ("text/plain", stderr)
            } else {
                ("application/json", stdout)
            }
        }
        "androidforge://context" => {
            let (_, stdout, stderr) = run_cli(&["context".to_string(), "--json".to_string()]);
            if stdout.trim().is_empty() {
                ("text/plain", stderr)
            } else {
                ("application/json", stdout)
            }
        }
        "androidforge://version" => (
            "application/json",
            serde_json::to_string_pretty(&json!({
                "version": env!("CARGO_PKG_VERSION"),
                "tool": "androidforge",
                "report_schema": 1
            }))
            .unwrap_or_default(),
        ),
        other if other.starts_with("androidforge://schema/") => {
            let name = other.trim_start_matches("androidforge://schema/");
            let Some((_, text)) = crate::commands::doctor::schema_files()
                .into_iter()
                .find(|(candidate, _)| *candidate == name)
            else {
                return error_response(id, -32602, &format!("unknown schema resource: {name}"));
            };
            ("application/json", text.to_string())
        }
        other => {
            return error_response(id, -32602, &format!("unknown resource: {other}"));
        }
    };
    result_response(
        id,
        json!({
            "contents": [{"uri": uri, "mimeType": mime, "text": text}]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_stdin_line_is_discarded_and_stream_resynchronizes() {
        const MAX: usize = 32;
        let first = b"{\"id\":1}\n";
        let oversized = vec![b'x'; MAX + 100];
        let second = b"{\"id\":2}\n";
        let mut stream = Vec::new();
        stream.extend_from_slice(first);
        stream.extend_from_slice(&oversized);
        stream.push(b'\n');
        stream.extend_from_slice(second);

        // A deliberately tiny BufReader forces the discard loop to cross
        // several fill_buf boundaries before it finds the oversized newline.
        let mut reader = std::io::BufReader::with_capacity(16, std::io::Cursor::new(stream));
        let mut buffer = Vec::new();

        assert_eq!(
            read_request_line(&mut reader, &mut buffer, MAX).unwrap(),
            RequestLine::Ready
        );
        assert_eq!(buffer, first);
        assert_eq!(
            read_request_line(&mut reader, &mut buffer, MAX).unwrap(),
            RequestLine::Oversized
        );
        assert!(
            buffer.len() <= MAX + 1,
            "oversized input must not be retained"
        );
        assert_eq!(
            read_request_line(&mut reader, &mut buffer, MAX).unwrap(),
            RequestLine::Ready
        );
        assert_eq!(
            buffer, second,
            "the next request must not be desynchronized"
        );
        assert_eq!(
            read_request_line(&mut reader, &mut buffer, MAX).unwrap(),
            RequestLine::Eof
        );
    }

    #[test]
    fn oversized_line_with_newline_in_window_still_resynchronizes() {
        const MAX: usize = 16;
        let mut stream = vec![b'x'; MAX];
        stream.push(b'\n');
        stream.extend_from_slice(b"{\"id\":3}\n");
        let mut reader = std::io::BufReader::new(std::io::Cursor::new(stream));
        let mut buffer = Vec::new();

        assert_eq!(
            read_request_line(&mut reader, &mut buffer, MAX).unwrap(),
            RequestLine::Oversized
        );
        assert_eq!(
            read_request_line(&mut reader, &mut buffer, MAX).unwrap(),
            RequestLine::Ready
        );
        assert_eq!(buffer, b"{\"id\":3}\n");
        assert_eq!(
            read_request_line(&mut reader, &mut buffer, MAX).unwrap(),
            RequestLine::Eof
        );
    }

    #[test]
    fn unterminated_oversized_line_ends_cleanly() {
        const MAX: usize = 8;
        let stream = vec![b'x'; MAX + 50];
        let mut reader = std::io::BufReader::with_capacity(4, std::io::Cursor::new(stream));
        let mut buffer = Vec::new();

        assert_eq!(
            read_request_line(&mut reader, &mut buffer, MAX).unwrap(),
            RequestLine::Oversized
        );
        assert_eq!(
            read_request_line(&mut reader, &mut buffer, MAX).unwrap(),
            RequestLine::Eof
        );
    }

    #[test]
    fn every_tool_has_a_name_description_and_schema() {
        let tools = tool_definitions();
        assert!(tools.len() >= 12);
        for tool in tools {
            assert!(
                tool["name"]
                    .as_str()
                    .is_some_and(|name| name.starts_with("androidforge_"))
            );
            assert!(
                tool["description"]
                    .as_str()
                    .is_some_and(|text| !text.is_empty())
            );
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn doctor_capacity_tool_maps_to_read_only_cli_argv() {
        let argv = tool_argv(
            "androidforge_doctor_capacity",
            &json!({"project": "A:/proj", "output_profile": "brief"}),
        )
        .expect("capacity argv");
        assert_eq!(
            argv,
            vec![
                "doctor",
                "--capacity",
                "--project",
                "A:/proj",
                "--output-profile",
                "brief",
                "--json"
            ]
        );
        assert!(
            tool_definitions()
                .iter()
                .any(|tool| tool["name"] == "androidforge_doctor_capacity")
        );
    }

    #[test]
    fn observe_tool_forwards_semantics_but_never_presentation_flags() {
        let argv = tool_argv(
            "androidforge_ui_observe",
            &json!({"session": "ses-1", "full": true, "semantics": true}),
        )
        .unwrap();
        assert_eq!(argv[0], "ui");
        assert_eq!(argv[1], "observe");
        assert!(argv.iter().any(|arg| arg == "--semantics"));
        assert!(argv.iter().any(|arg| arg == "--full"));
        for forbidden in ["--jsonl", "--fields", "--stats"] {
            assert!(
                !argv.iter().any(|arg| arg == forbidden),
                "MCP argv contains {forbidden}: {argv:?}"
            );
        }
        let catalog = tool_definitions();
        let observe = catalog
            .iter()
            .find(|tool| tool["name"] == "androidforge_ui_observe")
            .expect("observe tool");
        assert!(observe["inputSchema"]["properties"]["semantics"].is_object());
        for forbidden in ["jsonl", "fields", "stats"] {
            assert!(
                observe["inputSchema"]["properties"]
                    .get(forbidden)
                    .is_none(),
                "MCP schema exposes {forbidden}"
            );
        }
    }

    #[test]
    fn bounds_and_wait_tools_map_to_read_only_cli_argv() {
        let bounds = tool_argv(
            "androidforge_ui_bounds",
            &json!({"session": "ses-1", "text": "Save"}),
        )
        .unwrap();
        assert_eq!(
            bounds,
            vec![
                "ui",
                "bounds",
                "--json",
                "--session",
                "ses-1",
                "--text",
                "Save"
            ]
        );

        let eq = tool_argv(
            "androidforge_ui_assert_bounds",
            &json!({
                "session": "ses-1",
                "text": "Save",
                "bottom_y": 300,
                "tolerance": 2,
            }),
        )
        .unwrap();
        assert_eq!(
            eq,
            vec![
                "ui",
                "assert-bounds",
                "--json",
                "--session",
                "ses-1",
                "--text",
                "Save",
                "--bottom-y",
                "300",
                "--tolerance",
                "2"
            ]
        );

        let gt = tool_argv(
            "androidforge_ui_assert_bounds",
            &json!({"session": "ses-1", "text": "Save", "bottom_y_gt": 299}),
        )
        .unwrap();
        assert!(
            gt.windows(2)
                .any(|pair| pair[0] == "--bottom-y-gt" && pair[1] == "299")
        );
        let lt = tool_argv(
            "androidforge_ui_assert_bounds",
            &json!({"session": "ses-1", "text": "Save", "bottom_y_lt": 301}),
        )
        .unwrap();
        assert!(
            lt.windows(2)
                .any(|pair| pair[0] == "--bottom-y-lt" && pair[1] == "301")
        );

        let wait = tool_argv(
            "androidforge_ui_wait",
            &json!({
                "session": "ses-1",
                "text": "Save",
                "for_bounds_stable": true,
                "stable_for": "1ms",
                "timeout": "5s",
            }),
        )
        .unwrap();
        assert_eq!(wait[0], "ui");
        assert_eq!(wait[1], "wait");
        for flag in ["--for-bounds-stable", "--stable-for", "--timeout"] {
            assert!(
                wait.iter().any(|arg| arg == flag),
                "missing {flag}: {wait:?}"
            );
        }
        assert!(wait.windows(2).any(|pair| pair == ["--stable-for", "1ms"]));
        assert!(wait.windows(2).any(|pair| pair == ["--timeout", "5s"]));
    }

    #[test]
    fn destructive_tools_require_confirmation() {
        let error = tool_argv(
            "androidforge_app_uninstall",
            &json!({"package": "com.example", "session": "ses-1"}),
        )
        .unwrap_err();
        assert!(error.contains("confirm=true"));
        let error = tool_argv(
            "androidforge_app_install",
            &json!({"apk": "app.apk", "session": "ses-1", "fresh": true}),
        )
        .unwrap_err();
        assert!(error.contains("confirm=true"));
    }

    #[test]
    fn new_v010_tools_map_to_cli_commands() {
        let launch = tool_argv(
            "androidforge_app_launch",
            &json!({
                "package": "com.example",
                "session": "ses-1",
                "action": "android.intent.action.VIEW",
                "data": "myapp://checkout/42",
                "categories": ["android.intent.category.BROWSABLE"],
                "extras": ["coupon:s:SAVE10"],
                "wait": true
            }),
        )
        .unwrap();
        assert_eq!(launch[0], "app");
        assert_eq!(launch[1], "launch");
        assert!(launch.contains(&"--action".to_string()));
        assert!(launch.contains(&"myapp://checkout/42".to_string()));
        assert!(launch.contains(&"--wait".to_string()));

        let query = tool_argv(
            "androidforge_diagnostics_logcat_query",
            &json!({"session": "ses-1", "grep": "FATAL", "level": "error", "lines": 500}),
        )
        .unwrap();
        assert!(query.contains(&"query".to_string()));
        assert!(query.contains(&"FATAL".to_string()));

        let checkpoint = tool_argv(
            "androidforge_run_checkpoint",
            &json!({"name": "after-login", "run": "run-1", "note": "ready"}),
        )
        .unwrap();
        assert_eq!(checkpoint[0], "run");
        assert_eq!(checkpoint[1], "checkpoint");
        assert!(checkpoint.contains(&"--run".to_string()));

        let proxy = tool_argv(
            "androidforge_network_proxy_set",
            &json!({"session": "ses-1", "proxy": "http://127.0.0.1:8080"}),
        )
        .unwrap();
        assert_eq!(proxy[0], "network");
        assert_eq!(proxy[1], "proxy");
        assert!(proxy.contains(&"http://127.0.0.1:8080".to_string()));

        let restore =
            tool_argv("androidforge_network_restore", &json!({"session": "ses-1"})).unwrap();
        assert!(restore.contains(&"restore".to_string()));
    }

    #[test]
    fn e2_fixture_state_and_scenario_tools_map_to_cli_argv() {
        let scenarios = tool_argv("androidforge_network_mock_scenarios", &json!({})).unwrap();
        assert_eq!(scenarios, vec!["network", "mock", "scenarios", "--json"]);

        assert!(
            tool_argv("androidforge_fixture_apply", &json!({"name": "state-demo"})).is_err(),
            "state mutation must require confirm=true"
        );
        let apply = tool_argv(
            "androidforge_fixture_apply",
            &json!({"name": "state-demo", "session": "ses-1", "confirm": true}),
        )
        .unwrap();
        assert_eq!(apply[0], "fixture");
        assert_eq!(apply[1], "apply");
        assert_eq!(apply[2], "state-demo");
        assert!(apply.contains(&"--yes".to_string()));

        let reset = tool_argv(
            "androidforge_fixture_reset",
            &json!({"name": "state-demo", "session": "ses-1", "confirm": true}),
        )
        .unwrap();
        assert_eq!(reset[1], "reset");
        assert!(reset.contains(&"--yes".to_string()));

        let served = tool_argv(
            "androidforge_network_mock_serve",
            &json!({"scenario": "offline", "port": 18081}),
        )
        .unwrap();
        assert!(served.contains(&"--scenario".to_string()));
        assert!(served.contains(&"offline".to_string()));
    }

    #[test]
    fn frames_and_capture_tools_map_to_cli_commands() {
        let frames = tool_argv(
            "androidforge_diagnostics_frames",
            &json!({
                "package": "com.example",
                "session": "ses-1",
                "reset": true,
                "duration": "5s",
                "baseline": "frames/report.json",
                "max_janky": 2.5,
                "max_p99": 5.0,
                "max_frame_ms": 10.0,
            }),
        )
        .unwrap();
        assert_eq!(frames[0], "diagnostics");
        assert_eq!(frames[1], "frames");
        assert_eq!(frames[2], "com.example");
        for flag in [
            "--reset",
            "--duration",
            "5s",
            "--baseline",
            "--max-janky",
            "--max-p99",
            "--max-frame-ms",
        ] {
            assert!(
                frames.contains(&flag.to_string()),
                "missing {flag} in {frames:?}"
            );
        }

        let assert_color = tool_argv(
            "androidforge_capture_assert_color",
            &json!({
                "rgb": "#1A73E8",
                "session": "ses-1",
                "at": "540,1200",
                "tolerance": 8,
            }),
        )
        .unwrap();
        assert_eq!(assert_color[0], "capture");
        assert_eq!(assert_color[1], "assert-color");
        assert!(assert_color.contains(&"#1A73E8".to_string()));
        assert!(assert_color.contains(&"--at".to_string()));
        assert!(assert_color.contains(&"540,1200".to_string()));
        assert!(assert_color.contains(&"--tolerance".to_string()));

        let launch = tool_argv(
            "androidforge_capture_launch",
            &json!({
                "package": "com.example",
                "session": "ses-1",
                "fps": 12,
                "duration": "5s",
                "expect_first_frame": true,
                "no_splash": true,
                "expected_color": "#1A73E8",
                "color_sample": "dominant",
                "tolerance": 8,
                "ui_dump_package": true,
            }),
        )
        .unwrap();
        assert_eq!(launch[0], "capture");
        assert_eq!(launch[1], "launch");
        assert_eq!(launch[2], "com.example");
        for flag in [
            "--fps",
            "12",
            "--duration",
            "--expect-first-frame",
            "--no-splash",
            "--expected-color",
            "--color-sample",
            "dominant",
            "--tolerance",
            "--ui-dump-package",
        ] {
            assert!(
                launch.contains(&flag.to_string()),
                "missing {flag} in {launch:?}"
            );
        }
        assert!(
            !launch.iter().any(|arg| arg.contains("allow-physical")),
            "capture launch must not forward a physical mutation flag"
        );
    }

    #[test]
    fn selector_arguments_are_forwarded() {
        let argv = tool_argv(
            "androidforge_ui_tap",
            &json!({
                "session": "ses-1",
                "text_regex": "Save|Apply",
                "role": "button",
                "ocr_fallback": true
            }),
        )
        .unwrap();
        assert!(argv.contains(&"--text-regex".to_string()));
        assert!(argv.contains(&"Save|Apply".to_string()));
        assert!(argv.contains(&"--role".to_string()));
        assert!(argv.contains(&"--ocr-fallback".to_string()));
    }

    #[test]
    fn physical_capability_flags_are_never_exposed_or_forwarded_by_mcp() {
        let physical_keys = [
            "allow_physical_mutation",
            "allow_physical_app_control",
            "allow_physical_data",
            "allow_physical_network",
            "allow_physical_install",
            "allow_physical_input",
        ];
        for tool in tool_definitions() {
            let name = tool["name"].as_str().unwrap_or_default();
            let properties = &tool["inputSchema"]["properties"];
            for key in physical_keys {
                assert!(properties.get(key).is_none(), "{name} advertises {key}");
            }
        }

        let mut arguments = json!({
            "package": "com.example",
            "confirm": true,
            "allow_physical_mutation": true,
            "allow_physical_app_control": true,
            "allow_physical_data": true,
            "allow_physical_network": true,
            "allow_physical_install": true,
            "allow_physical_input": true,
        });
        arguments["path"] = json!("backup.tar");
        arguments["name"] = json!("fixture");
        arguments["proxy"] = json!("http://127.0.0.1:8080");
        for tool in [
            "androidforge_app_launch",
            "androidforge_app_force_stop",
            "androidforge_app_uninstall",
            "androidforge_app_data_import",
            "androidforge_ui_tap",
            "androidforge_network_proxy_set",
            "androidforge_fixture_run",
            "androidforge_diagnostics_performance",
        ] {
            let argv = tool_argv(tool, &arguments)
                .unwrap_or_else(|error| panic!("{tool} should be dispatchable: {error}"));
            assert!(
                !argv
                    .iter()
                    .any(|argument| argument.starts_with("--allow-physical")),
                "{tool} forwarded a physical flag: {argv:?}"
            );
        }

        // A scoped flag must not bypass the existing confirm=true requirement.
        let error = tool_argv(
            "androidforge_app_force_stop",
            &json!({"package": "com.example", "allow_physical_app_control": true}),
        )
        .unwrap_err();
        assert!(
            error.contains("confirm=true"),
            "scoped flag bypassed MCP confirmation: {error}"
        );
    }

    #[test]
    fn report_prune_requires_confirmation_and_forwards_yes() {
        let error = tool_argv("androidforge_report_prune", &json!({})).unwrap_err();
        assert!(error.contains("confirm=true"), "{error}");
        let argv = tool_argv(
            "androidforge_report_prune",
            &json!({"confirm": true, "older_than": "7d"}),
        )
        .unwrap();
        assert_eq!(argv[0], "report");
        assert_eq!(argv[1], "prune");
        assert!(argv.contains(&"--yes".to_string()), "{argv:?}");
    }

    #[test]
    fn report_scope_and_budget_flags_are_exposed_in_mcp() {
        let definitions = tool_definitions();
        let explain = definitions
            .iter()
            .find(|tool| tool["name"] == "androidforge_report_explain")
            .expect("explain tool");
        assert_eq!(explain["inputSchema"]["properties"]["tag"]["type"], "array");
        assert_eq!(
            explain["inputSchema"]["properties"]["since"]["type"],
            "string"
        );
        let explain_argv = tool_argv(
            "androidforge_report_explain",
            &json!({
                "run": "latest-failure",
                "tag": ["release", "smoke"],
                "since": "2026-01-01T00:00:00Z",
                "session": "ses-1",
            }),
        )
        .unwrap();
        assert_eq!(
            explain_argv.iter().filter(|arg| *arg == "--tag").count(),
            2,
            "{explain_argv:?}"
        );
        assert!(explain_argv.contains(&"release".to_string()));
        assert!(explain_argv.contains(&"smoke".to_string()));
        assert!(explain_argv.contains(&"--since".to_string()));
        assert!(explain_argv.contains(&"2026-01-01T00:00:00Z".to_string()));

        let compare = definitions
            .iter()
            .find(|tool| tool["name"] == "androidforge_report_compare")
            .expect("compare tool");
        for key in [
            "tag",
            "since",
            "fail-on-regression",
            "budget",
            "max-startup-ms",
            "max-jank-percent",
            "max-p99-ms",
            "max-frame-ms",
            "max-apk-size-bytes",
            "max-failed-tests",
        ] {
            assert!(
                compare["inputSchema"]["properties"][key].is_object(),
                "compare schema missing {key}: {compare}"
            );
        }
        let compare_argv = tool_argv(
            "androidforge_report_compare",
            &json!({
                "run": "current",
                "baseline": "old",
                "tag": ["release"],
                "since": "30d",
                "fail-on-regression": true,
                "max-startup-ms": 600,
            }),
        )
        .unwrap();
        for flag in [
            "--tag",
            "--since",
            "--fail-on-regression",
            "--max-startup-ms",
        ] {
            assert!(compare_argv.contains(&flag.to_string()), "{compare_argv:?}");
        }

        let start = definitions
            .iter()
            .find(|tool| tool["name"] == "androidforge_run_start")
            .expect("run start tool");
        assert_eq!(start["inputSchema"]["properties"]["tag"]["type"], "array");
        assert_eq!(start["inputSchema"]["properties"]["meta"]["type"], "array");
        let start_argv = tool_argv(
            "androidforge_run_start",
            &json!({
                "name": "smoke",
                "tag": ["release"],
                "meta": ["build=42"],
            }),
        )
        .unwrap();
        assert!(start_argv.contains(&"--tag".to_string()));
        assert!(start_argv.contains(&"--meta".to_string()));
        assert!(start_argv.contains(&"build=42".to_string()));
    }

    #[test]
    fn scoped_export_and_retention_tools_map_to_cli_argv() {
        let definitions = tool_definitions();
        let app_data = definitions
            .iter()
            .find(|tool| tool["name"] == "androidforge_app_data")
            .expect("app data tool");
        for key in ["output", "allow_in_project", "overwrite"] {
            assert!(
                app_data["inputSchema"]["properties"][key].is_object(),
                "app data tool is missing {key}"
            );
        }
        let clean = definitions
            .iter()
            .find(|tool| tool["name"] == "androidforge_maintenance_clean")
            .expect("maintenance clean tool");
        assert_eq!(
            clean["inputSchema"]["properties"]["keep_tag"]["type"],
            "array"
        );
        assert!(
            clean["description"]
                .as_str()
                .unwrap_or_default()
                .contains("destructive")
        );
        for key in ["keep_tag", "dry_run", "confirm"] {
            assert!(
                clean["inputSchema"]["properties"][key].is_object(),
                "maintenance clean tool is missing {key}"
            );
        }

        let export = tool_argv(
            "androidforge_app_data",
            &json!({
                "action": "export",
                "package": "com.example.app",
                "session": "ses-1",
                "output": "backup.tar",
                "allow_in_project": true,
                "overwrite": true,
            }),
        )
        .unwrap();
        assert!(
            export
                .windows(2)
                .any(|pair| pair == ["--output", "backup.tar"])
        );
        assert!(export.contains(&"--allow-in-project".to_string()));
        assert!(export.contains(&"--overwrite".to_string()));
        assert!(!export.contains(&"--yes".to_string()));

        let legacy = tool_argv(
            "androidforge_app_data",
            &json!({
                "action": "export",
                "package": "com.example.app",
                "session": "ses-1",
                "path": "legacy.tar",
            }),
        )
        .unwrap();
        assert!(legacy.contains(&"legacy.tar".to_string()));
        assert!(!legacy.contains(&"--output".to_string()));

        let error =
            tool_argv("androidforge_maintenance_clean", &json!({"runs": true})).unwrap_err();
        assert!(error.contains("confirm=true"), "{error}");
        let tagged_preview = tool_argv(
            "androidforge_maintenance_clean",
            &json!({"runs": true, "keep_tag": ["release"]}),
        )
        .unwrap();
        assert!(tagged_preview.contains(&"--runs".to_string()));
        assert!(tagged_preview.contains(&"--keep-tag".to_string()));
        assert!(!tagged_preview.contains(&"--yes".to_string()));

        let applied = tool_argv(
            "androidforge_maintenance_clean",
            &json!({
                "runs": true,
                "keep_tag": ["release", "smoke"],
                "keep_last": 3,
                "older_than": "7d",
                "confirm": true,
            }),
        )
        .unwrap();
        assert_eq!(applied.iter().filter(|arg| *arg == "--keep-tag").count(), 2);
        assert!(applied.contains(&"release".to_string()));
        assert!(applied.contains(&"smoke".to_string()));
        assert!(applied.windows(2).any(|pair| pair == ["--keep-last", "3"]));
        assert!(
            applied
                .windows(2)
                .any(|pair| pair == ["--older-than", "7d"])
        );
        assert!(applied.contains(&"--yes".to_string()));
        assert!(
            !applied
                .iter()
                .any(|argument| argument.starts_with("--allow-physical")),
            "retention must stay host-only: {applied:?}"
        );
    }

    #[test]
    fn hidden_unsafe_and_long_running_tools_are_not_dispatchable() {
        for name in [
            "androidforge_adb",
            "androidforge_run_exec",
            "androidforge_mcp_serve",
        ] {
            let error = tool_argv(
                name,
                &json!({"args": ["shell", "id"], "name": "cmd", "program": "cmd"}),
            )
            .unwrap_err();
            assert!(
                error.contains("not exposed") || error.contains("unknown AndroidForge tool"),
                "{name}: {error}"
            );
        }
        let advertised = tool_definitions()
            .into_iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_string))
            .collect::<std::collections::HashSet<_>>();
        for hidden in [
            "androidforge_adb",
            "androidforge_run_exec",
            "androidforge_mcp_serve",
        ] {
            assert!(!advertised.contains(hidden), "{hidden} must stay hidden");
        }
    }

    #[test]
    fn call_tool_rejects_hidden_tools_before_argv() {
        let response = call_tool(
            json!(1),
            &json!({
                "name": "androidforge_adb",
                "arguments": {"args": ["shell", "id"]}
            }),
        );
        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("unknown AndroidForge tool")),
            "{response}"
        );
    }

    #[test]
    fn generated_tools_cover_every_non_blocking_cli_command() {
        let tools = tool_definitions()
            .into_iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_string))
            .collect::<std::collections::HashSet<_>>();
        for command in crate::commands::catalog::command_catalog() {
            if command.safety == "long-running" || command.safety == "unsafe" {
                continue;
            }
            assert!(
                tools.contains(&command.tool_name),
                "MCP catalog is missing {}",
                command.tool_name
            );
        }
    }

    #[test]
    fn generated_argv_mirrors_cli_options_and_positionals() {
        let argv = tool_argv(
            "androidforge_tools_status",
            &json!({"tool": "maestro", "session": "ses-1"}),
        )
        .unwrap();
        assert_eq!(argv[0], "tools");
        assert_eq!(argv[1], "status");
        assert!(argv.contains(&"maestro".to_string()));
        assert!(argv.contains(&"--json".to_string()));

        let inspect =
            tool_argv("androidforge_session_inspect", &json!({"session": "ses-1"})).unwrap();
        assert_eq!(inspect[0], "session");
        assert_eq!(inspect[1], "inspect");
        assert!(inspect.contains(&"--session".to_string()));
    }

    #[test]
    fn matrix_serial_array_is_forwarded() {
        let argv = tool_argv(
            "androidforge_agent_matrix",
            &json!({"serial": ["emulator-5554", "emulator-5556"], "confirm": true}),
        )
        .unwrap();
        assert_eq!(
            argv.iter()
                .filter(|argument| *argument == "--serial")
                .count(),
            2,
            "{argv:?}"
        );
        assert!(argv.contains(&"emulator-5554".to_string()));
        assert!(argv.contains(&"emulator-5556".to_string()));

        let single = tool_argv(
            "androidforge_agent_matrix",
            &json!({"serial": "emulator-5554", "confirm": true}),
        )
        .unwrap();
        assert_eq!(
            single
                .iter()
                .filter(|argument| *argument == "--serial")
                .count(),
            1,
            "{single:?}"
        );

        let tool = tool_definitions()
            .into_iter()
            .find(|tool| tool["name"] == "androidforge_agent_matrix")
            .expect("matrix tool");
        assert_eq!(
            tool["inputSchema"]["properties"]["serial"]["type"], "array",
            "{tool}"
        );
    }

    #[test]
    fn generated_mutating_tools_require_confirmation() {
        for (name, arguments) in [
            ("androidforge_session_stop", json!({"session": "ses-1"})),
            (
                "androidforge_app_data_restore",
                json!({"package": "com.example", "path": "backup.tar"}),
            ),
            ("androidforge_doze_unforce", json!({})),
            ("androidforge_test_maestro", json!({"flow": "flow.yaml"})),
            (
                "androidforge_test_instrumentation",
                json!({"package": "com.example.test"}),
            ),
            ("androidforge_snapshot_save", json!({"name": "snap"})),
            (
                "androidforge_standby_set",
                json!({"package": "com.example", "bucket": "active"}),
            ),
            ("androidforge_agent_test", json!({})),
            ("androidforge_agent_matrix", json!({})),
            ("androidforge_notifications_tap", json!({})),
            ("androidforge_ui_dismiss_overlays", json!({})),
        ] {
            let error = tool_argv(name, &arguments).unwrap_err();
            assert!(error.contains("confirm=true"), "{name}: {error}");
        }

        let argv = tool_argv(
            "androidforge_session_stop",
            &json!({"session": "ses-1", "confirm": true}),
        )
        .unwrap();
        assert!(argv.contains(&"--yes".to_string()));
    }

    #[test]
    fn diagnostics_performance_schema_and_confirmation_contract() {
        let tool = tool_definitions()
            .into_iter()
            .find(|tool| tool["name"] == "androidforge_diagnostics_performance")
            .expect("diagnostics performance tool");
        let properties = &tool["inputSchema"]["properties"];
        assert_eq!(properties["min_frames"]["type"], "integer");
        assert_eq!(properties["min_frames"]["default"], 30);
        assert_eq!(properties["min_frames"]["minimum"], 1);
        for key in ["wake", "no_screen_check", "cold", "confirm", "dry_run"] {
            assert_eq!(properties[key]["type"], "boolean", "{key}");
        }
        assert_eq!(
            properties["confirm"]["description"].as_str(),
            Some("Required unless dry_run is true.")
        );
        for key in ["frame_window", "warmup", "settle", "timeout"] {
            assert_eq!(properties[key]["type"], "string", "{key}");
        }
        for physical_key in [
            "allow_physical_mutation",
            "allow_physical_app_control",
            "allow_physical_data",
            "allow_physical_network",
            "allow_physical_install",
            "allow_physical_input",
        ] {
            assert!(
                properties.get(physical_key).is_none(),
                "diagnostics performance advertises {physical_key}"
            );
        }

        let command = crate::commands::catalog::command_catalog()
            .into_iter()
            .find(|command| command.tool_name == "androidforge_diagnostics_performance")
            .expect("catalog entry");
        assert_eq!(command.safety, "device-mutation");

        let error = tool_argv(
            "androidforge_diagnostics_performance",
            &json!({"session": "ses-1"}),
        )
        .unwrap_err();
        assert_eq!(
            error,
            "androidforge_diagnostics_performance requires confirm=true or dry_run=true"
        );

        let argv = tool_argv(
            "androidforge_diagnostics_performance",
            &json!({"session": "ses-1", "confirm": true}),
        )
        .unwrap();
        assert_eq!(argv[0], "diagnostics");
        assert_eq!(argv[1], "performance");
        assert!(!argv.contains(&"--yes".to_string()), "{argv:?}");
        assert!(!argv.contains(&"--wake".to_string()), "{argv:?}");
        assert!(!argv.contains(&"--no-screen-check".to_string()), "{argv:?}");
        assert!(
            !argv.iter().any(|arg| arg.starts_with("--allow-physical")),
            "diagnostics performance forwarded a physical flag: {argv:?}"
        );

        let dry = tool_argv(
            "androidforge_diagnostics_performance",
            &json!({"session": "ses-1", "dry_run": true}),
        )
        .unwrap();
        assert!(dry.contains(&"--dry-run".to_string()), "{dry:?}");
        assert!(
            !dry.iter().any(|arg| arg.starts_with("--allow-physical")),
            "dry-run diagnostics performance forwarded a physical flag: {dry:?}"
        );
    }

    #[test]
    fn diagnostics_performance_argv_forwards_guard_fields() {
        let argv = tool_argv(
            "androidforge_diagnostics_performance",
            &json!({
                "package": "com.example",
                "activity": ".MainActivity",
                "session": "ses-1",
                "confirm": true,
                "cold": true,
                "settle": "1s",
                "timeout": "30s",
                "wake": true,
                "no_screen_check": true,
                "min_frames": 45,
                "frame_window": "5s",
                "warmup": "2s",
            }),
        )
        .unwrap();
        assert_eq!(argv[0], "diagnostics");
        assert_eq!(argv[1], "performance");
        assert_eq!(argv[2], "com.example");
        for expected in [
            ".MainActivity",
            "--activity",
            "--cold",
            "--settle",
            "1s",
            "--timeout",
            "30s",
            "--wake",
            "--no-screen-check",
            "--min-frames",
            "45",
            "--frame-window",
            "5s",
            "--warmup",
            "2s",
            "--session",
            "ses-1",
            "--json",
        ] {
            assert!(
                argv.iter().any(|argument| argument == expected),
                "missing {expected} in {argv:?}"
            );
        }
    }

    #[test]
    fn build_tool_is_host_only_and_maps_to_cli_argv() {
        let tool = tool_definitions()
            .into_iter()
            .find(|tool| tool["name"] == "androidforge_build")
            .expect("build tool");
        let properties = tool["inputSchema"]["properties"].clone();
        for key in [
            "variant",
            "module",
            "timeout",
            "expected_package",
            "project",
            "dry_run",
        ] {
            assert!(
                properties.get(key).is_some_and(Value::is_object),
                "build schema missing {key}"
            );
        }
        for key in [
            "install", "launch", "cold", "fresh", "session", "serial", "confirm",
        ] {
            assert!(properties.get(key).is_none(), "build tool exposes {key}");
        }
        let command = crate::commands::catalog::command_catalog()
            .into_iter()
            .find(|command| command.tool_name == "androidforge_build")
            .expect("build catalog entry");
        assert_eq!(command.safety, "read-only");

        let argv = tool_argv(
            "androidforge_build",
            &json!({
                "project": "A:/proj",
                "variant": "debug",
                "module": "app",
                "timeout": "20m",
                "expected_package": "com.example.app",
                "dry_run": true,
                "session": "ses-1",
                "serial": "emulator-5554",
                "install": true,
                "launch": true,
                "cold": true,
            }),
        )
        .unwrap();
        assert_eq!(argv[0], "build");
        assert_eq!(argv[1], "--json");
        for expected in [
            "--project",
            "A:/proj",
            "--dry-run",
            "--variant",
            "debug",
            "--module",
            "app",
            "--timeout",
            "20m",
            "--expected-package",
            "com.example.app",
        ] {
            assert!(
                argv.iter().any(|argument| argument == expected),
                "missing {expected} in {argv:?}"
            );
        }
        for forbidden in [
            "--session",
            "ses-1",
            "--serial",
            "emulator-5554",
            "--install",
            "--launch",
            "--cold",
        ] {
            assert!(
                !argv.iter().any(|argument| argument == forbidden),
                "build argv leaked {forbidden}: {argv:?}"
            );
        }
    }

    #[test]
    fn app_install_confirmation_flags_are_in_schema_and_argv() {
        let tool = tool_definitions()
            .into_iter()
            .find(|tool| tool["name"] == "androidforge_app_install")
            .expect("app install tool");
        for key in [
            "wait_for_confirmation",
            "confirm_text",
            "confirmation_timeout",
        ] {
            assert!(
                tool["inputSchema"]["properties"]
                    .get(key)
                    .is_some_and(Value::is_object),
                "app install schema missing {key}"
            );
        }
        let argv = tool_argv(
            "androidforge_app_install",
            &json!({
                "apk": "app-debug.apk",
                "session": "ses-1",
                "wait_for_confirmation": true,
                "confirm_text": ["Install", "Allow"],
                "confirmation_timeout": "20s",
            }),
        )
        .expect("app install argv");
        for expected in [
            "--wait-for-confirmation",
            "--confirm-text",
            "Install",
            "Allow",
            "--confirmation-timeout",
            "20s",
        ] {
            assert!(
                argv.iter().any(|argument| argument == expected),
                "missing {expected} in {argv:?}"
            );
        }
        for forbidden in ["--allow-physical-install", "--allow-physical-mutation"] {
            assert!(
                !argv.iter().any(|argument| argument == forbidden),
                "install argv leaked {forbidden}: {argv:?}"
            );
        }
    }

    #[test]
    fn scenario_tools_map_to_cli_argv_and_keep_physical_gates() {
        let definitions = tool_definitions();
        for name in [
            "androidforge_scenario_run",
            "androidforge_scenario_validate",
        ] {
            let tool = definitions
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("missing {name}"));
            for physical in [
                "allow_physical_mutation",
                "allow_physical_app_control",
                "allow_physical_data",
                "allow_physical_network",
                "allow_physical_install",
                "allow_physical_input",
            ] {
                assert!(
                    tool["inputSchema"]["properties"].get(physical).is_none(),
                    "{name} advertises {physical}"
                );
            }
        }

        let validate = tool_argv(
            "androidforge_scenario_validate",
            &json!({"file": "smoke.toml", "session": "ses-1"}),
        )
        .expect("validate argv");
        assert_eq!(
            validate,
            vec![
                "scenario",
                "validate",
                "smoke.toml",
                "--json",
                "--session",
                "ses-1"
            ]
        );

        let error = tool_argv("androidforge_scenario_run", &json!({"file": "smoke.toml"}))
            .expect_err("run needs confirm or dry_run");
        assert!(error.contains("confirm=true"), "{error}");

        let dry = tool_argv(
            "androidforge_scenario_run",
            &json!({"file": "smoke.toml", "dry_run": true, "repeat": 3, "session": "ses-1"}),
        )
        .expect("dry run argv");
        assert_eq!(
            dry,
            vec![
                "scenario",
                "run",
                "smoke.toml",
                "--session",
                "ses-1",
                "--repeat",
                "3",
                "--dry-run",
                "--json"
            ]
        );

        let run = tool_argv(
            "androidforge_scenario_run",
            &json!({
                "file": "smoke.json",
                "confirm": true,
                "repeat": 2,
                "session": "ses-1",
                "allow_physical_mutation": true,
                "allow_physical_input": true,
            }),
        )
        .expect("run argv");
        assert_eq!(run[0], "scenario");
        assert_eq!(run[1], "run");
        assert_eq!(run[2], "smoke.json");
        assert!(run.contains(&"--yes".to_string()));
        assert!(run.windows(2).any(|pair| pair == ["--repeat", "2"]));
        assert!(
            !run.iter()
                .any(|argument| argument.starts_with("--allow-physical")),
            "scenario argv leaked a physical flag: {run:?}"
        );
    }

    #[test]
    fn devices_shell_mcp_tool_is_read_only_and_tap_forwards_coordinates() {
        let tools = tool_definitions();
        let shell = tools
            .iter()
            .find(|tool| tool["name"] == "androidforge_devices_shell")
            .expect("device shell MCP tool");
        assert!(
            shell["description"]
                .as_str()
                .unwrap_or_default()
                .contains("--read-only")
        );
        for physical in [
            "allow_physical_mutation",
            "allow_physical_input",
            "allow_physical_app_control",
        ] {
            assert!(
                shell["inputSchema"]["properties"].get(physical).is_none(),
                "device shell advertises {physical}"
            );
        }
        let argv = tool_argv(
            "androidforge_devices_shell",
            &json!({
                "session": "ses-1",
                "command": ["dumpsys", "gfxinfo", "com.example"],
                "deny": ["reset"],
            }),
        )
        .expect("shell argv");
        assert!(argv.contains(&"devices".to_string()));
        assert!(argv.contains(&"shell".to_string()));
        assert!(argv.contains(&"--read-only".to_string()));
        assert!(argv.contains(&"--".to_string()));
        assert!(argv.contains(&"dumpsys".to_string()));
        assert!(argv.contains(&"reset".to_string()));
        let separator = argv
            .iter()
            .position(|arg| arg == "--")
            .expect("argument separator");
        let json_flag = argv
            .iter()
            .position(|arg| arg == "--json")
            .expect("json flag");
        assert!(json_flag < separator, "argv: {argv:?}");
        assert!(!argv.iter().any(|arg| arg.starts_with("--allow-physical")));

        let tap = tool_argv(
            "androidforge_devices_input_tap",
            &json!({"session": "ses-1", "x": 540, "y": 1200}),
        )
        .expect("tap argv");
        assert!(tap.contains(&"devices".to_string()));
        assert!(tap.contains(&"input".to_string()));
        assert!(tap.contains(&"tap".to_string()));
        assert!(tap.contains(&"540".to_string()));
        assert!(tap.contains(&"1200".to_string()));
        assert!(!tap.iter().any(|arg| arg.starts_with("--allow-physical")));
    }

    #[test]
    fn logcat_mcp_tools_forward_literal_redact_prefixes() {
        let tools = tool_definitions();
        for name in [
            "androidforge_diagnostics_logcat_query",
            "androidforge_diagnostics_logcat_watch",
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("missing {name}"));
            assert!(
                tool["inputSchema"]["properties"]
                    .get("redact_patterns")
                    .is_some(),
                "{name} does not advertise redact_patterns"
            );
            let argv = tool_argv(
                name,
                &json!({"session": "ses-1", "redact_patterns": ["AIza", "sk-"]}),
            )
            .expect("argv");
            let flags = argv
                .iter()
                .filter(|arg| *arg == "--redact-patterns")
                .count();
            assert_eq!(flags, 2, "{argv:?}");
            assert!(argv.contains(&"AIza".to_string()));
            assert!(argv.contains(&"sk-".to_string()));
        }
    }

    #[test]
    fn new_read_only_tools_are_generated_without_physical_flags() {
        let names = tool_definitions()
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_string))
            .collect::<Vec<_>>();
        for expected in [
            "androidforge_ui_audit_a11y",
            "androidforge_capture_baseline_save",
            "androidforge_capture_baseline_update",
            "androidforge_capture_baseline_compare",
            "androidforge_capture_baseline_list",
            "androidforge_capture_contrast",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}");
        }
        let catalog = crate::commands::catalog::command_catalog();
        for path in [
            "ui audit a11y",
            "capture baseline save",
            "capture baseline update",
            "capture baseline compare",
            "capture baseline list",
            "capture contrast",
        ] {
            let command = catalog
                .iter()
                .find(|command| command.path == path)
                .unwrap_or_else(|| panic!("missing catalog entry {path}"));
            assert_eq!(command.safety, "read-only", "{path}");
        }
    }
}
