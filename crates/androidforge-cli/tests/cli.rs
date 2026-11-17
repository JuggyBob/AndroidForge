use std::io::Read;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

fn binary() -> Command {
    Command::cargo_bin("androidforge").expect("androidforge binary")
}

#[test]
fn version_and_help_do_not_require_an_sdk() {
    binary()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("androidforge"));
    binary()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("doctor"))
        .stdout(predicate::str::contains("devices"));
}

#[test]
fn invalid_command_is_structured_json_with_suggestion() {
    let output = binary()
        .args(["emualtor", "strt", "--json"])
        .assert()
        .failure()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).expect("structured JSON error");
    assert_eq!(json["status"], "error");
    assert_eq!(json["error"]["code"], "E_INVALID_COMMAND");
    assert!(
        json["error"]["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|suggestion| suggestion.as_str().unwrap().contains("emulator"))
    );
}

#[test]
fn schema_list_and_show_are_available() {
    let output = binary()
        .args(["schema", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    for expected in ["androidforge.schema.json", "performance.schema.json"] {
        assert!(
            json["result"]["schemas"]
                .as_array()
                .unwrap()
                .iter()
                .any(|schema| schema == expected),
            "missing schema {expected}"
        );
    }
    binary()
        .args(["schema", "show", "androidforge.schema.json", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("schema_version"));
    binary()
        .args(["schema", "show", "performance.schema.json", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("startup"));
}

#[test]
fn config_validate_and_init_dry_run() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("Project With Spaces (x)");
    std::fs::create_dir_all(dir.join(".androidforge")).unwrap();
    std::fs::write(
        dir.join(".androidforge/project.toml"),
        "adapter_version = 1\n[project]\nname = \"Example\"\npackage = \"com.example.app\"\n",
    )
    .unwrap();
    let output = binary()
        .args([
            "--project",
            dir.to_str().unwrap(),
            "config",
            "validate",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["result"]["valid"], true);

    let init_dir = temp.path().join("init-target");
    std::fs::create_dir_all(&init_dir).unwrap();
    binary()
        .current_dir(&init_dir)
        .args(["init", "--dry-run", "--json"])
        .assert()
        .success();
    assert!(!init_dir.join(".androidforge/project.toml").exists());
}

#[test]
fn capabilities_reports_toolchain_without_claiming_runtime_verification() {
    let output = binary()
        .args(["capabilities", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["result"]["runtime_verified"], false);
    assert!(json["result"]["capabilities"].as_array().unwrap().len() > 5);
}

#[test]
fn doctor_returns_json_even_when_optional_tools_are_missing() {
    let output = binary()
        .args(["doctor", "--json"])
        .output()
        .expect("doctor");
    let json: Value = serde_json::from_slice(&output.stdout).expect("doctor JSON");
    assert_eq!(json["schema_version"], 1);
    assert!(json["result"]["checks"].as_array().unwrap().len() > 3);
    // Missing optional tools are warnings; missing required ADB yields exit 3.
    assert!(matches!(output.status.code(), Some(0 | 3)));
}

#[test]
fn run_end_derives_failure_from_failed_steps() {
    let temp = tempfile::tempdir().unwrap();
    let output = binary()
        .current_dir(temp.path())
        .args(["run", "start", "--name", "failure-check", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let started: Value = serde_json::from_slice(&output).unwrap();
    let run_id = started["run_id"].as_str().unwrap();
    let report_path = temp
        .path()
        .join(".artifacts")
        .join("runs")
        .join(run_id)
        .join("report.json");
    let mut report: Value = serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    report["steps"] = serde_json::json!([{
        "id": "step-001",
        "name": "intentional failure",
        "command": "ui assert",
        "status": "failed",
        "exit_code": 7,
        "started_at": "2026-11-15T10:00:00.000Z",
        "ended_at": "2026-11-15T10:00:01.000Z",
        "duration_ms": 1000,
        "failure": {
            "code": "E_UI_ASSERTION_FAILED",
            "message": "expected failure",
            "failed_operation": "intentional failure",
            "recoverable": true
        },
        "artifacts": [],
        "details": {},
        "retries": 0
    }]);
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();

    let output = binary()
        .current_dir(temp.path())
        .args(["run", "end", "--json"])
        .output()
        .expect("run end");
    assert_eq!(output.status.code(), Some(6));
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["status"], "error");
    assert_eq!(error["error"]["code"], "E_COMMAND_FAILED");

    let final_report: Value =
        serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(final_report["status"], "failed");
    assert_eq!(final_report["exit_code"], 6);
}

#[test]
fn run_end_with_explicit_run_clears_the_active_pointer() {
    let temp = tempfile::tempdir().unwrap();
    let output = binary()
        .current_dir(temp.path())
        .env_remove("ANDROIDFORGE_RUN_ID")
        .args(["run", "start", "--name", "pointer-check", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let started: Value = serde_json::from_slice(&output).unwrap();
    let run_id = started["run_id"].as_str().unwrap().to_string();

    binary()
        .current_dir(temp.path())
        .env_remove("ANDROIDFORGE_RUN_ID")
        .args(["run", "end", "--run", &run_id, "--json"])
        .assert()
        .success();

    let output = binary()
        .current_dir(temp.path())
        .env_remove("ANDROIDFORGE_RUN_ID")
        .args(["run", "status", "--json"])
        .output()
        .expect("run status");
    assert_eq!(output.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "E_INVALID_ARGUMENT");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no active run")
    );
}

#[test]
fn process_cwd_with_unicode_and_parentheses() {
    // This is a smoke test of the Windows process boundary without touching a
    // device: use a native command with a path that contains spaces and Unicode.
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("path with spaces (测试)");
    std::fs::create_dir_all(&path).unwrap();
    let status = StdCommand::new("cmd")
        .current_dir(&path)
        .args(["/C", "echo", "ok"])
        .status()
        .unwrap();
    assert!(status.success());
}

fn project_with_defaults(dir: &std::path::Path) -> std::path::PathBuf {
    let config_dir = dir.join(".androidforge");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("project.toml"),
        "adapter_version = 1\n[project]\nname = \"ContextTest\"\npackage = \"com.example.contexttest\"\ndefault_avd = \"Test_AVD\"\n",
    )
    .unwrap();
    config_dir
}

#[test]
fn context_returns_partial_structured_result_without_a_session() {
    let temp = tempfile::tempdir().unwrap();
    project_with_defaults(temp.path());
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["context", "--json"])
        .output()
        .expect("context");
    assert!(output.status.success(), "{output:?}");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["command"], "context");
    assert_eq!(json["result"]["project"]["available"], true);
    assert_eq!(json["result"]["project"]["name"], "ContextTest");
    assert_eq!(json["result"]["project"]["valid"], true);
    assert_eq!(json["result"]["session"]["available"], false);
    assert_eq!(json["result"]["device"]["available"], false);
    assert_eq!(json["result"]["screen"]["available"], false);
    let actions = json["result"]["next_actions"].as_array().unwrap();
    assert!(!actions.is_empty());
    assert!(actions.iter().any(|action| {
        action["command"]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "Test_AVD")
    }));
    for action in actions {
        for arg in action["command"]["argv"].as_array().unwrap() {
            let arg = arg.as_str().unwrap();
            assert!(!arg.contains('<') && !arg.contains('>'), "{arg}");
        }
    }
}

#[test]
fn doctor_fix_plan_is_read_only_and_structured() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = project_with_defaults(temp.path());
    let config_path = config_dir.join("project.toml");
    let before = std::fs::read(&config_path).unwrap();
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["doctor", "fix", "--plan", "--json"])
        .output()
        .expect("doctor fix");
    assert!(output.status.success(), "{output:?}");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["command"], "doctor.fix");
    assert_eq!(json["result"]["read_only"], true);
    assert_eq!(json["result"]["mutations_performed"], serde_json::json!([]));
    assert_eq!(json["result"]["check_only"], false);
    assert!(json["result"]["issues"].is_array());
    assert_eq!(std::fs::read(&config_path).unwrap(), before);
    assert!(!temp.path().join(".artifacts").exists());
}

