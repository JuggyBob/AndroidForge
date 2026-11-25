use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::{Value, json};

fn schema_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("schema")
}

fn schema(name: &str) -> Value {
    let path = schema_dir().join(name);
    let bytes = fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn validate(name: &str, value: &Value) {
    let schema = schema(name);
    let validator = jsonschema::validator_for(&schema)
        .unwrap_or_else(|error| panic!("compile schema {name}: {error}"));
    if let Err(error) = validator.validate(value) {
        panic!("{name} rejected payload: {error}\npayload: {value:#}");
    }
}

fn temp_project() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("temp project");
    let config = temp.path().join(".androidforge");
    fs::create_dir_all(&config).expect("adapter dir");
    fs::write(
        config.join("project.toml"),
        "adapter_version = 1\n[project]\nname = \"SchemaTest\"\npackage = \"com.example.schema\"\n",
    )
    .expect("adapter");
    temp
}

fn run_json(temp: &Path, args: &[&str]) -> Value {
    let output = Command::cargo_bin("androidforge")
        .expect("binary")
        .current_dir(temp)
        .env("ANDROIDFORGE_HOME", temp)
        .env_remove("ANDROIDFORGE_SESSION")
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run {args:?}: {error}"));
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("parse {args:?}: {error}\nstdout: {stdout}"))
}

#[test]
fn observation_schema_accepts_additive_semantics_and_test_tag() {
    let node = json!({
        "ref": "el-abcdefabcdef",
        "role": "button",
        "path": "root/0",
        "depth": 1,
        "resource_id": "checkout_button",
        "test_tag": "checkout_button",
        "bounds": {"left": 0, "top": 0, "right": 100, "bottom": 50},
        "clickable": true,
        "enabled": true
    });
    let observation = json!({
        "schema_version": 1,
        "token": "obs-semantics",
        "age_ms": 0,
        "stale": false,
        "foreground": {"package": "com.example"},
        "actionable": [node],
        "actionable_total": 1,
        "context": [],
        "context_total": 0,
        "semantics": [node],
        "semantics_total": 1,
        "semantics_truncated": false
    });
    validate("observation.schema.json", &observation);

    let mut without_semantics = observation;
    {
        let object = without_semantics.as_object_mut().unwrap();
        object.remove("semantics");
        object.remove("semantics_total");
        object.remove("semantics_truncated");
    }
    validate("observation.schema.json", &without_semantics);
}

