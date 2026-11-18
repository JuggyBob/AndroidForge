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