#[test]
fn report_explain_is_deterministic_and_markdown_ready() {
    let temp = tempfile::tempdir().unwrap();
    project_with_defaults(temp.path());
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["run", "start", "--name", "explain-check", "--json"])
        .output()
        .expect("run start");
    assert!(output.status.success(), "{output:?}");
    let started: Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_id = started["run_id"].as_str().unwrap().to_string();
    let report_path = temp
        .path()
        .join(".artifacts")
        .join("runs")
        .join(&run_id)
        .join("report.json");
    let mut report: Value = serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    report["status"] = serde_json::json!("failed");
    report["failure"] = serde_json::json!({
        "code": "E_SELECTOR_NOT_FOUND",
        "message": "no UI element matched the requested selector",
        "failed_operation": "ui.tap",
        "recoverable": true,
        "suggestions": [],
        "context": {"selector": {"text": "Save"}},
        "artifacts": []
    });
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();

    let first = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["report", "explain", &run_id, "--json"])
        .output()
        .expect("explain");
    assert!(first.status.success(), "{first:?}");
    let first_json: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first_json["result"]["diagnosis"]["category"], "selector");
    assert_eq!(first_json["result"]["diagnosis"]["confidence"], "high");
    assert!(
        first_json["result"]["fingerprint"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(
        first_json["result"]["next_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["command"]["argv"]
                .as_array()
                .unwrap()
                .iter()
                .any(|arg| arg == "observe"))
    );

    let second = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["report", "explain", &run_id, "--json"])
        .output()
        .expect("explain again");
    let second_json: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(
        first_json["result"]["fingerprint"],
        second_json["result"]["fingerprint"]
    );

    binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["report", "explain", &run_id, "--markdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("# Failure explanation"))
        .stdout(predicate::str::contains("## Next actions"));
}

#[test]
fn session_ensure_dry_run_creates_no_session_state() {
    let temp = tempfile::tempdir().unwrap();
    project_with_defaults(temp.path());
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args([
            "session",
            "ensure",
            "--avd",
            "Test_AVD",
            "--owner",
            "TestAgent",
            "--ttl",
            "30m",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("ensure dry run");
    assert!(output.status.success(), "{output:?}");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["command"], "session.ensure");
    assert_eq!(json["result"]["dry_run"], true);
    assert_eq!(json["result"]["lease_ttl_seconds"], 1800);
    assert_eq!(json["result"]["owner"], "TestAgent");
    assert!(!temp.path().join("state").join("sessions").exists());
    assert!(!temp.path().join(".androidforge/current-session").exists());
}

#[test]
fn ui_observe_without_a_session_is_a_clear_session_error() {
    let temp = tempfile::tempdir().unwrap();
    project_with_defaults(temp.path());
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["ui", "observe", "--json"])
        .output()
        .expect("ui observe");
    assert_eq!(output.status.code(), Some(4));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "error");
    assert_eq!(json["error"]["code"], "E_SESSION_NOT_FOUND");
}

