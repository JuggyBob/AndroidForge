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
