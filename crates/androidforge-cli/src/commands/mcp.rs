//! Local MCP stdio server and function-calling tool catalog.
//!
//! The server deliberately shells out to the same `androidforge` binary. That
//! keeps the MCP surface, the CLI and the JSON contract on one implementation
//! instead of creating a second command engine.

use std::io::{BufRead, Write};
use std::process::Command;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use serde_json::{Value, json};

use crate::cli::{McpCommand, ToolsCommand, ToolsSchemaArgs};

pub fn mcp(command: &McpCommand) -> Result<CommandSuccess> {
    match command {
        McpCommand::Serve => Err(ForgeError::new(
            ErrorCode::UnsupportedOperation,
            "mcp serve is a long-running stdio command and must be started directly",
        )
        .with_suggestion("Run `androidforge mcp serve` as a child process.")),
    }
}

pub fn tools(command: &ToolsCommand) -> Result<CommandSuccess> {
    match command {
        ToolsCommand::Schema(args) => tools_schema(args),
    }
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
    tools
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
    for line in stdin.lock().lines() {
        let line = line
            .map_err(|error| ForgeError::internal(format!("could not read MCP stdin: {error}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
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
    let output = match Command::new(exe).args(argv).output() {
        Ok(output) => output,
        Err(error) => return (false, String::new(), format!("could not run CLI: {error}")),
    };
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
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
        other => return Err(format!("unknown AndroidForge tool: {other}")),
    }
    Ok(argv)
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
}
