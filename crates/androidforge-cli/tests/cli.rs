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
    assert!(
        json["result"]["schemas"]
            .as_array()
            .unwrap()
            .iter()
            .any(|schema| schema == "androidforge.schema.json")
    );
    binary()
        .args(["schema", "show", "androidforge.schema.json", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("schema_version"));
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