#[test]
fn every_schema_document_is_valid_draft_2020_12() {
    let mut count = 0;
    for entry in fs::read_dir(schema_dir()).expect("schema directory") {
        let path = entry.expect("schema entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path).expect("read schema");
        let value: Value = serde_json::from_slice(&bytes).expect("parse schema");
        jsonschema::meta::validate(&value)
            .unwrap_or_else(|error| panic!("invalid schema {}: {error}", path.display()));
        count += 1;
    }
    assert!(count >= 20, "expected the full schema set, found {count}");
}

#[test]
fn context_and_agent_profiles_validate_against_their_schemas() {
    let temp = temp_project();
    let root = temp.path();

    for profile in [None, Some("compact"), Some("brief")] {
        let mut args = vec!["context", "--json"];
        if let Some(profile) = profile {
            args.extend(["--output-profile", profile]);
        }
        let output = run_json(root, &args);
        assert_eq!(
            output["status"], "success",
            "context {profile:?}: {output:#}"
        );
        validate("context.schema.json", &output["result"]);
    }

    let brief = run_json(root, &["agent", "brief", "--json"]);
    assert_eq!(brief["command"], "agent.brief");
    validate("agent-brief.schema.json", &brief["result"]);

    for profile in [None, Some("compact"), Some("brief")] {
        let mut args = vec!["agent", "test", "--json", "--variant", "debug", "--dry-run"];
        if let Some(profile) = profile {
            args.extend(["--output-profile", profile]);
        }
        let output = run_json(root, &args);
        assert!(
            output["result"]["stages"].is_array(),
            "agent test {profile:?}: {output:#}"
        );
        validate("agent-test.schema.json", &output["result"]);
    }
}

#[test]
fn build_result_is_versioned_and_additive_in_dry_run() {
    let temp = temp_project();
    let config = temp.path().join(".androidforge/project.toml");
    let mut contents = fs::read_to_string(&config).expect("project config");
    let argv = if cfg!(windows) {
        r#"[build.commands]
debug = ["cmd", "/c", "echo fake build"]
"#
    } else {
        r#"[build.commands]
debug = ["sh", "-c", "echo fake build"]
"#
    };
    contents.push_str("[build]\ndefault_variant = \"debug\"\ntimeout_secs = 30\n");
    contents.push_str(argv);
    fs::write(&config, contents).expect("write build config");

    let value = run_json(temp.path(), &["--dry-run", "build", "--json"]);
    assert_eq!(value["status"], "success", "{value:#}");
    assert_eq!(value["command"], "build");
    let result = value["result"].clone();
    let schema = json!({
        "type": "object",
        "required": ["build"],
        "properties": {
            "build": {
                "type": "object",
                "required": ["schema_version", "status", "variant", "argv", "cwd", "timeout_ms"],
                "properties": {
                    "schema_version": {"const": 1},
                    "status": {"enum": ["success", "failed", "dry_run"]},
                    "variant": {"type": "string"},
                    "argv": {"type": "array", "minItems": 1},
                    "cwd": {"type": "string"},
                    "timeout_ms": {"type": "integer", "minimum": 1},
                    "warnings": {"type": "array"}
                },
                "additionalProperties": true
            }
        },
        "additionalProperties": true
    });
    let validator = jsonschema::validator_for(&schema).expect("compile inline build schema");
    validator
        .validate(&result)
        .unwrap_or_else(|error| panic!("build result rejected: {error}\n{result:#}"));
    assert_eq!(result["build"]["status"], "dry_run");
    assert!(result.get("apk").is_none());
    assert!(result.get("install").is_none());
    assert!(result.get("launch").is_none());
}

fn observation_payload(compact: bool) -> Value {
    let mut value = json!({
        "schema_version": 1,
        "id": "obs-123456789012",
        "token": "obs-123456789012",
        "session_id": "ses-1",
        "device_serial": "emulator-5554",
        "observed_at": "2026-01-01T00:00:00.000Z",
        "age_ms": 0,
        "stale": false,
        "display": {"width": 1080, "height": 2400, "orientation": "portrait", "rotation": 0},
        "foreground": {"package": "com.example"},
        "focused": null,
        "actionable": [{
            "ref": "el-0123456789ab",
            "actionable_via": "ancestor",
            "role": "button",
            "path": "0/1",
            "depth": 1,
            "text": "Save",
            "resource_id": "com.example:id/save",
            "class": "android.widget.Button",
            "package": "com.example",
            "bounds": {"left": 0, "top": 0, "right": 100, "bottom": 50},
            "clickable": false,
            "enabled": true
        }],
        "actionable_total": 1,
        "context": [],
        "context_total": 0,
        "scrollable": [],
        "visible_text": ["Save"],
        "hierarchy": {"package": "com.example", "rotation": 0, "node_count": 1, "compact_node_count": 1},
        "raw_xml_path": "ui/obs.xml"
    });
    if compact {
        let object = value.as_object_mut().expect("object");
        for key in [
            "id",
            "observed_at",
            "session_id",
            "device_serial",
            "display",
            "hierarchy",
            "raw_xml_path",
        ] {
            object.remove(key);
        }
        if let Some(node) = object
            .get_mut("actionable")
            .and_then(Value::as_array_mut)
            .and_then(|items| items.first_mut())
            .and_then(Value::as_object_mut)
        {
            for key in ["class", "package"] {
                node.remove(key);
            }
        }
    }
    value
}

fn receipt_payload(compact: bool) -> Value {
    let mut value = json!({
        "schema_version": 1,
        "receipt_id": "rcpt-0123456789ab",
        "action": "tap",
        "selector": {"text": "Save"},
        "session_id": "ses-1",
        "device_serial": "emulator-5554",
        "started_at": "2026-01-01T00:00:00.000Z",
        "ended_at": "2026-01-01T00:00:01.000Z",
        "duration_ms": 1000,
        "target_resolved": true,
        "dispatched": true,
        "dispatch_state": "dispatched",
        "target": {"text": "Save"},
        "before": {"token": "obs-1", "observed_at": "2026-01-01T00:00:00.000Z"},
        "after": {"token": "obs-2", "observed_at": "2026-01-01T00:00:01.000Z"},
        "effect": "content_changed",
        "effect_evidence": ["semantic diff: 1 modified"],
        "changes": {"added": [], "removed": [], "modified": [], "added_total": 0, "removed_total": 0, "modified_total": 1, "truncated": false, "focus_changed": false, "package_changed": false, "activity_changed": false},
        "completeness": "complete",
        "warnings": [],
        "postcondition": {"result": "satisfied", "duration_ms": 10, "conditions": []},
        "dispatch_error": null,
        "action_result": {"tap_x": 50, "tap_y": 25}
    });
    if compact {
        let object = value.as_object_mut().expect("object");
        for key in ["selector", "target", "action_result"] {
            object.remove(key);
        }
    }
    value
}

fn diff_payload(_brief: bool) -> Value {
    json!({
        "schema_version": 1,
        "from": "obs-1",
        "to": "obs-2",
        "session_id": "ses-1",
        "device_serial": "emulator-5554",
        "changes": {
            "added": [],
            "removed": [],
            "modified": [],
            "added_total": 0,
            "removed_total": 0,
            "modified_total": 0,
            "truncated": false,
            "focus_changed": false,
            "foreground_package_before": "com.example",
            "foreground_package_after": "com.example",
            "foreground_activity_before": ".Main",
            "foreground_activity_after": ".Main",
            "package_changed": false,
            "activity_changed": false
        }
    })
}

fn performance_payload() -> Value {
    json!({
        "schema_version": 1,
        "package": "com.example",
        "serial": "emulator-5554",
        "collected_at": "2026-01-01T00:00:00.000Z",
        "run_id": "run-1",
        "run_dir": "A:/runs/run-1",
        "artifacts": [{"kind": "performance_report", "path": "performance/report.json"}],
        "cold_start_requested": true,
        "settle_ms": 2000,
        "startup": {"requested": true, "cold": true, "total_time_ms": 500},
        "jank": {"reset_before_launch": true, "total_frames": 100, "janky_frames": 5, "janky_percent": 5.0},
        "memory": {"total_pss_kb": 1000},
        "cpu": {"total_percent": 1.0},
        "commands": []
    })
}

fn diagnosis_payload(compact: bool) -> Value {
    let mut value = json!({
        "schema_version": 1,
        "generated_at": "2026-01-01T00:00:00.000Z",
        "run_id": "run-1",
        "project": "Example",
        "project_root": "A:/example",
        "scenario": "ui.tap",
        "status": "failed",
        "diagnosis": {"category": "selector", "code": "E_SELECTOR_NOT_FOUND", "confidence": "high", "summary": "not found"},
        "alternatives": [],
        "evidence": [],
        "next_actions": [],
        "fingerprint": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "fingerprint_version": 1,
        "artifacts": [],
        "uncertainty": []
    });
    if compact {
        let object = value.as_object_mut().expect("object");
        for key in ["evidence", "alternatives", "uncertainty"] {
            object.remove(key);
        }
    }
    value
}

#[test]
fn every_diagnosis_category_validates_against_the_schema() {
    use androidforge_core::diagnosis::DiagnosisCategory;

    // Includes the 1.1.0 additions: performance, build and artifact. Iterating
    // the enum instead of a handwritten list catches future schema drift.
    for category in DiagnosisCategory::ALL {
        let mut payload = diagnosis_payload(false);
        payload["diagnosis"]["category"] = json!(category.as_str());
        validate("diagnosis.schema.json", &payload);
    }
}

#[test]
fn frames_and_capture_launch_payloads_validate_against_their_schemas() {
    use androidforge_diagnostics::{compare_frames_reports_default, frames_report_from_gfxinfo};

    let gfxinfo = "Graphics info for pid 1234 [com.example]\n\
        Uptime: 123456 Realtime: 123456\n\
        Total frames rendered: 120\n\
        Janky frames: 9 (7.50%)\n\
        Janky frames legacy: 5 (4.17%)\n\
        50th percentile: 8ms\n\
        99th percentile: 40ms\n\
        Max: 72ms\n\
        Number Missed Vsync: 3\n";
    let report = frames_report_from_gfxinfo(
        gfxinfo,
        "com.example",
        "emulator-5554",
        "2026-01-01T00:00:00.000Z",
        30,
    );
    validate(
        "frames.schema.json",
        &serde_json::to_value(&report).expect("frames"),
    );

    let mut compared = report.clone();
    compared.comparison = Some(compare_frames_reports_default(&report, &report));
    compared.baseline_source = Some("runs/old/frames/report.json".to_string());
    compared.run_id = Some("run-1".to_string());
    validate(
        "frames.schema.json",
        &serde_json::to_value(&compared).expect("compared frames"),
    );

    let launch = androidforge_diagnostics::LaunchCaptureResult::default();
    let mut value = serde_json::to_value(&launch).expect("capture launch");
    value["schema_version"] = json!(1);
    value["collected_at"] = json!("2026-01-01T00:00:00.000Z");
    validate("capture-launch.schema.json", &value);

    value["status"] = json!("first_frame_found");
    value["first_app_frame_ms"] = json!(120);
    value["first_frame_rgb"] = json!({"hex": "#FF0000", "r": 255, "g": 0, "b": 0});
    value["dominant_color"] = json!({"hex": "#FF0000", "r": 255, "g": 0, "b": 0});
    value["assertion"] = json!({
        "passed": true,
        "expect_first_frame": true,
        "no_splash": true,
        "expected_color": "#FF0000",
        "expected_color_sample": "centre",
        "checked_color": {"hex": "#FF0000", "r": 255, "g": 0, "b": 0},
        "first_app_frame_ms": 120,
        "tolerance": 16,
    });
    validate("capture-launch.schema.json", &value);
}

#[test]
fn representative_payloads_validate_against_their_schemas() {
    validate("observation.schema.json", &observation_payload(false));
    validate("observation.schema.json", &observation_payload(true));
    validate("action-receipt.schema.json", &receipt_payload(false));
    validate("action-receipt.schema.json", &receipt_payload(true));
    validate("ui-diff.schema.json", &diff_payload(false));
    validate("ui-diff.schema.json", &diff_payload(true));
    validate("performance.schema.json", &performance_payload());
    validate("diagnosis.schema.json", &diagnosis_payload(false));
    validate("diagnosis.schema.json", &diagnosis_payload(true));
    validate(
        "bundle.schema.json",
        &json!({
            "schema_version": 1,
            "bundle_id": "bundle-0123456789ab",
            "created_at": "2026-01-01T00:00:00.000Z",
            "tool_version": env!("CARGO_PKG_VERSION"),
            "run_id": "run-1",
            "project": "Example",
            "project_root": "A:/example",
            "source_run_dir": "A:/runs/run-1",
            "redaction_enabled": true,
            "format": "zip",
            "entries": [{
                "path": "summary.json",
                "kind": "summary",
                "size_bytes": 10,
                "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }],
            "excluded": [],
            "total_payload_bytes": 10,
            "warnings": [],
            "checksums_verified": true,
            "original_run_modified": false
        }),
    );
}

fn performance_details(startup_ms: u64, janky_percent: f64) -> Value {
    json!({
        "status": "passed",
        "started_at": "2026-01-02T00:00:00Z",
        "steps": [{
            "id": "step-001",
            "name": "diagnostics performance",
            "command": "diagnostics.performance",
            "status": "success",
            "exit_code": 0,
            "started_at": "2026-01-02T00:00:00Z",
            "ended_at": "2026-01-02T00:00:01Z",
            "duration_ms": 1000,
            "failure": null,
            "artifacts": [],
            "details": {
                "startup": {"total_time_ms": startup_ms},
                "jank": {
                    "total_frames": 100,
                    "janky_frames": janky_percent as u64,
                    "janky_percent": janky_percent,
                    "percentile_99_ms": 40.0,
                    "comparable": true,
                    "sample_status": "ok"
                }
            },
            "retries": 0
        }]
    })
}

#[test]
fn tagged_and_legacy_reports_and_budgeted_comparisons_validate_against_schemas() {
    let temp = temp_project();
    let root = temp.path();

    let start = |extra: &[&str]| -> String {
        let mut args = vec!["run", "start", "--name", "schema-run", "--json"];
        args.extend_from_slice(extra);
        let value = run_json(root, &args);
        value["run_id"].as_str().expect("run id").to_string()
    };
    let tagged = start(&["--tag", "release", "--meta", "build=42"]);
    let legacy = start(&[]);
    let report_path = |id: &str| root.join(".artifacts/runs").join(id).join("report.json");

    let tagged_report: Value =
        serde_json::from_slice(&fs::read(report_path(&tagged)).unwrap()).unwrap();
    validate("report.schema.json", &tagged_report);
    assert_eq!(tagged_report["tags"], json!(["release"]));
    assert_eq!(tagged_report["meta"]["build"], "42");

    let mut legacy_report: Value =
        serde_json::from_slice(&fs::read(report_path(&legacy)).unwrap()).unwrap();
    let object = legacy_report.as_object_mut().unwrap();
    object.remove("tags");
    object.remove("meta");
    validate("report.schema.json", &legacy_report);
    fs::write(
        report_path(&legacy),
        serde_json::to_vec_pretty(&legacy_report).unwrap(),
    )
    .unwrap();

    let patch = |id: &str, patch: Value| {
        let path = report_path(id);
        let mut report: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for (key, value) in patch.as_object().unwrap() {
            report[key.clone()] = value.clone();
        }
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    };
    patch(&legacy, performance_details(500, 5.0));
    let mut current = performance_details(600, 6.0);
    current["started_at"] = json!("2026-01-03T00:00:00Z");
    patch(&tagged, current);

    let passing = run_json(
        root,
        &[
            "report",
            "compare",
            &tagged,
            "--max-startup-ms",
            "800",
            "--json",
        ],
    );
    assert_eq!(passing["status"], "success", "{passing:#}");
    validate("analysis.schema.json", &passing["result"]);
    assert_eq!(passing["result"]["budget_passed"], true);

    let failing = run_json(
        root,
        &[
            "report",
            "compare",
            &tagged,
            "--max-startup-ms",
            "550",
            "--json",
        ],
    );
    assert_eq!(failing["status"], "success", "{failing:#}");
    validate("analysis.schema.json", &failing["result"]);
    assert_eq!(failing["result"]["budget_passed"], false);
    assert_eq!(
        failing["result"]["budget_violations"][0]["metric"],
        "startup_ms"
    );
}

#[test]
fn synthetic_bounds_stability_result_validates_against_ui_wait_schema() {
    // The bounds-stability result is produced by a read-only selector loop;
    // its shape is checked with synthetic data so no device is involved.
    let payload = json!({
        "stable": true,
        "bounds": {
            "left": 100,
            "top": 200,
            "right": 300,
            "bottom": 300,
            "width": 200,
            "height": 100,
            "center_x": 200,
            "center_y": 250
        },
        "attempts": 2,
        "duration_ms": 250,
        "stable_for_ms": 1
    });
    validate("ui-wait.schema.json", &payload);
}

#[test]
fn e1_catalog_and_mcp_entries_carry_argv_and_safety() {
    let temp = temp_project();
    let catalog = run_json(temp.path(), &["tools", "catalog", "--json"]);
    assert_eq!(catalog["status"], "success", "{catalog:#}");
    let commands = catalog["result"]["commands"].as_array().expect("commands");
    let find = |path: &str| {
        commands
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("missing catalog entry {path}"))
            .clone()
    };
    for (path, safety) in [
        ("avd images", "read-only"),
        ("avd create", "destructive"),
        ("ports list", "read-only"),
        ("devices scenario", "device-mutation"),
        ("devices tag add", "destructive"),
        ("devices tag remove", "destructive"),
        ("devices tag list", "read-only"),
        ("devices tag clear", "destructive"),
    ] {
        assert_eq!(find(path)["safety"], safety, "{path}");
    }
    let arguments = |path: &str| {
        find(path)["arguments"]
            .as_array()
            .expect("arguments")
            .iter()
            .filter_map(|argument| argument["long"].as_str().map(str::to_string))
            .collect::<Vec<_>>()
    };
    for flag in ["name", "image", "device"] {
        assert!(
            find("avd create")["arguments"]
                .as_array()
                .expect("arguments")
                .iter()
                .any(|argument| argument["id"] == flag),
            "avd create is missing {flag}"
        );
    }
    for flag in ["night", "locale", "font", "network"] {
        assert!(
            arguments("devices scenario").contains(&flag.to_string()),
            "devices scenario is missing --{flag}"
        );
    }
    assert!(arguments("agent matrix").contains(&"device".to_string()));
    assert!(arguments("session gc").contains(&"forget-terminal".to_string()));

    let tools = run_json(
        temp.path(),
        &["tools", "schema", "--format", "mcp", "--json"],
    );
    assert_eq!(tools["status"], "success", "{tools:#}");
    let definitions = tools["result"]["tools"].as_array().expect("tools");
    let find_tool = |name: &str| {
        definitions
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing MCP tool {name}"))
            .clone()
    };
    let capacity = find_tool("androidforge_doctor_capacity");
    assert!(
        capacity["description"]
            .as_str()
            .unwrap_or_default()
            .contains("Unknown")
    );
    let create = find_tool("androidforge_avd_create");
    for key in ["name", "image", "device", "confirm"] {
        assert!(
            create["inputSchema"]["properties"][key].is_object(),
            "avd create MCP schema is missing {key}"
        );
    }
    let scenario = find_tool("androidforge_devices_scenario");
    for key in ["night", "locale", "font", "network"] {
        assert!(
            scenario["inputSchema"]["properties"][key].is_object(),
            "scenario MCP schema is missing {key}"
        );
    }
    let matrix = find_tool("androidforge_agent_matrix");
    assert!(matrix["inputSchema"]["properties"]["device"].is_object());
}

#[test]
fn e1_capacity_and_port_payloads_never_fabricate_values() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let capacity = run_json(temp.path(), &["doctor", "--capacity", "--json"]);
    assert_eq!(capacity["status"], "success", "{capacity:#}");
    let reading_schema = json!({
        "type": "object",
        "required": ["value_bytes", "source"],
        "properties": {
            "value_bytes": {"type": ["integer", "null"], "minimum": 1},
            "source": {"type": "string", "minLength": 1},
            "unavailable_reason": {"type": "string"}
        },
        "allOf": [{
            "if": {"properties": {"value_bytes": {"type": "null"}}, "required": ["value_bytes"]},
            "then": {"required": ["unavailable_reason"], "properties": {"unavailable_reason": {"type": "string", "minLength": 1}}}
        }],
        "additionalProperties": true
    });
    let validator = jsonschema::validator_for(&reading_schema).expect("compile reading schema");
    for field in [
        "physical_total",
        "physical_available",
        "commit_limit",
        "commit_available",
        "disk_free",
    ] {
        validator
            .validate(&capacity["result"]["host"][field])
            .unwrap_or_else(|error| panic!("{field} rejected: {error}"));
    }
    let entries = run_json(temp.path(), &["ports", "list", "--json"]);
    assert_eq!(entries["status"], "success", "{entries:#}");
    let ports = entries["result"]["ports"].as_array().expect("ports");
    assert_eq!(ports.len(), 65);
    for entry in ports {
        assert!(
            ["free", "allocated", "occupied"]
                .contains(&entry["state"].as_str().unwrap_or_default()),
            "{entry}"
        );
        assert!(
            entry["console_port"]
                .as_u64()
                .is_some_and(|port| port >= 5554)
        );
        assert_eq!(
            entry["adb_port"].as_u64(),
            entry["console_port"].as_u64().map(|port| port + 1)
        );
    }
}

#[test]
fn e2_fixture_state_and_mock_scenario_payloads_match_their_schemas() {
    let temp = temp_project();
    let fixtures = temp.path().join(".androidforge/fixtures");
    fs::create_dir_all(&fixtures).expect("fixtures dir");
    let manifest = json!({
        "version": 1,
        "name": "state-demo",
        "state": {"prefs": {"values": {"onboarding_done": "true"}}},
        "steps": [
            {"name": "apply", "action": {"type": "apply_state"}},
            {"name": "reset", "phase": "cleanup", "action": {"type": "reset_state"}}
        ]
    });
    fs::write(
        fixtures.join("state-demo.json"),
        serde_json::to_string_pretty(&manifest).expect("fixture JSON"),
    )
    .expect("write fixture");
    validate("fixture.schema.json", &manifest);

    let scenarios = run_json(temp.path(), &["network", "mock", "scenarios", "--json"]);
    assert_eq!(scenarios["status"], "success", "{scenarios:#}");
    let entries = scenarios["result"]["scenarios"]
        .as_array()
        .expect("scenarios");
    assert_eq!(entries.len(), 4);
    for entry in entries {
        validate("network-mock-scenario.schema.json", entry);
        assert!(entry.get("fixture_file").is_some());
    }

    let log = temp.path().join("requests.ndjson");
    let offline = run_json(
        temp.path(),
        &[
            "network",
            "mock",
            "serve",
            "--scenario",
            "offline",
            "--port",
            "18082",
            "--log",
            log.to_str().expect("log path"),
            "--json",
        ],
    );
    assert_eq!(offline["status"], "success", "{offline:#}");
    validate("network-mock.schema.json", &offline["result"]);
    assert_eq!(offline["result"]["scenario"], "offline");
    assert_eq!(offline["result"]["stopped_by"], "offline");
    assert_eq!(offline["result"]["requests"], 0);

    let apply = run_json(
        temp.path(),
        &["fixture", "apply", "state-demo", "--dry-run", "--json"],
    );
    assert_eq!(apply["status"], "success", "{apply:#}");
    assert_eq!(apply["result"]["action"], "apply_state");
    assert_eq!(apply["result"]["dry_run"], true);

    let tools = run_json(
        temp.path(),
        &["tools", "schema", "--format", "mcp", "--json"],
    );
    let definitions = tools["result"]["tools"].as_array().expect("tools");
    for name in [
        "androidforge_fixture_apply",
        "androidforge_fixture_reset",
        "androidforge_network_mock_scenarios",
    ] {
        assert!(
            definitions.iter().any(|tool| tool["name"] == name),
            "missing MCP tool {name}"
        );
    }
    let apply_tool = definitions
        .iter()
        .find(|tool| tool["name"] == "androidforge_fixture_apply")
        .expect("fixture apply tool");
    assert!(
        apply_tool["inputSchema"]["properties"]["confirm"].is_object(),
        "fixture apply MCP schema must require explicit confirm"
    );
}

#[test]
fn a11y_and_baseline_schemas_are_registered_and_validate_payloads() {
    let temp = temp_project();
    let listed = run_json(temp.path(), &["schema", "list", "--json"]);
    let schemas = listed["result"]["schemas"].as_array().expect("schemas");
    for name in ["a11y.schema.json", "visual-baseline.schema.json"] {
        assert!(
            schemas.iter().any(|value| value == name),
            "schema registry is missing {name}"
        );
    }
    let shown = run_json(
        temp.path(),
        &["schema", "show", "a11y.schema.json", "--json"],
    );
    assert_eq!(shown["status"], "success", "{shown:#}");
    assert!(
        shown["result"]["title"]
            .as_str()
            .is_some_and(|title| title.contains("accessibility"))
    );

    let finding = json!({
        "rule": "low_contrast",
        "severity": "error",
        "node_path": "0/1",
        "node_class": "android.widget.TextView",
        "bounds": {"left": 0, "top": 0, "right": 100, "bottom": 40},
        "label": "Low contrast",
        "measured": {
            "kind": "low_contrast",
            "ratio": 3.1,
            "threshold": 4.5,
            "foreground_hex": "#808080",
            "background_hex": "#FFFFFF"
        },
        "message": "darken the text or lighten the background"
    });
    let report = json!({
        "findings": [finding],
        "nodes_examined": 1,
        "options": {
            "min_touch_target_dp": 48.0,
            "density": 1.0,
            "min_contrast_ratio": 4.5,
            "large_text_contrast_ratio": 3.0,
            "large_text_min_sp": 24.0,
            "check_content_descriptions": true,
            "check_touch_targets": true,
            "check_duplicate_labels": true,
            "max_findings": 200,
            "max_nodes": 5000
        },
        "truncated": false
    });
    let payload = json!({
        "schema_version": 1,
        "report": report,
        "findings": [finding],
        "counts": {
            "error": 1,
            "warning": 0,
            "info": 0,
            "missing_content_description": 0,
            "tiny_touch_target": 0,
            "duplicate_label": 0,
            "low_contrast": 1
        },
        "contrast": {
            "text_rgb": "#808080",
            "threshold": 4.5,
            "large_text": false,
            "nodes_sampled": 1,
            "width": 100,
            "height": 40
        },
        "serial": "emulator-5554"
    });
    validate("a11y.schema.json", &payload);

    let baseline = json!({
        "name": "home_screen",
        "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "width": 1080,
        "height": 2400,
        "created_at": "2026-11-25T00:00:00Z",
        "source_path": "screens/home.png",
        "path": "home_screen.png",
        "size_bytes": 2048
    });
    validate("visual-baseline.schema.json", &baseline);
}

#[test]
fn retention_plan_and_scoped_export_are_exposed_in_schema_catalog_and_mcp() {
    let temp = temp_project();
    let root = temp.path();
    let runs = root.join(".artifacts/runs");
    for (run_id, tags) in [("run-tagged", vec!["release"]), ("run-plain", vec![])] {
        let dir = runs.join(run_id);
        fs::create_dir_all(&dir).expect("run dir");
        fs::write(dir.join("payload.bin"), vec![b'x'; 16]).expect("payload");
        let report = json!({
            "schema_version": 1,
            "run_id": run_id,
            "command": "test.command",
            "project": "Example",
            "project_root": root.to_string_lossy(),
            "status": "passed",
            "exit_code": 1,
            "started_at": "2020-01-01T00:00:00Z",
            "duration_ms": 0,
            "tags": tags,
        });
        fs::write(
            dir.join("report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .expect("report");
    }

    let cleaned = run_json(
        root,
        &[
            "maintenance",
            "clean",
            "--runs",
            "--keep-last",
            "0",
            "--keep-tag",
            "release",
            "--json",
        ],
    );
    assert_eq!(cleaned["status"], "success", "{cleaned:#}");
    assert_eq!(cleaned["result"]["dry_run"], true);
    validate("maintenance.schema.json", &cleaned["result"]);
    let plan = &cleaned["result"]["retention"];
    assert_eq!(plan["keep_tags"], json!(["release"]));
    assert_eq!(plan["deletions"][0]["run_id"], "run-plain");
    assert_eq!(plan["deletions"][0]["relative_path"], "runs/run-plain");
    assert!(
        plan["deletions"][0]["artifacts"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("payload.bin")
    );
    assert!(plan["kept_runs"].as_array().unwrap().iter().any(|entry| {
        entry["run_id"] == "run-tagged"
            && entry["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("release")
    }));

    let catalog = run_json(root, &["tools", "catalog", "--json"]);
    let commands = catalog["result"]["commands"].as_array().expect("commands");
    let clean_entry = commands
        .iter()
        .find(|entry| entry["path"] == "maintenance clean")
        .expect("maintenance clean catalog entry");
    assert_eq!(clean_entry["safety"], "destructive");
    let export_entry = commands
        .iter()
        .find(|entry| entry["path"] == "app data export")
        .expect("app data export catalog entry");
    assert_eq!(export_entry["safety"], "read-only");
    for flag in ["output", "allow-in-project", "overwrite"] {
        assert!(
            export_entry["arguments"]
                .as_array()
                .unwrap()
                .iter()
                .any(|argument| argument["long"] == flag),
            "app data export catalog is missing --{flag}"
        );
    }
    assert!(
        clean_entry["arguments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|argument| argument["long"] == "keep-tag")
    );

    let tools = run_json(root, &["tools", "schema", "--format", "mcp", "--json"]);
    let definitions = tools["result"]["tools"].as_array().expect("tools");
    let clean_tool = definitions
        .iter()
        .find(|tool| tool["name"] == "androidforge_maintenance_clean")
        .expect("maintenance clean MCP tool");
    assert_eq!(
        clean_tool["inputSchema"]["properties"]["keep_tag"]["type"],
        "array"
    );
    assert!(
        clean_tool["inputSchema"]["properties"]["confirm"].is_object(),
        "retention must stay destructive in MCP"
    );
    let app_data = definitions
        .iter()
        .find(|tool| tool["name"] == "androidforge_app_data")
        .expect("app data MCP tool");
    for key in ["output", "allow_in_project", "overwrite"] {
        assert!(
            app_data["inputSchema"]["properties"][key].is_object(),
            "app data MCP tool is missing {key}"
        );
    }
}

#[test]
fn scenario_manifest_schema_and_mcp_catalog_entries() {
    let temp = temp_project();
    let root = temp.path();
    let manifest = json!({
        "version": 1,
        "name": "schema-smoke",
        "repeat": {"count": 2},
        "steps": [
            {"type": "screenshot", "name": "home"},
            {"type": "assert", "expect": {"text": "Welcome"}}
        ]
    });
    validate("scenario.schema.json", &manifest);

    let path = root.join("scenario.json");
    fs::write(
        &path,
        serde_json::to_vec(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
    let validated = run_json(
        root,
        &["scenario", "validate", path.to_str().unwrap(), "--json"],
    );
    assert_eq!(validated["status"], "success", "{validated:#}");
    assert_eq!(validated["result"]["steps"].as_array().unwrap().len(), 2);

    let catalog = run_json(root, &["tools", "catalog", "--json"]);
    let commands = catalog["result"]["commands"].as_array().expect("catalog");
    let run = commands
        .iter()
        .find(|command| command["path"] == "scenario run")
        .expect("scenario run catalog entry");
    assert_eq!(run["safety"], "device-mutation");
    assert_eq!(run["tool_name"], "androidforge_scenario_run");
    let validate_command = commands
        .iter()
        .find(|command| command["path"] == "scenario validate")
        .expect("scenario validate catalog entry");
    assert_eq!(validate_command["safety"], "read-only");
    assert_eq!(
        validate_command["tool_name"],
        "androidforge_scenario_validate"
    );

    let tools = run_json(root, &["tools", "schema", "--format", "mcp", "--json"]);
    let definitions = tools["result"]["tools"].as_array().expect("tools");
    for name in [
        "androidforge_scenario_run",
        "androidforge_scenario_validate",
    ] {
        assert!(
            definitions.iter().any(|tool| tool["name"] == name),
            "missing MCP tool {name}"
        );
    }
    let scenario_run = definitions
        .iter()
        .find(|tool| tool["name"] == "androidforge_scenario_run")
        .expect("scenario run MCP tool");
    for physical in [
        "allow_physical_mutation",
        "allow_physical_app_control",
        "allow_physical_data",
        "allow_physical_network",
        "allow_physical_install",
        "allow_physical_input",
    ] {
        assert!(
            scenario_run["inputSchema"]["properties"]
                .get(physical)
                .is_none(),
            "scenario run MCP tool advertises {physical}"
        );
    }
    assert!(
        scenario_run["inputSchema"]["properties"]["confirm"].is_object(),
        "scenario run must advertise confirm in MCP"
    );
}
