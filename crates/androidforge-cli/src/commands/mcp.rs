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
        "Export, import, reset or seed app-private data. Import/reset/seed require confirm=true.",
        schema(
            json!({
                "action": {"type": "string", "enum": ["export", "import", "reset", "seed"]},
                "package": {"type": "string"},
                "path": {"type": "string"},
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
        "Wait for a selector, for text to disappear, for an idle hierarchy, or for a stable hierarchy.",
        merge_common_selector(
            common_selector(&["session"]),
            json!({
                "not_exists": {"type": "boolean"},
                "for_text_gone": {"type": "array", "items": {"type": "string"}},
                "for_idle": {"type": "boolean"},
                "for_stable": {"type": "boolean"},
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
        "Run a bounded local mock HTTP(S) server. Set max_requests or duration so the call returns.",
        schema(
            json!({
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
            &["route", "max_requests", "session"],
        ),
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
        "androidforge_app_install",
        "Install an APK or split set. `fresh` requires confirm=true.",
        schema(
            json!({
                "apk": {"type": "string"},
                "session": {"type": "string"},
                "project": {"type": "string"},
                "fresh": {"type": "boolean"},
                "only_if_changed": {"type": "boolean"},
                "allow_downgrade": {"type": "boolean"},
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
        "Explain a run or the latest failure with diagnosis, evidence and next commands.",
        schema(
            json!({
                "run": {"type": "string", "description": "Run id, latest or latest-failure."},
                "project": {"type": "string"},
                "output_profile": {"enum": ["full", "compact", "brief"]},
            }),
            &["run"],
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
        if matches!(
            key.as_str(),
            "session"
                | "project"
                | "serial"
                | "output_profile"
                | "confirm"
                | "allow_physical_mutation"
                | "dry_run"
        ) && !(key == "serial" && serial_is_array(command))
        {
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

pub fn serve_stdio() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut reader = std::io::BufReader::new(stdin.lock());
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = reader
            .read_until(b'\n', &mut buffer)
            .map_err(|error| ForgeError::internal(format!("could not read MCP stdin: {error}")))?;
        if read == 0 {
            break;
        }
        if buffer.len() > 16 * 1024 * 1024 {
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
            argv.extend([
                "ui".to_string(),
                "dismiss-overlays".to_string(),
                "--json".to_string(),
            ]);
            push_global(&mut argv, arguments);
            if let Some(max) = arguments.get("max").and_then(Value::as_u64) {
                argv.push("--max".to_string());
                argv.push(max.to_string());
            }
            if arg_bool(arguments, "dry_run") {
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
                "export" | "import" => {
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
            let max_requests = arguments
                .get("max_requests")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if max_requests == 0 && arg_str(arguments, "duration").is_none() {
                return Err(
                    "androidforge_network_mock_serve requires max_requests > 0 or duration"
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
        if matches!(
            key.as_str(),
            "session"
                | "project"
                | "serial"
                | "output_profile"
                | "confirm"
                | "allow_physical_mutation"
                | "dry_run"
        ) && !(key == "serial" && serial_is_array(command))
        {
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
            &json!({"serial": ["emulator-5554", "emulator-5556"]}),
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
            &json!({"serial": "emulator-5554"}),
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
        let error =
            tool_argv("androidforge_session_stop", &json!({"session": "ses-1"})).unwrap_err();
        assert!(error.contains("confirm=true"), "{error}");

        let error = tool_argv(
            "androidforge_app_data_restore",
            &json!({"package": "com.example", "path": "backup.tar"}),
        )
        .unwrap_err();
        assert!(error.contains("confirm=true"), "{error}");

        let error = tool_argv("androidforge_doze_unforce", &json!({})).unwrap_err();
        assert!(error.contains("confirm=true"), "{error}");

        let argv = tool_argv(
            "androidforge_session_stop",
            &json!({"session": "ses-1", "confirm": true}),
        )
        .unwrap();
        assert!(argv.contains(&"--yes".to_string()));
    }
}