#[test]
fn agent_test_dry_run_is_transparent_and_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join(".androidforge");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("project.toml"),
        "adapter_version = 1\n[project]\nname = \"NoTarget\"\npackage = \"com.example.notarget\"\n",
    )
    .unwrap();
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args([
            "agent",
            "test",
            "--flow",
            "conformance",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("agent dry run");
    assert_eq!(output.status.code(), Some(8));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["command"], "agent.test");
    assert_eq!(json["status"], "blocked");
    assert_eq!(json["result"]["dry_run"], true);
    assert_eq!(json["result"]["final_status"], "blocked");
    assert!(
        !json["result"]["blocking_issues"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let stages = json["result"]["stages"].as_array().unwrap();
    assert!(stages.iter().any(|stage| stage["status"] == "planned"));
    assert!(!temp.path().join("state/agent-tests").exists());
}

#[test]
fn agent_test_failure_uses_a_failed_envelope_status() {
    let temp = tempfile::tempdir().unwrap();
    project_with_defaults(temp.path());
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args([
            "agent",
            "test",
            "--apk",
            "missing.apk",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("agent test failure");
    assert_eq!(output.status.code(), Some(6));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["command"], "agent.test");
    assert_eq!(json["status"], "failed");
    assert_eq!(json["exit_code"], 6);
    assert_eq!(json["result"]["final_status"], "failed");
    assert_eq!(json["result"]["failure"]["stage"], "inspect_apk");
}

#[test]
fn report_explain_latest_finds_failures_across_project_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("ProjectWithAdapter");
    project_with_defaults(&project);

    let start = |name: &str, project: Option<&std::path::Path>| {
        let mut command = binary();
        command
            .current_dir(temp.path())
            .env("ANDROIDFORGE_HOME", temp.path());
        if let Some(project) = project {
            command.args(["--project", project.to_str().unwrap()]);
        }
        let output = command
            .args(["run", "start", "--name", name, "--json"])
            .output()
            .expect("run start");
        assert!(output.status.success(), "{output:?}");
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        json["run_id"].as_str().unwrap().to_string()
    };

    let failed_id = start("root-failure", None);
    let passed_id = start("project-pass", Some(&project));

    let failed_report = temp
        .path()
        .join(".artifacts/runs")
        .join(&failed_id)
        .join("report.json");
    let mut report: Value =
        serde_json::from_slice(&std::fs::read(&failed_report).unwrap()).unwrap();
    report["status"] = serde_json::json!("failed");
    report["failure"] = serde_json::json!({
        "code": "E_SELECTOR_NOT_FOUND",
        "message": "no UI element matched",
        "failed_operation": "ui.tap",
        "recoverable": true,
        "suggestions": [],
        "context": {"selector": {"text": "Save"}},
        "artifacts": []
    });
    std::fs::write(&failed_report, serde_json::to_vec_pretty(&report).unwrap()).unwrap();

    let passed_report = project
        .join(".artifacts/runs")
        .join(&passed_id)
        .join("report.json");
    let mut report: Value =
        serde_json::from_slice(&std::fs::read(&passed_report).unwrap()).unwrap();
    report["status"] = serde_json::json!("passed");
    std::fs::write(&passed_report, serde_json::to_vec_pretty(&report).unwrap()).unwrap();

    let latest = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args([
            "--project",
            project.to_str().unwrap(),
            "report",
            "explain",
            "latest",
            "--json",
        ])
        .output()
        .expect("explain latest");
    assert!(latest.status.success(), "{latest:?}");
    let latest_json: Value = serde_json::from_slice(&latest.stdout).unwrap();
    assert_eq!(latest_json["result"]["run_id"], failed_id);
    assert_eq!(latest_json["result"]["diagnosis"]["category"], "selector");

    let by_id = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args([
            "--project",
            project.to_str().unwrap(),
            "report",
            "explain",
            &failed_id,
            "--json",
        ])
        .output()
        .expect("explain by id");
    assert!(by_id.status.success(), "{by_id:?}");
    let by_id_json: Value = serde_json::from_slice(&by_id.stdout).unwrap();
    assert_eq!(by_id_json["result"]["run_id"], failed_id);
}

fn write_memory_report(
    root: &std::path::Path,
    run_id: &str,
    status: &str,
    started_at: &str,
    artifacts: serde_json::Value,
) {
    let dir = root.join(".artifacts").join("runs").join(run_id);
    std::fs::create_dir_all(&dir).unwrap();
    let failure = if status == "passed" {
        serde_json::Value::Null
    } else {
        serde_json::json!({
            "code": "E_UI_ASSERTION_FAILED",
            "message": "expected dialog did not appear",
            "failed_operation": "ui.tap",
            "recoverable": true,
            "suggestions": [],
            "context": {},
            "artifacts": artifacts.clone()
        })
    };
    let step_status = if status == "passed" {
        "success"
    } else {
        "failed"
    };
    let report = serde_json::json!({
        "schema_version": 1,
        "run_id": run_id,
        "command": "conformance.run",
        "project": "RecurrenceTest",
        "project_root": androidforge_core::util::canonicalize_or_self(root).to_string_lossy(),
        "build": {
            "package": "com.example.recurrence",
            "version_name": "1.0",
            "version_code": 1,
            "apk_sha256": "aaaa"
        },
        "device_serial": "emulator-5554",
        "device": {"serial": "emulator-5554", "state": "device", "kind": "emulator", "model": "Pixel", "api_level": 35},
        "status": status,
        "exit_code": if status == "passed" { 0 } else { 6 },
        "started_at": started_at,
        "ended_at": started_at,
        "duration_ms": 10,
        "steps": [{
            "id": "step-001",
            "name": "conformance:release_smoke",
            "command": "conformance release_smoke",
            "status": step_status,
            "exit_code": if status == "passed" { 0 } else { 7 },
            "started_at": started_at,
            "ended_at": started_at,
            "duration_ms": 10,
            "failure": failure.clone(),
            "artifacts": artifacts.clone(),
            "details": {},
            "retries": 0
        }],
        "failure": failure,
        "artifacts": artifacts,
        "recovery_suggestions": []
    });
    std::fs::write(
        dir.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

#[test]
fn recurrence_memory_detects_regression_and_bundle_is_redacted_and_verifiable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Project (Recurrence)");
    project_with_defaults(&root);
    let log_dir = root.join(".artifacts/runs/run-fail-2/logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(
        log_dir.join("app.log"),
        "expected dialog did not appear\npassword=hunter2\npairing code: 123456\n",
    )
    .unwrap();
    let artifacts = serde_json::json!([{
        "kind": "logcat_filtered",
        "path": "logs/app.log",
        "sha256": null,
        "size_bytes": 80,
        "description": "Filtered logcat excerpt"
    }]);
    write_memory_report(
        &root,
        "run-fail-1",
        "failed",
        "2026-01-01T00:00:00Z",
        serde_json::json!([]),
    );
    write_memory_report(
        &root,
        "run-pass",
        "passed",
        "2026-01-02T00:00:00Z",
        serde_json::json!([]),
    );
    write_memory_report(
        &root,
        "run-fail-2",
        "failed",
        "2026-01-03T00:00:00Z",
        artifacts,
    );

    let recurrences = binary()
        .current_dir(&root)
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["report", "recurrences", "--json"])
        .output()
        .expect("recurrences");
    assert!(recurrences.status.success(), "{recurrences:?}");
    let json: Value = serde_json::from_slice(&recurrences.stdout).unwrap();
    assert_eq!(json["result"]["count"], 1);
    assert_eq!(json["result"]["fingerprints"][0]["status"], "regressed");
    assert_eq!(json["result"]["fingerprints"][0]["occurrence_count"], 2);
    assert_eq!(
        json["result"]["fingerprints"][0]["currently_resolved"],
        false
    );
    let fingerprint = json["result"]["fingerprints"][0]["fingerprint"]
        .as_str()
        .unwrap()
        .to_string();

    let history = binary()
        .current_dir(&root)
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["report", "history", "--fingerprint", &fingerprint, "--json"])
        .output()
        .expect("history");
    assert!(history.status.success(), "{history:?}");
    let history_json: Value = serde_json::from_slice(&history.stdout).unwrap();
    assert_eq!(
        history_json["result"]["fingerprint"]["occurrences"][0]["run_id"],
        "run-fail-2"
    );
    assert_eq!(
        history_json["result"]["fingerprint"]["occurrences"][1]["run_id"],
        "run-fail-1"
    );

    let explain = binary()
        .current_dir(&root)
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["report", "explain", "run-fail-2", "--json"])
        .output()
        .expect("explain");
    assert!(explain.status.success(), "{explain:?}");
    let explain_json: Value = serde_json::from_slice(&explain.stdout).unwrap();
    assert_eq!(explain_json["result"]["recurrence"]["status"], "regressed");

    let bundle_path = root.join("evidence.zip");
    let bundle = binary()
        .current_dir(&root)
        .env("ANDROIDFORGE_HOME", temp.path())
        .args([
            "report",
            "bundle",
            "run-fail-2",
            "--output",
            bundle_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("bundle");
    assert!(bundle.status.success(), "{bundle:?}");
    let bundle_json: Value = serde_json::from_slice(&bundle.stdout).unwrap();
    assert_eq!(bundle_json["result"]["verified"], true);
    assert_eq!(
        bundle_json["result"]["manifest"]["checksums_verified"],
        true
    );
    assert!(bundle_path.is_file());

    let file = std::fs::File::open(&bundle_path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    assert!(archive.by_name("manifest.json").is_ok());
    assert!(archive.by_name("summary.json").is_ok());
    assert!(archive.by_name("explanation.md").is_ok());
    assert!(archive.by_name("evidence/001-app.log").is_ok());
    let mut log = String::new();
    archive
        .by_name("evidence/001-app.log")
        .unwrap()
        .read_to_string(&mut log)
        .unwrap();
    assert!(!log.contains("hunter2"));
    assert!(!log.contains("123456"));
    assert!(log.contains("[REDACTED]"));

    let preview_path = root.join("preview.zip");
    let preview = binary()
        .current_dir(&root)
        .env("ANDROIDFORGE_HOME", temp.path())
        .args([
            "report",
            "bundle",
            "run-fail-2",
            "--output",
            preview_path.to_str().unwrap(),
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("bundle preview");
    assert!(preview.status.success(), "{preview:?}");
    assert!(!preview_path.exists());
}

#[test]
fn apk_resolve_rejects_malformed_candidates_and_unknown_variants() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Apk Resolve");
    std::fs::create_dir_all(root.join(".androidforge")).unwrap();
    std::fs::create_dir_all(root.join("app/build/outputs/apk/debug")).unwrap();
    std::fs::write(
        root.join(".androidforge/project.toml"),
        "adapter_version = 1\n[project]\nname = \"Resolve\"\n[build.variants]\ndebug = [\"app/build/outputs/apk/debug/*.apk\"]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("app/build/outputs/apk/debug/broken.apk"),
        b"not an apk",
    )
    .unwrap();
    let malformed = binary()
        .current_dir(&root)
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["apk", "resolve", "--variant", "debug", "--json"])
        .output()
        .expect("resolve malformed");
    assert_eq!(malformed.status.code(), Some(6));
    let json: Value = serde_json::from_slice(&malformed.stdout).unwrap();
    assert_eq!(json["error"]["code"], "E_APK_INVALID");
    assert!(
        json["error"]["context"]["rejected"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );

    let unknown = binary()
        .current_dir(&root)
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["apk", "resolve", "--variant", "release", "--json"])
        .output()
        .expect("resolve unknown");
    assert_eq!(unknown.status.code(), Some(2));
    let json: Value = serde_json::from_slice(&unknown.stdout).unwrap();
    assert_eq!(json["error"]["code"], "E_INVALID_ARGUMENT");
}

#[test]
fn v06_commands_expose_their_new_options_in_help() {
    binary()
        .args(["ui", "tap", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--receipt"))
        .stdout(predicate::str::contains("--receipt-full"));
    binary()
        .args(["agent", "test", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--variant"));
    binary()
        .args(["apk", "resolve", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--variant"));
    binary()
        .args(["apk", "diff", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("installed"));
    binary()
        .args(["report", "recurrences", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--all-projects"));
    binary()
        .args(["report", "bundle", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--include-screenshots"));
    binary()
        .args(["app", "install", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--only-if-changed"));
}

#[test]
fn adapter_accepts_build_variant_configuration_and_rejects_escaping_patterns() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Build Config");
    std::fs::create_dir_all(root.join(".androidforge")).unwrap();
    std::fs::write(
        root.join(".androidforge/project.toml"),
        "adapter_version = 1\n[project]\nname = \"Build\"\n[build]\ndefault_variant = \"debug\"\n[build.variants]\ndebug = [\"app/build/outputs/apk/debug/*.apk\"]\n",
    )
    .unwrap();
    let output = binary()
        .current_dir(&root)
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["config", "validate", "--json"])
        .output()
        .expect("config validate");
    assert!(output.status.success(), "{output:?}");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["result"]["valid"], true);

    std::fs::write(
        root.join(".androidforge/project.toml"),
        "adapter_version = 1\n[project]\nname = \"Build\"\n[build.variants]\ndebug = [\"../outside/*.apk\"]\n",
    )
    .unwrap();
    let output = binary()
        .current_dir(&root)
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["config", "validate", "--json"])
        .output()
        .expect("config validate invalid");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["result"]["valid"], false);
    assert!(
        json["result"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["message"].as_str().unwrap().contains("project root"))
    );
}

#[test]
fn v07_commands_expose_references_matrix_health_and_wireless_options() {
    binary()
        .args(["ui", "tap", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--ref"))
        .stdout(predicate::str::contains("--expect-token"));
    binary()
        .args(["ui", "observe", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--retries"));
    binary()
        .args(["diagnostics", "logcat", "peek", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--retries"));
    binary()
        .args(["agent", "matrix", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--fail-fast"))
        .stdout(predicate::str::contains("--parallel"));
    binary()
        .args(["devices", "pair", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--code-stdin"));
    binary()
        .args(["devices", "health", "--help"])
        .assert()
        .success();
}

#[test]
fn v07_safety_options_are_explicit_without_a_device() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("V07 Safety");
    project_with_defaults(&root);
    let parallel = binary()
        .current_dir(&root)
        .env("ANDROIDFORGE_HOME", temp.path())
        .args([
            "agent",
            "matrix",
            "--flow",
            "conformance",
            "--parallel",
            "2",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("matrix parallel");
    assert_eq!(parallel.status.code(), Some(3));
    let json: Value = serde_json::from_slice(&parallel.stdout).unwrap();
    assert_eq!(json["error"]["code"], "E_UNSUPPORTED_OPERATION");

    let no_targets = binary()
        .current_dir(&root)
        .env("ANDROIDFORGE_HOME", temp.path())
        .args([
            "agent",
            "matrix",
            "--flow",
            "conformance",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("matrix targets");
    assert_eq!(no_targets.status.code(), Some(2));
    let json: Value = serde_json::from_slice(&no_targets.stdout).unwrap();
    assert_eq!(json["error"]["code"], "E_MATRIX_TARGET_INVALID");

    let bad_endpoint = binary()
        .args([
            "devices",
            "pair",
            "0.0.0.0:5555",
            "--code",
            "123456",
            "--json",
        ])
        .output()
        .expect("wireless endpoint");
    assert_eq!(bad_endpoint.status.code(), Some(2));
    let json: Value = serde_json::from_slice(&bad_endpoint.stdout).unwrap();
    assert_eq!(json["error"]["code"], "E_WIRELESS_ENDPOINT_INVALID");
}

#[test]
fn retry_request_is_rejected_on_a_state_changing_command() {
    let output = binary()
        .args(["ui", "tap", "--retries", "2", "--json"])
        .output()
        .expect("retry rejection");
    assert_eq!(output.status.code(), Some(2));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "E_INVALID_COMMAND");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unexpected argument")
    );
}
