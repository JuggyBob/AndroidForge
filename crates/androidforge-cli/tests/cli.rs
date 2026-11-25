use std::io::Read;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

fn binary() -> Command {
    Command::cargo_bin("androidforge").expect("androidforge binary")
}

/// Fake ADB that always reports zero devices. Used by top-level tests that call
/// `context`/`agent brief` so no real ADB daemon is spawned (a daemon would
/// inherit the captured stdio handles and keep `Command::output()` open).
#[cfg(windows)]
fn fake_empty_adb(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("fake-adb.cmd");
    std::fs::write(
        &path,
        "@echo off\r\necho List of devices attached\r\necho.\r\nexit /b 0\r\n",
    )
    .expect("fake adb");
    path
}

#[cfg(not(windows))]
fn fake_empty_adb(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("fake-adb");
    std::fs::write(
        &path,
        "#!/bin/sh\nprintf 'List of devices attached\\n\\n'\nexit 0\n",
    )
    .expect("fake adb");
    let mut permissions = std::fs::metadata(&path)
        .expect("fake adb metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("fake adb permissions");
    path
}

fn ndjson_values(bytes: &[u8]) -> Vec<Value> {
    let text = std::str::from_utf8(bytes).expect("utf8 NDJSON");
    text.lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("invalid NDJSON line {line:?}: {error}"))
        })
        .collect()
}

fn stats_from_stderr(bytes: &[u8]) -> Value {
    let text = String::from_utf8_lossy(bytes);
    let lines = text
        .lines()
        .filter(|line| line.starts_with("{\"stats\":"))
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 1, "expected exactly one stats line: {text}");
    serde_json::from_str(lines[0]).expect("stats JSON")
}

fn assert_stats_match(stats: &Value, stdout: &[u8]) {
    let text = std::str::from_utf8(stdout).expect("utf8 stdout");
    let lines = text.lines().count();
    assert_eq!(stats["stats"]["schema_version"], 1);
    assert_eq!(stats["stats"]["token_method"], "bytes_div_4");
    assert_eq!(
        stats["stats"]["stdout_bytes"],
        serde_json::json!(stdout.len())
    );
    assert_eq!(
        stats["stats"]["estimated_tokens"],
        serde_json::json!(stdout.len().div_ceil(4))
    );
    assert_eq!(stats["stats"]["lines"], serde_json::json!(lines));
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
    assert!(json.get("summary").is_none());
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
    for expected in [
        "androidforge.schema.json",
        "performance.schema.json",
        "agent-brief.schema.json",
    ] {
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
        "adapter_version = 1
[project]
name = \"Example\"
package = \"com.example.app\"
",
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
        "adapter_version = 1
[project]
name = \"ContextTest\"
package = \"com.example.contexttest\"
default_avd = \"Test_AVD\"
",
    )
    .unwrap();
    config_dir
}

#[test]
fn output_profiles_minify_and_explain_without_extra_whitespace() {
    let output = binary()
        .args(["--output-profile", "compact", "schema", "list", "--json"])
        .output()
        .expect("compact schema list");
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("{\"schema_version\""));
    assert!(
        !text.contains(
            "
  \""
        ),
        "compact output should be minified"
    );
    let json: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["profile"], "compact");
    assert!(json["summary"].as_str().is_some());
}

#[test]
fn full_output_remains_the_default_without_agent_metadata() {
    let output = binary()
        .args(["schema", "list", "--json"])
        .output()
        .expect("full schema list");
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("profile").is_none());
    assert!(json.get("summary").is_none());
    assert!(json.get("next").is_none());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains(
            "
  \""
        ),
        "full output should remain pretty-printed"
    );
}

#[test]
fn agent_profile_can_come_from_project_config_and_cli_overrides_it() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = project_with_defaults(temp.path());
    std::fs::write(
        config_dir.join("project.toml"),
        "adapter_version = 1
[project]
name = \"ContextTest\"
[agent]
profile = \"brief\"
max_items = 7
max_text_chars = 64
",
    )
    .unwrap();
    let configured = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["schema", "list", "--json"])
        .output()
        .expect("configured profile");
    assert!(configured.status.success());
    let json: Value = serde_json::from_slice(&configured.stdout).unwrap();
    assert_eq!(json["profile"], "brief");

    let overridden = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["schema", "list", "--output-profile", "full", "--json"])
        .output()
        .expect("full override");
    assert!(overridden.status.success());
    let json: Value = serde_json::from_slice(&overridden.stdout).unwrap();
    assert!(json.get("profile").is_none());

    let environment = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .env("ANDROIDFORGE_PROFILE", "compact")
        .args(["schema", "list", "--json"])
        .output()
        .expect("environment profile");
    assert!(environment.status.success());
    let json: Value = serde_json::from_slice(&environment.stdout).unwrap();
    assert_eq!(json["profile"], "compact");
}
#[test]
fn brief_context_is_smaller_and_preserves_next_actions() {
    let temp = tempfile::tempdir().unwrap();
    project_with_defaults(temp.path());
    let full = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["context", "--json"])
        .output()
        .expect("full context");
    let brief = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["context", "--output-profile", "brief", "--json"])
        .output()
        .expect("brief context");
    assert!(full.status.success(), "{full:?}");
    assert!(brief.status.success(), "{brief:?}");
    let brief_json: Value = serde_json::from_slice(&brief.stdout).unwrap();
    assert_eq!(brief_json["profile"], "brief");
    assert_eq!(brief_json["result"]["project"]["name"], "ContextTest");
    assert!(brief_json["result"].get("environment").is_none());
    assert!(
        brief_json["result"]["next_actions"]
            .as_array()
            .is_some_and(|actions| !actions.is_empty())
    );
    assert!(
        brief.stdout.len() * 2 < full.stdout.len(),
        "brief context should be materially smaller (full={}, brief={})",
        full.stdout.len(),
        brief.stdout.len()
    );
}

#[test]
fn agent_brief_is_a_small_decision_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    project_with_defaults(temp.path());
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["agent", "brief", "--json"])
        .output()
        .expect("agent brief");
    assert!(output.status.success(), "{output:?}");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["command"], "agent.brief");
    assert_eq!(json["profile"], "brief");
    assert!(
        json["summary"]
            .as_str()
            .unwrap()
            .starts_with("Agent brief:")
    );
    assert!(json["result"].get("environment").is_none());
    assert_eq!(json["result"]["project"]["name"], "ContextTest");
    assert!(json["result"].get("session").is_some());
    assert!(json["result"].get("device").is_some());
    assert!(json["result"].get("screen").is_some());
}

#[test]
fn short_command_aliases_resolve_to_the_canonical_commands() {
    for args in [
        vec!["ui", "o", "--help"],
        vec!["ui", "d", "--help"],
        vec!["ui", "t", "--help"],
        vec!["ui", "w", "--help"],
        vec!["app", "i", "--help"],
        vec!["app", "l", "--help"],
        vec!["diagnostics", "perf", "--help"],
        vec!["report", "why", "--help"],
        vec!["agent", "b", "--help"],
        vec!["ui", "find", "--id", "com.example:id/save", "--help"],
        vec!["ui", "find", "--desc", "Save", "--help"],
        vec!["ui", "wait", "--ref", "el-1", "--token", "obs-1", "--help"],
    ] {
        binary().args(args.clone()).assert().success();
    }
}

#[test]
fn brief_errors_keep_the_profile_and_bound_suggestions() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing.apk");
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args([
            "apk",
            "inspect",
            &missing.display().to_string(),
            "--output-profile",
            "brief",
            "--json",
        ])
        .output()
        .expect("brief error");
    assert_eq!(output.status.code(), Some(6));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["profile"], "brief");
    assert_eq!(json["status"], "error");
    assert_eq!(json["error"]["code"], "E_FILE_NOT_FOUND");
    assert!(json["summary"].as_str().is_some());
    assert!(
        json["error"]["suggestions"]
            .as_array()
            .is_some_and(|suggestions| suggestions.len() <= 2)
    );
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

fn write_synthetic_session(home: &std::path::Path, session: &androidforge_core::session::Session) {
    let dir = home.join("state").join("sessions").join(&session.id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("session.json"),
        serde_json::to_vec_pretty(session).unwrap(),
    )
    .unwrap();
}

fn select_synthetic_session(home: &std::path::Path, id: &str) {
    std::fs::write(home.join(".androidforge").join("current-session"), id).unwrap();
}

fn advisory_session(id: &str, owner: &str) -> androidforge_core::session::Session {
    let mut session = androidforge_core::session::Session::new(
        owner,
        androidforge_core::session::SessionKind::Attached,
        Some("emulator-5554".to_string()),
        None,
        None,
        true,
    );
    session.id = id.to_string();
    session.state = androidforge_core::session::SessionState::Ready;
    session
}

#[test]
fn context_surfaces_stale_and_expired_advisories_without_a_device() {
    let temp = tempfile::tempdir().unwrap();
    project_with_defaults(temp.path());
    let mut session = advisory_session("ses-advisory", "alice");
    session.heartbeat_at = (chrono::Utc::now() - chrono::Duration::hours(3)).to_rfc3339();
    session.lease_expires_at = Some(
        (chrono::Utc::now() - chrono::Duration::minutes(1))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    );
    write_synthetic_session(temp.path(), &session);
    select_synthetic_session(temp.path(), &session.id);

    let adb = fake_empty_adb(temp.path());
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .env("ANDROIDFORGE_ADB", &adb)
        .args(["context", "--no-screen", "--json"])
        .output()
        .expect("context");
    assert!(output.status.success(), "{output:?}");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let result = &json["result"];
    assert_eq!(result["session"]["stale"], true);
    assert_eq!(result["session"]["lease_expired"], true);
    assert!(
        result["session"]["heartbeat_age_seconds"]
            .as_i64()
            .is_some_and(|age| age > 2 * 60 * 60)
    );
    let warnings = result["warnings"].as_array().unwrap();
    assert!(warnings.iter().any(|warning| {
        warning
            .as_str()
            .is_some_and(|text| text.contains("heartbeat") && text.contains("ses-advisory"))
    }));
    assert!(warnings.iter().any(|warning| {
        warning
            .as_str()
            .is_some_and(|text| text.contains("expired advisory lease"))
    }));
}

#[test]
fn agent_brief_surfaces_busy_and_device_in_use_warnings() {
    let temp = tempfile::tempdir().unwrap();
    project_with_defaults(temp.path());

    let mut owner = advisory_session("ses-owner", "");
    owner.lease_expires_at = Some(
        (chrono::Utc::now() + chrono::Duration::hours(1))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    );
    write_synthetic_session(temp.path(), &owner);
    let lock = androidforge_core::session::DeviceLock {
        schema_version: 1,
        session_id: owner.id.clone(),
        serial: "emulator-5554".to_string(),
        owner: String::new(),
        acquired_at: chrono::Utc::now().to_rfc3339(),
    };
    let lock_dir = temp.path().join("state").join("locks").join("devices");
    std::fs::create_dir_all(&lock_dir).unwrap();
    std::fs::write(
        lock_dir.join("emulator-5554.json"),
        serde_json::to_vec_pretty(&lock).unwrap(),
    )
    .unwrap();

    let mut session = advisory_session("ses-current", "alice");
    session.operation = Some(androidforge_core::session::OperationLock {
        pid: std::process::id(),
        operation: "ui.tap".to_string(),
        started_at: chrono::Utc::now().to_rfc3339(),
    });
    write_synthetic_session(temp.path(), &session);
    select_synthetic_session(temp.path(), &session.id);

    let adb = fake_empty_adb(temp.path());
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .env("ANDROIDFORGE_ADB", &adb)
        .args(["agent", "brief", "--json"])
        .output()
        .expect("agent brief");
    assert!(output.status.success(), "{output:?}");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["command"], "agent.brief");
    let warnings = json["result"]["warnings"].as_array().unwrap();
    assert!(warnings.iter().any(|warning| {
        warning
            .as_str()
            .is_some_and(|text| text.contains("active 'ui.tap' operation"))
    }));
    assert!(warnings.iter().any(|warning| {
        warning.as_str().is_some_and(|text| {
            text.contains("Device emulator-5554 is in use by session ses-owner")
                && text.contains("no owner recorded")
                && text.contains("until")
        })
    }));
    assert!(
        json["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("warning(s)"))
    );
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
fn run_resume_preserves_manual_failure_history() {
    let temp = tempfile::tempdir().unwrap();
    project_with_defaults(temp.path());
    let run = |args: &[&str]| {
        binary()
            .current_dir(temp.path())
            .env("ANDROIDFORGE_HOME", temp.path())
            .args(args)
            .output()
            .expect("run command")
    };

    let start = run(&["run", "start", "--name", "manual-failure", "--json"]);
    assert!(start.status.success(), "{start:?}");
    let started: Value = serde_json::from_slice(&start.stdout).unwrap();
    let run_id = started["run_id"].as_str().unwrap().to_string();

    let checkpoint = run(&["run", "checkpoint", "cp1", "--run", &run_id, "--json"]);
    assert!(checkpoint.status.success(), "{checkpoint:?}");

    let end = run(&[
        "run", "end", "--run", &run_id, "--status", "failed", "--json",
    ]);
    assert_eq!(end.status.code(), Some(6), "{end:?}");

    let resume = run(&[
        "run",
        "resume",
        &run_id,
        "--from-checkpoint",
        "cp1",
        "--json",
    ]);
    assert!(resume.status.success(), "{resume:?}");

    let report_path = temp
        .path()
        .join(".artifacts")
        .join("runs")
        .join(&run_id)
        .join("report.json");
    let report: Value = serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    let history = report["failure_history"].as_array().unwrap();
    assert_eq!(history.len(), 1, "{report}");
    assert_eq!(history[0]["code"], "E_COMMAND_FAILED");
    assert_eq!(history[0]["failed_operation"], "run");
    assert_eq!(history[0]["context"]["run_status"], "failed");
    assert_eq!(report["resume_count"], 1);
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
    assert!(matches!(
        json["error"]["code"].as_str(),
        Some("E_SESSION_NOT_FOUND" | "E_DEVICE_NOT_FOUND" | "E_DEVICE_AMBIGUOUS")
    ));
}

#[test]
fn agent_test_dry_run_is_transparent_and_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join(".androidforge");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("project.toml"),
        "adapter_version = 1
[project]
name = \"NoTarget\"
package = \"com.example.notarget\"
",
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
fn report_latest_selection_is_scoped_to_the_project_and_consistent() {
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

    let project_failed_id = start("project-failure", Some(&project));
    let project_passed_id = start("project-pass", Some(&project));
    let other_failed_id = start("other-failure", None);

    let mutate = |path: &std::path::Path, status: &str, started_at: &str, failure: bool| {
        let mut report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        report["status"] = serde_json::json!(status);
        report["started_at"] = serde_json::json!(started_at);
        if failure {
            report["failure"] = serde_json::json!({
                "code": "E_SELECTOR_NOT_FOUND",
                "message": "no UI element matched",
                "failed_operation": "ui.tap",
                "recoverable": true,
                "suggestions": [],
                "context": {"selector": {"text": "Save"}},
                "artifacts": []
            });
        }
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    };
    mutate(
        &project
            .join(".artifacts/runs")
            .join(&project_failed_id)
            .join("report.json"),
        "failed",
        "2026-01-01T00:00:00Z",
        true,
    );
    mutate(
        &project
            .join(".artifacts/runs")
            .join(&project_passed_id)
            .join("report.json"),
        "passed",
        "2026-01-02T00:00:00Z",
        false,
    );
    mutate(
        &temp
            .path()
            .join(".artifacts/runs")
            .join(&other_failed_id)
            .join("report.json"),
        "failed",
        "2026-01-03T00:00:00Z",
        true,
    );

    let run_report = |args: &[&str]| {
        let output = binary()
            .current_dir(temp.path())
            .env("ANDROIDFORGE_HOME", temp.path())
            .args(["--project", project.to_str().unwrap()])
            .args(args)
            .output()
            .expect("report command");
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };

    let latest = run_report(&["report", "explain", "latest", "--json"]);
    assert_eq!(latest["result"]["run_id"], project_passed_id);
    assert_eq!(latest["result"]["status"], "passed");
    assert!(
        latest["result"]["next_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| {
                action["command"]["argv"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|arg| arg == "latest-failure")
            })
    );

    let latest_failure = run_report(&["report", "explain", "latest-failure", "--json"]);
    assert_eq!(latest_failure["result"]["run_id"], project_failed_id);
    assert_eq!(
        latest_failure["result"]["diagnosis"]["category"],
        "selector"
    );

    let bundle_output = temp.path().join("latest-failure.zip");
    let bundle = run_report(&[
        "report",
        "bundle",
        "latest-failure",
        "--output",
        bundle_output.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(bundle["result"]["manifest"]["run_id"], project_failed_id);
    assert!(bundle_output.is_file());

    let by_id = run_report(&["report", "explain", &project_failed_id, "--json"]);
    assert_eq!(by_id["result"]["run_id"], project_failed_id);
}

#[test]
fn report_latest_selection_is_scoped_to_session_and_device() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("ScopedProject");
    project_with_defaults(&project);

    let start = |name: &str| {
        let output = binary()
            .current_dir(temp.path())
            .env("ANDROIDFORGE_HOME", temp.path())
            .args([
                "--project",
                project.to_str().unwrap(),
                "run",
                "start",
                "--name",
                name,
                "--json",
            ])
            .output()
            .expect("run start");
        assert!(output.status.success(), "{output:?}");
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        json["run_id"].as_str().unwrap().to_string()
    };

    let a_fail = start("session-a-failure");
    let a_pass = start("session-a-pass");
    let b_fail = start("session-b-failure");
    let b_pass = start("session-b-pass");

    let patch = |run_id: &str,
                 session: &str,
                 serial: &str,
                 status: &str,
                 started_at: &str,
                 tags: &[&str]| {
        let path = project
            .join(".artifacts/runs")
            .join(run_id)
            .join("report.json");
        let mut report: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        report["session_id"] = serde_json::json!(session);
        report["device_serial"] = serde_json::json!(serial);
        report["status"] = serde_json::json!(status);
        report["started_at"] = serde_json::json!(started_at);
        report["ended_at"] = serde_json::json!(started_at);
        report["tags"] = serde_json::json!(tags);
        if status == "failed" {
            report["failure"] = serde_json::json!({
                "code": "E_SELECTOR_NOT_FOUND",
                "message": "no UI element matched",
                "failed_operation": "ui.tap",
                "recoverable": true,
                "suggestions": [],
                "context": {"selector": {"text": "Save"}},
                "artifacts": []
            });
        }
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    };
    patch(
        &a_fail,
        "ses-a",
        "dev-1",
        "failed",
        "2026-01-01T00:00:00Z",
        &[],
    );
    patch(
        &a_pass,
        "ses-a",
        "dev-1",
        "passed",
        "2026-01-02T00:00:00Z",
        &[],
    );
    patch(
        &b_fail,
        "ses-b",
        "dev-2",
        "failed",
        "2026-01-03T00:00:00Z",
        &[],
    );
    patch(
        &b_pass,
        "ses-b",
        "dev-2",
        "passed",
        "2026-01-04T00:00:00Z",
        &[],
    );

    let run = |args: &[&str], session: Option<&str>, serial: Option<&str>| {
        let mut command = binary();
        command
            .current_dir(temp.path())
            .env("ANDROIDFORGE_HOME", temp.path())
            .env_remove("ANDROIDFORGE_SESSION")
            .env_remove("ANDROIDFORGE_SERIAL")
            .args(["--project", project.to_str().unwrap()]);
        if let Some(session) = session {
            command.args(["--session", session]);
        }
        if let Some(serial) = serial {
            command.args(["--serial", serial]);
        }
        let output = command.args(args).output().expect("report command");
        assert!(output.status.success(), "args {args:?}: {output:?}");
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };

    // Without a filter, selection stays project-wide (the 1.0.0 behavior).
    let latest = run(&["report", "explain", "latest", "--json"], None, None);
    assert_eq!(latest["result"]["run_id"], b_pass);
    assert_eq!(
        latest["result"]["scope"],
        serde_json::json!({"session_id": null, "device_serial": null, "tags": []})
    );
    let latest_failure = run(
        &["report", "explain", "latest-failure", "--json"],
        None,
        None,
    );
    assert_eq!(latest_failure["result"]["run_id"], b_fail);

    // Session scoping never returns the other session's run.
    let a_latest = run(
        &["report", "explain", "latest", "--json"],
        Some("ses-a"),
        None,
    );
    assert_eq!(a_latest["result"]["run_id"], a_pass);
    assert_eq!(a_latest["result"]["scope"]["session_id"], "ses-a");
    let a_failure = run(
        &["report", "explain", "latest-failure", "--json"],
        Some("ses-a"),
        None,
    );
    assert_eq!(a_failure["result"]["run_id"], a_fail);
    let b_failure = run(
        &["report", "explain", "latest-failure", "--json"],
        Some("ses-b"),
        None,
    );
    assert_eq!(b_failure["result"]["run_id"], b_fail);

    // Device scoping never returns the other device's run; filters combine.
    let serial_latest = run(
        &["report", "explain", "latest", "--json"],
        None,
        Some("dev-1"),
    );
    assert_eq!(serial_latest["result"]["run_id"], a_pass);
    assert_eq!(serial_latest["result"]["scope"]["device_serial"], "dev-1");
    let combined = run(
        &["report", "explain", "latest", "--json"],
        Some("ses-b"),
        Some("dev-2"),
    );
    assert_eq!(combined["result"]["run_id"], b_pass);

    // Bundles use the same resolver and therefore the same scope.
    let bundle = run(
        &[
            "report",
            "bundle",
            "latest-failure",
            "--format",
            "json",
            "--json",
        ],
        Some("ses-a"),
        None,
    );
    assert_eq!(bundle["result"]["manifest"]["run_id"], a_fail);
    let bundle_unfiltered = run(
        &[
            "report",
            "bundle",
            "latest-failure",
            "--format",
            "json",
            "--json",
        ],
        None,
        None,
    );
    assert_eq!(bundle_unfiltered["result"]["manifest"]["run_id"], b_fail);

    // An explicit run id is unaffected by latest-selection filters.
    let explicit = run(
        &["report", "explain", &a_fail, "--json"],
        Some("ses-a"),
        None,
    );
    assert_eq!(explicit["result"]["run_id"], a_fail);
    assert_eq!(explicit["result"]["scope"]["session_id"], Value::Null);
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
        "expected dialog did not appear
password=hunter2
pairing code: 123456
",
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
        "adapter_version = 1
[project]
name = \"Resolve\"
[build.variants]
debug = [\"app/build/outputs/apk/debug/*.apk\"]
",
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
        "adapter_version = 1
[project]
name = \"Build\"
[build]
default_variant = \"debug\"
[build.variants]
debug = [\"app/build/outputs/apk/debug/*.apk\"]
",
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
        "adapter_version = 1
[project]
name = \"Build\"
[build.variants]
debug = [\"../outside/*.apk\"]
",
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
    let invalid_parallel = binary()
        .current_dir(&root)
        .env("ANDROIDFORGE_HOME", temp.path())
        .args([
            "agent",
            "matrix",
            "--flow",
            "conformance",
            "--avd",
            "alpha",
            "--parallel",
            "0",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("matrix parallel bounds");
    assert_eq!(invalid_parallel.status.code(), Some(2));

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

/// Local-only fake ADB for dry-run planning. It reports an emulator in
/// `devices -l` and accepts every other invocation without touching hardware.
#[cfg(windows)]
fn performance_fake_adb(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("fake-adb.cmd");
    std::fs::write(
        &path,
        "@echo off\r\n\
         if \"%~1\"==\"devices\" (\r\n\
         echo List of devices attached\r\n\
         echo emulator-5554 device product:sdk_gphone model:Android transport_id:1\r\n\
         echo.\r\n\
         )\r\n\
         exit /b 0\r\n",
    )
    .expect("fake adb");
    path
}

#[cfg(not(windows))]
fn performance_fake_adb(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("fake-adb");
    std::fs::write(
        &path,
        "#!/bin/sh\n\
         if [ \"$1\" = \"devices\" ]; then\n\
         printf 'List of devices attached\\nemulator-5554 device product:sdk_gphone model:Android transport_id:1\\n\\n'\n\
         fi\n\
         exit 0\n",
    )
    .expect("fake adb");
    let mut permissions = std::fs::metadata(&path)
        .expect("fake adb metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("fake adb permissions");
    path
}

#[test]
fn performance_guard_flags_are_declared_with_safe_defaults() {
    let help = binary()
        .args(["diagnostics", "performance", "--help"])
        .output()
        .expect("performance help");
    assert!(help.status.success(), "{help:?}");
    let help = String::from_utf8(help.stdout).expect("help utf8");
    for flag in [
        "--wake",
        "--no-screen-check",
        "--min-frames",
        "--frame-window",
        "--warmup",
    ] {
        assert!(help.contains(flag), "{flag} missing from help:\n{help}");
    }

    let output = binary()
        .args(["tools", "catalog", "--json"])
        .output()
        .expect("tools catalog");
    assert!(output.status.success(), "{output:?}");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let commands = json["result"]["commands"].as_array().expect("commands");
    let command = commands
        .iter()
        .find(|command| command["path"] == "diagnostics performance")
        .expect("diagnostics performance catalog entry");
    assert_eq!(command["safety"], "device-mutation");
    let arguments = command["arguments"].as_array().expect("arguments");
    let argument = |id: &str| {
        arguments
            .iter()
            .find(|argument| argument["id"] == id)
            .unwrap_or_else(|| panic!("missing catalog argument {id}"))
    };
    assert_eq!(argument("min_frames")["default"], "30");
    assert_eq!(argument("wake")["kind"], "boolean");
    assert!(argument("wake")["default"].is_null());
    assert_eq!(argument("no_screen_check")["kind"], "boolean");
    assert_eq!(argument("frame_window")["kind"], "string");
    assert_eq!(argument("warmup")["kind"], "string");
}

#[test]
fn performance_dry_run_reports_guard_defaults_and_explicit_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("Perf Project");
    project_with_defaults(&project);
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let adb = performance_fake_adb(&home);

    let run = |extra: &[&str]| {
        let mut command = binary();
        command
            .current_dir(&project)
            .env("ANDROIDFORGE_HOME", &home)
            .env("ANDROIDFORGE_ADB", &adb)
            .env_remove("ANDROIDFORGE_SESSION")
            .args([
                "--project",
                project.to_str().unwrap(),
                "diagnostics",
                "performance",
                "--serial",
                "emulator-5554",
                "--dry-run",
                "--json",
            ])
            .args(extra);
        let output = command.output().expect("performance dry run");
        assert!(
            output.status.success(),
            "args {extra:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).expect("JSON")
    };

    let defaults = run(&[]);
    assert_eq!(defaults["status"], "success");
    assert_eq!(defaults["result"]["wake"], false);
    assert_eq!(defaults["result"]["require_screen_on"], true);
    assert_eq!(defaults["result"]["min_frames"], 30);
    assert!(defaults["result"]["frame_window_ms"].is_null());
    assert!(defaults["result"]["warmup_ms"].is_null());

    let overrides = run(&[
        "--wake",
        "--no-screen-check",
        "--min-frames",
        "45",
        "--frame-window",
        "5s",
        "--warmup",
        "2s",
    ]);
    assert_eq!(overrides["status"], "success");
    assert_eq!(overrides["result"]["wake"], true);
    assert_eq!(overrides["result"]["require_screen_on"], false);
    assert_eq!(overrides["result"]["min_frames"], 45);
    assert_eq!(overrides["result"]["frame_window_ms"], 5000);
    assert_eq!(overrides["result"]["warmup_ms"], 2000);
}

#[test]
fn frames_launch_and_assert_color_help_catalog_and_mcp_entries_exist() {
    for (args, flags) in [
        (
            vec!["diagnostics", "frames", "--help"],
            vec![
                "--reset",
                "--duration",
                "--baseline",
                "--max-janky",
                "--max-p99",
                "--max-frame-ms",
            ],
        ),
        (
            vec!["capture", "launch", "--help"],
            vec![
                "--fps",
                "--duration",
                "--expect-first-frame",
                "--no-splash",
                "--expected-color",
                "--tolerance",
                "--ui-dump-package",
            ],
        ),
        (
            vec!["capture", "assert-color", "--help"],
            vec!["--rgb", "--at", "--region", "--majority", "--tolerance"],
        ),
    ] {
        let output = binary().args(&args).output().expect("command help");
        assert!(output.status.success(), "help {args:?}: {output:?}");
        let help = String::from_utf8(output.stdout).expect("utf8 help");
        for flag in flags {
            assert!(help.contains(flag), "{flag} missing from {args:?}:\n{help}");
        }
    }

    let catalog = binary()
        .args(["tools", "catalog", "--json"])
        .output()
        .expect("tools catalog");
    assert!(catalog.status.success(), "{catalog:?}");
    let catalog: Value = serde_json::from_slice(&catalog.stdout).expect("catalog JSON");
    let commands = catalog["result"]["commands"].as_array().expect("commands");
    for (path, safety, tool_name) in [
        (
            "diagnostics frames",
            "read-only",
            "androidforge_diagnostics_frames",
        ),
        (
            "capture launch",
            "device-mutation",
            "androidforge_capture_launch",
        ),
        (
            "capture assert-color",
            "read-only",
            "androidforge_capture_assert_color",
        ),
    ] {
        let entry = commands
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("missing catalog entry {path}"));
        assert_eq!(entry["safety"], safety, "{path}");
        assert_eq!(entry["tool_name"], tool_name, "{path}");
    }

    let tools = binary()
        .args(["tools", "schema", "--format", "mcp", "--json"])
        .output()
        .expect("tools schema");
    assert!(tools.status.success(), "{tools:?}");
    let tools: Value = serde_json::from_slice(&tools.stdout).expect("tools JSON");
    let names = tools["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();
    for name in [
        "androidforge_diagnostics_frames",
        "androidforge_capture_launch",
        "androidforge_capture_assert_color",
    ] {
        assert!(names.contains(&name), "missing MCP tool {name}");
    }
}

#[test]
fn frames_dry_run_plans_without_a_device() {
    let temp = tempfile::tempdir().expect("temp home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let output = binary()
        .env("ANDROIDFORGE_HOME", &home)
        .env_remove("ANDROIDFORGE_SESSION")
        .args([
            "diagnostics",
            "frames",
            "com.example",
            "--reset",
            "--duration",
            "5s",
            "--baseline",
            "frames/report.json",
            "--max-janky",
            "2",
            "--max-p99",
            "5",
            "--max-frame-ms",
            "10",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("frames dry run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(json["status"], "success");
    assert_eq!(json["result"]["dry_run"], true);
    assert_eq!(json["result"]["package"], "com.example");
    assert_eq!(json["result"]["reset"], true);
    assert_eq!(json["result"]["duration_ms"], 5000);
    assert_eq!(json["result"]["thresholds"]["jank_percent_pp"], 2.0);
    assert_eq!(json["result"]["thresholds"]["p99_ms"], 5.0);
    assert_eq!(json["result"]["thresholds"]["max_ms"], 10.0);
    assert_eq!(json["result"]["artifact"], "frames/report.json");
    assert!(json["result"]["serial"].is_null());
    assert!(!home.join("runs").exists(), "dry run must not write a run");
}

#[test]
fn capture_launch_dry_run_maps_options_and_rejects_bad_color() {
    let temp = tempfile::tempdir().expect("temp home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let output = binary()
        .env("ANDROIDFORGE_HOME", &home)
        .env_remove("ANDROIDFORGE_SESSION")
        .args([
            "capture",
            "launch",
            "com.example",
            "--fps",
            "12",
            "--duration",
            "5s",
            "--expect-first-frame",
            "--no-splash",
            "--expected-color",
            "#1A73E8",
            "--color-sample",
            "dominant",
            "--tolerance",
            "8",
            "--ui-dump-package",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("capture launch dry run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(json["status"], "success");
    assert_eq!(json["result"]["dry_run"], true);
    assert_eq!(json["result"]["fps"], 12);
    assert_eq!(json["result"]["duration_ms"], 5000);
    assert_eq!(json["result"]["expect_first_frame"], true);
    assert_eq!(json["result"]["no_splash"], true);
    assert_eq!(json["result"]["expected_color"], "#1A73E8");
    assert_eq!(json["result"]["expected_color_sample"], "dominant");
    assert_eq!(json["result"]["tolerance"], 8);
    assert_eq!(json["result"]["use_ui_dump_package"], true);
    assert_eq!(json["result"]["would_launch"], true);

    let invalid = binary()
        .env("ANDROIDFORGE_HOME", &home)
        .args([
            "capture",
            "launch",
            "com.example",
            "--expected-color",
            "not-a-color",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("invalid color");
    assert_eq!(invalid.status.code(), Some(2));
    let invalid: Value = serde_json::from_slice(&invalid.stdout).expect("JSON");
    assert_eq!(invalid["error"]["code"], "E_INVALID_ARGUMENT");
}

#[test]
fn assert_color_dry_run_needs_no_session_and_plans_the_sample() {
    let temp = tempfile::tempdir().expect("temp home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    for (extra, expected_source) in [
        (vec!["--at", "540,1200"], "at"),
        (
            vec!["--region", "0,0,10,20", "--majority"],
            "region_majority",
        ),
        (vec![], "centre"),
    ] {
        let mut args = vec![
            "capture",
            "assert-color",
            "--rgb",
            "#1A73E8",
            "--tolerance",
            "8",
            "--dry-run",
            "--json",
        ];
        args.extend(extra);
        let output = binary()
            .env("ANDROIDFORGE_HOME", &home)
            .env_remove("ANDROIDFORGE_SESSION")
            .args(&args)
            .output()
            .expect("assert-color dry run");
        assert!(
            output.status.success(),
            "args {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: Value = serde_json::from_slice(&output.stdout).expect("JSON");
        assert_eq!(json["status"], "success");
        assert_eq!(json["result"]["dry_run"], true);
        assert_eq!(json["result"]["expected"], "#1A73E8");
        assert_eq!(json["result"]["tolerance"], 8);
        assert_eq!(json["result"]["sample_source"], expected_source);
        assert!(json["result"]["serial"].is_null());
    }

    let invalid = binary()
        .env("ANDROIDFORGE_HOME", &home)
        .args([
            "capture",
            "assert-color",
            "--rgb",
            "#11223344",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("invalid color");
    assert_eq!(invalid.status.code(), Some(2));
    let invalid: Value = serde_json::from_slice(&invalid.stdout).expect("JSON");
    assert_eq!(invalid["error"]["code"], "E_INVALID_ARGUMENT");
}

/// Local-only fake ADB that reports an emulator and canned gfxinfo text. No
/// device is contacted; it exists solely to exercise the frames command end to end.
#[cfg(windows)]
fn frames_fake_adb(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("frames-fake-adb.cmd");
    std::fs::write(
        &path,
        "@echo off\r\n\
         echo %* | findstr /i \"devices\" >nul\r\n\
         if not errorlevel 1 goto devices\r\n\
         echo %* | findstr /i \"gfxinfo\" >nul\r\n\
         if not errorlevel 1 goto gfx\r\n\
         exit /b 0\r\n\
         :devices\r\n\
         echo List of devices attached\r\n\
         echo emulator-5554 device product:sdk_gphone model:Android transport_id:1\r\n\
         echo.\r\n\
         exit /b 0\r\n\
         :gfx\r\n\
         echo Graphics info for pid 1234 [com.example]\r\n\
         echo Total frames rendered: 120\r\n\
         echo Janky frames: 9 (7.50%%)\r\n\
         echo 50th percentile: 8ms\r\n\
         echo 99th percentile: 40ms\r\n\
         echo Max: 72ms\r\n\
         exit /b 0\r\n",
    )
    .expect("fake adb");
    path
}

#[cfg(not(windows))]
fn frames_fake_adb(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("frames-fake-adb");
    std::fs::write(
        &path,
        "#!/bin/sh\n\
         case \"$*\" in\n\
           *devices*) printf 'List of devices attached\\nemulator-5554 device product:sdk_gphone model:Android transport_id:1\\n\\n';;\n\
           *gfxinfo*) printf 'Graphics info for pid 1234 [com.example]\\nTotal frames rendered: 120\\nJanky frames: 9 (7.50%%)\\n50th percentile: 8ms\\n99th percentile: 40ms\\nMax: 72ms\\n';;\n\
         esac\n\
         exit 0\n",
    )
    .expect("fake adb");
    let mut permissions = std::fs::metadata(&path)
        .expect("fake adb metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("fake adb permissions");
    path
}

#[test]
fn frames_with_fake_adb_parses_gfxinfo_and_writes_report_artifact() {
    let temp = tempfile::tempdir().expect("temp");
    let project = temp.path().join("Frames Project");
    project_with_defaults(&project);
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let adb = frames_fake_adb(&home);

    let output = binary()
        .current_dir(&project)
        .env("ANDROIDFORGE_HOME", &home)
        .env("ANDROIDFORGE_ADB", &adb)
        .env_remove("ANDROIDFORGE_SESSION")
        .args([
            "--serial",
            "emulator-5554",
            "diagnostics",
            "frames",
            "com.example",
            "--reset",
            "--json",
        ])
        .output()
        .expect("frames");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(json["status"], "success");
    assert_eq!(json["result"]["package"], "com.example");
    assert_eq!(json["result"]["reset_before_collection"], true);
    assert_eq!(json["result"]["sample"]["total_frames"], 120);
    assert_eq!(json["result"]["sample"]["janky"], 9);
    assert_eq!(json["result"]["sample"]["p99_ms"], 40.0);
    let path = json["result"]["path"].as_str().expect("artifact path");
    assert!(path.ends_with("frames/report.json"), "path: {path}");
    let run_dir = json["result"]["run_dir"].as_str().expect("run dir");
    let report_path = std::path::Path::new(run_dir).join(path);
    assert!(report_path.is_file(), "missing {}", report_path.display());
    let saved: Value = serde_json::from_slice(&std::fs::read(&report_path).expect("report"))
        .expect("saved report JSON");
    assert_eq!(saved["schema_version"], 1);
    assert_eq!(saved["sample"]["max_ms"], 72.0);
}

fn start_run(root: &std::path::Path, project: &std::path::Path, extra: &[&str]) -> String {
    let mut command = binary();
    command
        .current_dir(root)
        .env("ANDROIDFORGE_HOME", root)
        .env_remove("ANDROIDFORGE_SESSION")
        .args([
            "--project",
            project.to_str().unwrap(),
            "run",
            "start",
            "--name",
            "test-run",
            "--json",
        ]);
    command.args(extra);
    let output = command.output().expect("run start");
    assert!(output.status.success(), "{output:?}");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    json["run_id"].as_str().unwrap().to_string()
}

fn patch_report(root: &std::path::Path, run_id: &str, patch: Value) {
    let path = root
        .join(".artifacts")
        .join("runs")
        .join(run_id)
        .join("report.json");
    let mut report: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for (key, value) in patch.as_object().expect("patch object") {
        report[key.clone()] = value.clone();
    }
    std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}

fn report_command(
    root: &std::path::Path,
    project: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    binary()
        .current_dir(root)
        .env("ANDROIDFORGE_HOME", root)
        .env_remove("ANDROIDFORGE_SESSION")
        .env_remove("ANDROIDFORGE_SERIAL")
        .args(["--project", project.to_str().unwrap()])
        .args(args)
        .output()
        .expect("report command")
}

fn report_json(root: &std::path::Path, project: &std::path::Path, args: &[&str]) -> Value {
    let output = report_command(root, project, args);
    assert!(output.status.success(), "{args:?}: {output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn run_start_tags_and_meta_round_trip_and_validate() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("TaggedProject");
    project_with_defaults(&root);

    let run_id = start_run(
        temp.path(),
        &root,
        &[
            "--tag",
            "release",
            "--tag",
            "smoke",
            "--meta",
            "build=42",
            "--meta",
            "branch=main",
        ],
    );
    let report_path = root
        .join(".artifacts/runs")
        .join(&run_id)
        .join("report.json");
    let report: Value = serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report["tags"], serde_json::json!(["release", "smoke"]));
    assert_eq!(
        report["meta"],
        serde_json::json!({"branch": "main", "build": "42"})
    );
    let metadata_path = root
        .join(".artifacts/runs")
        .join(&run_id)
        .join("metadata.json");
    let metadata: Value = serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
    assert_eq!(metadata["tags"], report["tags"]);
    assert_eq!(metadata["meta"], report["meta"]);
    let shown = report_json(temp.path(), &root, &["report", "show", &run_id, "--json"]);
    assert_eq!(shown["result"]["tags"], report["tags"]);
    assert_eq!(shown["result"]["meta"], report["meta"]);

    for invalid in [
        vec!["--meta", "password=hunter2"],
        vec!["--meta", "api-key=abc"],
        vec!["--meta", "no-equals"],
        vec!["--meta", "=value"],
        vec!["--tag", "   "],
    ] {
        let mut command = binary();
        command
            .current_dir(temp.path())
            .env("ANDROIDFORGE_HOME", temp.path())
            .args(["run", "start", "--name", "bad", "--json"]);
        command.args(&invalid);
        let output = command.output().expect("invalid run start");
        assert!(!output.status.success(), "{invalid:?}: {output:?}");
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["status"], "error");
        assert_eq!(json["error"]["code"], "E_INVALID_ARGUMENT");
    }
}

#[test]
fn report_tag_and_since_filters_scope_latest_list_and_bundles() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("FilteredProject");
    project_with_defaults(&root);

    let old_failure = start_run(temp.path(), &root, &[]);
    let tagged_pass = start_run(temp.path(), &root, &[]);
    let untagged_pass = start_run(temp.path(), &root, &[]);
    let fresh = start_run(temp.path(), &root, &[]);
    patch_report(
        &root,
        &old_failure,
        serde_json::json!({
            "status": "failed",
            "started_at": "2026-01-01T00:00:00Z",
            "ended_at": "2026-01-01T00:00:01Z",
            "tags": ["release", "smoke"],
            "failure": {
                "code": "E_SELECTOR_NOT_FOUND",
                "message": "no UI element matched",
                "failed_operation": "ui.tap",
                "recoverable": true,
                "suggestions": [],
                "context": {},
                "artifacts": []
            }
        }),
    );
    patch_report(
        &root,
        &tagged_pass,
        serde_json::json!({
            "status": "passed",
            "started_at": "2026-01-02T00:00:00Z",
            "tags": ["release"]
        }),
    );
    patch_report(
        &root,
        &untagged_pass,
        serde_json::json!({
            "status": "passed",
            "started_at": "2026-01-03T00:00:00Z",
            "tags": []
        }),
    );
    patch_report(
        &root,
        &fresh,
        serde_json::json!({
            "status": "passed",
            "started_at": chrono::Utc::now().to_rfc3339(),
            "tags": ["fresh"]
        }),
    );

    let smoke = report_json(
        temp.path(),
        &root,
        &["report", "explain", "latest", "--tag", "smoke", "--json"],
    );
    assert_eq!(smoke["result"]["run_id"], old_failure);
    assert_eq!(
        smoke["result"]["scope"]["tags"],
        serde_json::json!(["smoke"])
    );
    let smoke_failure = report_json(
        temp.path(),
        &root,
        &[
            "report",
            "explain",
            "latest-failure",
            "--tag",
            "smoke",
            "--json",
        ],
    );
    assert_eq!(smoke_failure["result"]["run_id"], old_failure);
    let release = report_json(
        temp.path(),
        &root,
        &["report", "explain", "latest", "--tag", "release", "--json"],
    );
    assert_eq!(release["result"]["run_id"], tagged_pass);
    let both = report_json(
        temp.path(),
        &root,
        &[
            "report", "explain", "latest", "--tag", "release", "--tag", "smoke", "--json",
        ],
    );
    assert_eq!(both["result"]["run_id"], old_failure);

    let since_absolute = report_json(
        temp.path(),
        &root,
        &[
            "report",
            "explain",
            "latest",
            "--since",
            "2026-01-02T00:00:00Z",
            "--json",
        ],
    );
    assert_eq!(since_absolute["result"]["run_id"], fresh);
    let since_and_tag = report_json(
        temp.path(),
        &root,
        &[
            "report",
            "explain",
            "latest",
            "--tag",
            "release",
            "--since",
            "2026-01-02T00:00:00Z",
            "--json",
        ],
    );
    assert_eq!(since_and_tag["result"]["run_id"], tagged_pass);
    let since_relative = report_json(
        temp.path(),
        &root,
        &["report", "explain", "latest", "--since", "1d", "--json"],
    );
    assert_eq!(since_relative["result"]["run_id"], fresh);

    let listed = report_json(
        temp.path(),
        &root,
        &["report", "list", "--tag", "release", "--json"],
    );
    assert_eq!(listed["result"]["count"], 2);
    let listed_since = report_json(
        temp.path(),
        &root,
        &[
            "report",
            "list",
            "--since",
            "2026-01-02T00:00:00Z",
            "--json",
        ],
    );
    assert_eq!(listed_since["result"]["count"], 3);

    let bundle = report_json(
        temp.path(),
        &root,
        &[
            "report",
            "bundle",
            "latest-failure",
            "--tag",
            "smoke",
            "--format",
            "json",
            "--json",
        ],
    );
    assert_eq!(bundle["result"]["manifest"]["run_id"], old_failure);

    let missing = report_command(
        temp.path(),
        &root,
        &["report", "explain", "latest", "--tag", "missing", "--json"],
    );
    assert!(!missing.status.success());
}

fn performance_patch(startup_ms: u64, janky_percent: f64, p99_ms: f64) -> Value {
    serde_json::json!({
        "status": "passed",
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
                    "percentile_99_ms": p99_ms,
                    "comparable": true,
                    "sample_status": "ok"
                }
            },
            "retries": 0
        }],
        "artifacts": [{
            "kind": "apk",
            "path": "app.apk",
            "sha256": null,
            "size_bytes": 1_000_000,
            "description": null
        }]
    })
}

#[test]
fn report_compare_budgets_pass_fail_and_preserve_no_budget_behavior() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("BudgetProject");
    project_with_defaults(&root);

    let baseline = start_run(temp.path(), &root, &[]);
    let current = start_run(temp.path(), &root, &[]);
    patch_report(
        &root,
        &baseline,
        serde_json::json!({
            "status": "passed",
            "started_at": "2026-01-01T00:00:00Z",
            "steps": performance_patch(500, 5.0, 20.0)["steps"].clone(),
            "artifacts": performance_patch(500, 5.0, 20.0)["artifacts"].clone(),
        }),
    );
    patch_report(
        &root,
        &current,
        serde_json::json!({
            "status": "passed",
            "started_at": "2026-01-02T00:00:00Z",
            "steps": performance_patch(700, 12.0, 50.0)["steps"].clone(),
            "artifacts": performance_patch(700, 12.0, 50.0)["artifacts"].clone(),
        }),
    );

    let plain = report_json(
        temp.path(),
        &root,
        &["report", "compare", &current, "--json"],
    );
    assert_eq!(plain["result"]["baseline_run_id"], baseline);
    assert!(plain["result"].get("budget").is_none());
    assert!(plain["result"].get("budget_violations").is_none());

    let with_budget = report_json(
        temp.path(),
        &root,
        &[
            "report",
            "compare",
            &current,
            "--max-startup-ms",
            "600",
            "--json",
        ],
    );
    assert_eq!(with_budget["result"]["budget_passed"], false);
    assert_eq!(
        with_budget["result"]["budget_violations"][0]["metric"],
        "startup_ms"
    );

    let regression = report_command(
        temp.path(),
        &root,
        &[
            "report",
            "compare",
            &current,
            "--max-startup-ms",
            "600",
            "--max-jank-percent",
            "10",
            "--fail-on-regression",
            "--json",
        ],
    );
    assert_eq!(regression.status.code(), Some(11), "{regression:?}");
    let error: Value = serde_json::from_slice(&regression.stdout).unwrap();
    assert_eq!(error["status"], "error");
    assert_eq!(error["error"]["code"], "E_PERFORMANCE_REGRESSION");
    let violations = error["error"]["context"]["budget_violations"]
        .as_array()
        .expect("structured violations");
    assert!(violations.iter().any(|item| item["metric"] == "startup_ms"));
    assert!(
        violations
            .iter()
            .any(|item| item["metric"] == "jank_percent")
    );
    assert_eq!(
        error["error"]["context"]["comparison"]["current_run_id"],
        serde_json::json!(current)
    );

    let within_budget = report_json(
        temp.path(),
        &root,
        &[
            "report",
            "compare",
            &current,
            "--budget",
            "{\"startup_ms\":800,\"jank_percent\":15,\"max_frame_ms\":60}",
            "--fail-on-regression",
            "--json",
        ],
    );
    assert_eq!(within_budget["result"]["budget_passed"], true);
    assert!(within_budget["result"].get("budget_violations").is_none());

    let no_budget_failure = report_command(
        temp.path(),
        &root,
        &[
            "report",
            "compare",
            &current,
            "--fail-on-regression",
            "--json",
        ],
    );
    assert!(!no_budget_failure.status.success());
}

#[test]
fn legacy_reports_without_tags_or_meta_remain_readable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("LegacyProject");
    project_with_defaults(&root);
    let baseline = start_run(temp.path(), &root, &[]);
    let current = start_run(temp.path(), &root, &[]);
    for run_id in [&baseline, &current] {
        let dir = root.join(".artifacts/runs").join(run_id);
        for name in ["report.json", "metadata.json"] {
            let path = dir.join(name);
            let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            value.as_object_mut().unwrap().remove("tags");
            value.as_object_mut().unwrap().remove("meta");
            std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        }
        patch_report(&root, run_id, serde_json::json!({"status": "passed"}));
    }
    let shown = report_json(temp.path(), &root, &["report", "show", &current, "--json"]);
    assert_eq!(shown["result"]["run_id"], current);
    let explained = report_json(
        temp.path(),
        &root,
        &["report", "explain", &current, "--json"],
    );
    assert_eq!(explained["result"]["run_id"], current);
    let bundled = report_json(
        temp.path(),
        &root,
        &["report", "bundle", &current, "--format", "json", "--json"],
    );
    assert_eq!(bundled["result"]["manifest"]["run_id"], current);
    let compared = report_json(
        temp.path(),
        &root,
        &["report", "compare", &current, "--json"],
    );
    assert_eq!(compared["result"]["baseline_run_id"], baseline);
}

const FAKE_UI_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<hierarchy rotation=\"0\">\n  <node index=\"0\" text=\"Save\" resource-id=\"com.example:id/save\" class=\"android.widget.Button\" package=\"com.example\" content-desc=\"Save changes\" clickable=\"true\" enabled=\"true\" focused=\"false\" scrollable=\"false\" bounds=\"[100,200][300,300]\" />\n</hierarchy>\n";

const COMPOSE_UI_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<hierarchy rotation=\"0\">\n  <node index=\"0\" text=\"Save\" resource-id=\"com.example:id/save\" class=\"android.widget.Button\" package=\"com.example\" content-desc=\"Save changes\" clickable=\"true\" enabled=\"true\" focused=\"false\" scrollable=\"false\" bounds=\"[100,200][300,300]\" />\n  <node index=\"1\" text=\"Checkout\" resource-id=\"checkout_button\" class=\"android.view.View\" package=\"com.example\" content-desc=\"Checkout\" clickable=\"true\" enabled=\"true\" focused=\"false\" scrollable=\"false\" bounds=\"[100,400][500,480]\" />\n</hierarchy>\n";

/// Fake ADB that reports one emulator and returns a canned UIAutomator
/// hierarchy for `exec-out cat`. It never touches a physical device.
#[cfg(windows)]
fn ui_fake_adb(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("fake-adb.cmd");
    std::fs::write(dir.join("ui.xml"), FAKE_UI_XML).expect("fake ui hierarchy");
    std::fs::write(
        &path,
        "@echo off\r\n\
         setlocal enabledelayedexpansion\r\n\
         set \"all=%*\"\r\n\
         echo !all! | findstr /I \"devices\" >nul\r\n\
         if !errorlevel! equ 0 (\r\n\
         echo List of devices attached\r\n\
         echo emulator-5554 device product:sdk_gphone model:Android transport_id:1\r\n\
         echo.\r\n\
         exit /b 0\r\n\
         )\r\n\
         echo !all! | findstr /I \"exec-out\" >nul\r\n\
         if !errorlevel! equ 0 (\r\n\
         type \"%~dp0ui.xml\"\r\n\
         exit /b 0\r\n\
         )\r\n\
         exit /b 0\r\n",
    )
    .expect("fake adb");
    path
}

#[cfg(not(windows))]
fn ui_fake_adb(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("fake-adb");
    std::fs::write(dir.join("ui.xml"), FAKE_UI_XML).expect("fake ui hierarchy");
    std::fs::write(
        &path,
        "#!/bin/sh\n\
         case \"$*\" in\n\
           *devices*)\n\
             printf 'List of devices attached\\nemulator-5554 device product:sdk_gphone model:Android transport_id:1\\n\\n'\n\
             ;;\n\
           *exec-out*)\n\
             cat \"$(dirname \"$0\")/ui.xml\"\n\
             ;;\n\
         esac\n\
         exit 0\n",
    )
    .expect("fake adb");
    let mut permissions = std::fs::metadata(&path)
        .expect("fake adb metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("fake adb permissions");
    path
}

fn ui_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = tempfile::tempdir().expect("temp ui project");
    let project = temp.path().join("UiProject");
    project_with_defaults(&project);
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let adb = ui_fake_adb(&home);
    (temp, project, adb)
}

fn ui_run(
    home: &std::path::Path,
    project: &std::path::Path,
    adb: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    binary()
        .current_dir(project)
        .env("ANDROIDFORGE_HOME", home)
        .env("ANDROIDFORGE_ADB", adb)
        .env_remove("ANDROIDFORGE_SESSION")
        .args([
            "--project",
            project.to_str().unwrap(),
            "--serial",
            "emulator-5554",
        ])
        .args(args)
        .output()
        .expect("ui command")
}

fn ui_json(
    home: &std::path::Path,
    project: &std::path::Path,
    adb: &std::path::Path,
    args: &[&str],
) -> Value {
    let output = ui_run(home, project, adb, args);
    let stdout = String::from_utf8(output.stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "parse {args:?}: {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn ui_attach_session(home: &std::path::Path, project: &std::path::Path) {
    use androidforge_core::session::{Session, SessionKind, SessionState};

    let mut session = Session::new(
        "tester",
        SessionKind::Attached,
        Some("emulator-5554".to_string()),
        None,
        None,
        true,
    );
    session.state = SessionState::Ready;
    let dir = home.join("state").join("sessions").join(&session.id);
    std::fs::create_dir_all(&dir).expect("session dir");
    std::fs::write(
        dir.join("session.json"),
        serde_json::to_vec_pretty(&session).expect("session JSON"),
    )
    .expect("write session");
    std::fs::create_dir_all(project.join(".androidforge")).expect("pointer dir");
    std::fs::write(project.join(".androidforge/current-session"), &session.id).expect("pointer");
}

#[test]
fn ui_observe_semantics_and_test_tag_selectors_with_fake_adb() {
    let (_temp, project, adb) = ui_fixture();
    let home = adb.parent().expect("adb home");
    std::fs::write(home.join("ui.xml"), COMPOSE_UI_XML).expect("compose hierarchy");
    ui_attach_session(home, &project);

    let plain = ui_json(home, &project, &adb, &["ui", "observe", "--json"]);
    assert_eq!(plain["status"], "success", "{plain:#}");
    assert!(plain["result"].get("semantics").is_none());
    assert!(plain["result"].get("semantics_total").is_none());
    assert!(plain["result"].get("semantics_truncated").is_none());

    let semantic = ui_json(
        home,
        &project,
        &adb,
        &["ui", "observe", "--semantics", "--json"],
    );
    assert_eq!(semantic["status"], "success", "{semantic:#}");
    let result = &semantic["result"];
    assert!(result["semantics"].is_array());
    assert!(result["semantics_total"].as_u64().unwrap() >= 2);
    assert!(
        result
            .get("semantics_truncated")
            .is_none_or(|value| value == false)
    );
    let semantics = result["semantics"].as_array().unwrap();
    let compose = semantics
        .iter()
        .find(|node| node["test_tag"] == "checkout_button")
        .expect("compose test tag");
    assert_eq!(compose["resource_id"], "checkout_button");
    assert!(compose["ref"].as_str().unwrap().starts_with("el-"));
    let android = semantics
        .iter()
        .find(|node| node["resource_id"] == "com.example:id/save")
        .expect("android resource id");
    assert!(android.get("test_tag").is_none());

    // Stable refs from the semantic view are usable by later actions.
    let reference = compose["ref"].as_str().unwrap();
    let token = result["token"].as_str().unwrap();
    let tapped = ui_json(
        home,
        &project,
        &adb,
        &[
            "ui",
            "tap",
            "--ref",
            reference,
            "--expect-token",
            token,
            "--json",
        ],
    );
    assert_eq!(tapped["status"], "success", "{tapped:#}");

    // `--semantics` also works with `--full`.
    let full = ui_json(
        home,
        &project,
        &adb,
        &["ui", "observe", "--semantics", "--full", "--json"],
    );
    assert_eq!(full["status"], "success", "{full:#}");
    assert!(full["result"]["nodes"].is_array());
    assert!(full["result"]["semantics"].is_array());

    // Compose test tags map onto the existing resource-id selector path.
    for command in ["tap", "assert"] {
        let output = ui_json(
            home,
            &project,
            &adb,
            &["ui", command, "--test-tag", "checkout_button", "--json"],
        );
        assert_eq!(output["status"], "success", "{command}: {output:#}");
    }
}

#[test]
fn ui_bounds_assert_bounds_and_wait_flags_are_documented() {
    for (args, flags) in [
        (
            vec!["ui", "bounds", "--help"],
            vec!["--text", "--content-desc", "--resource-id", "--region"],
        ),
        (
            vec!["ui", "assert-bounds", "--help"],
            vec![
                "--bottom-y",
                "--bottom-y-gt",
                "--bottom-y-lt",
                "--tolerance",
            ],
        ),
        (
            vec!["ui", "wait", "--help"],
            vec!["--for-bounds-stable", "--stable-for", "--timeout"],
        ),
    ] {
        let output = binary().args(&args).output().expect("command help");
        assert!(output.status.success(), "help {args:?}: {output:?}");
        let help = String::from_utf8(output.stdout).expect("utf8 help");
        for flag in flags {
            assert!(help.contains(flag), "{flag} missing from {args:?}:\n{help}");
        }
    }

    let catalog = binary()
        .args(["tools", "catalog", "--json"])
        .output()
        .expect("tools catalog");
    assert!(catalog.status.success(), "{catalog:?}");
    let catalog: Value = serde_json::from_slice(&catalog.stdout).expect("catalog JSON");
    let commands = catalog["result"]["commands"].as_array().expect("commands");
    for (path, tool_name, flags) in [
        ("ui bounds", "androidforge_ui_bounds", vec![]),
        (
            "ui assert-bounds",
            "androidforge_ui_assert_bounds",
            vec!["bottom-y", "bottom-y-gt", "bottom-y-lt", "tolerance"],
        ),
        (
            "ui wait",
            "androidforge_ui_wait",
            vec!["for-bounds-stable", "stable-for", "timeout"],
        ),
    ] {
        let entry = commands
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("missing catalog entry {path}"));
        assert_eq!(entry["safety"], "read-only", "{path}");
        assert_eq!(entry["tool_name"], tool_name, "{path}");
        let arguments = entry["arguments"].as_array().expect("arguments");
        for flag in flags {
            assert!(
                arguments.iter().any(|argument| argument["long"] == flag),
                "{path} is missing --{flag}"
            );
        }
    }

    let tools = binary()
        .args(["tools", "schema", "--format", "mcp", "--json"])
        .output()
        .expect("tools schema");
    assert!(tools.status.success(), "{tools:?}");
    let tools: Value = serde_json::from_slice(&tools.stdout).expect("tools JSON");
    let definitions = tools["result"]["tools"].as_array().expect("tools");
    let find_tool = |name: &str| {
        definitions
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing MCP tool {name}"))
            .clone()
    };
    let bounds = find_tool("androidforge_ui_bounds");
    assert!(bounds["inputSchema"]["properties"]["text"].is_object());
    let assert_bounds = find_tool("androidforge_ui_assert_bounds");
    for key in ["bottom_y", "bottom_y_gt", "bottom_y_lt", "tolerance"] {
        assert!(
            assert_bounds["inputSchema"]["properties"][key].is_object(),
            "missing MCP property {key}"
        );
    }
    let wait = find_tool("androidforge_ui_wait");
    assert!(wait["inputSchema"]["properties"]["for_bounds_stable"].is_object());
}

#[test]
fn ui_bounds_returns_all_eight_fields_with_fake_adb() {
    let (_temp, project, adb) = ui_fixture();
    let home = adb.parent().expect("adb home");
    let json = ui_json(
        home,
        &project,
        &adb,
        &["ui", "bounds", "--text", "Save", "--json"],
    );
    assert_eq!(json["status"], "success", "{json:#}");
    assert_eq!(json["command"], "ui.bounds");
    let result = &json["result"];
    assert_eq!(result["left"], 100);
    assert_eq!(result["top"], 200);
    assert_eq!(result["right"], 300);
    assert_eq!(result["bottom"], 300);
    assert_eq!(result["width"], 200);
    assert_eq!(result["height"], 100);
    assert_eq!(result["center_x"], 200);
    assert_eq!(result["center_y"], 250);
}

#[test]
fn ui_assert_bounds_relations_and_failure_context_with_fake_adb() {
    let (_temp, project, adb) = ui_fixture();
    let home = adb.parent().expect("adb home");

    let eq = ui_json(
        home,
        &project,
        &adb,
        &[
            "ui",
            "assert-bounds",
            "--text",
            "Save",
            "--bottom-y",
            "300",
            "--tolerance",
            "0",
            "--json",
        ],
    );
    assert_eq!(eq["status"], "success", "{eq:#}");
    assert_eq!(eq["result"]["relation"], "eq");
    assert_eq!(eq["result"]["passed"], true);

    let gt = ui_json(
        home,
        &project,
        &adb,
        &[
            "ui",
            "assert-bounds",
            "--text",
            "Save",
            "--bottom-y-gt",
            "299",
            "--json",
        ],
    );
    assert_eq!(gt["status"], "success", "{gt:#}");
    assert_eq!(gt["result"]["relation"], "gt");

    let lt = ui_json(
        home,
        &project,
        &adb,
        &[
            "ui",
            "assert-bounds",
            "--text",
            "Save",
            "--bottom-y-lt",
            "301",
            "--json",
        ],
    );
    assert_eq!(lt["status"], "success", "{lt:#}");
    assert_eq!(lt["result"]["relation"], "lt");

    let output = ui_run(
        home,
        &project,
        &adb,
        &[
            "ui",
            "assert-bounds",
            "--text",
            "Save",
            "--bottom-y",
            "299",
            "--json",
        ],
    );
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    let failure: Value = serde_json::from_slice(&output.stdout).expect("failure JSON");
    assert_eq!(failure["error"]["code"], "E_UI_ASSERTION_FAILED");
    let context = &failure["error"]["context"];
    assert_eq!(context["expected"], 299);
    assert_eq!(context["actual"], 300);
    assert_eq!(context["tolerance"], 0);
    assert_eq!(context["passed"], false);
    assert_eq!(context["relation"], "eq");
    assert_eq!(context["comparison"]["actual"], 300);

    let ambiguous = ui_run(
        home,
        &project,
        &adb,
        &[
            "ui",
            "assert-bounds",
            "--text",
            "Save",
            "--bottom-y",
            "300",
            "--bottom-y-gt",
            "299",
            "--json",
        ],
    );
    assert_eq!(ambiguous.status.code(), Some(2), "{ambiguous:?}");
}

#[test]
fn ui_wait_for_bounds_stable_with_fake_adb() {
    let (_temp, project, adb) = ui_fixture();
    let home = adb.parent().expect("adb home");

    let wait = ui_json(
        home,
        &project,
        &adb,
        &[
            "ui",
            "wait",
            "--text",
            "Save",
            "--for-bounds-stable",
            "--stable-for",
            "1ms",
            "--timeout",
            "5s",
            "--json",
        ],
    );
    assert_eq!(wait["status"], "success", "{wait:#}");
    assert_eq!(wait["command"], "ui.wait");
    assert_eq!(wait["result"]["stable"], true);
    assert_eq!(wait["result"]["bounds"]["bottom"], 300);
    assert!(wait["result"]["attempts"].as_u64().unwrap_or(0) >= 2);

    let assert = ui_json(
        home,
        &project,
        &adb,
        &[
            "ui",
            "assert",
            "--text",
            "Save",
            "--for-bounds-stable",
            "--stable-for",
            "1ms",
            "--timeout",
            "5s",
            "--json",
        ],
    );
    assert_eq!(assert["status"], "success", "{assert:#}");
    assert_eq!(assert["command"], "ui.assert");
    assert_eq!(assert["result"]["stable"], true);

    for (args, code) in [
        (
            vec![
                "ui",
                "wait",
                "--text",
                "Save",
                "--for-bounds-stable",
                "--for-stable",
                "--json",
            ],
            2,
        ),
        (
            vec![
                "ui",
                "wait",
                "--text",
                "Save",
                "--for-bounds-stable",
                "--not-exists",
                "--json",
            ],
            2,
        ),
    ] {
        let output = ui_run(home, &project, &adb, &args);
        assert_eq!(output.status.code(), Some(code), "{args:?}: {output:?}");
        let error: Value = serde_json::from_slice(&output.stdout).expect("error JSON");
        assert_eq!(error["error"]["code"], "E_INVALID_ARGUMENT");
    }

    let missing_selector = ui_run(
        home,
        &project,
        &adb,
        &["ui", "wait", "--for-bounds-stable", "--json"],
    );
    assert_eq!(
        missing_selector.status.code(),
        Some(2),
        "{missing_selector:?}"
    );
    let error: Value = serde_json::from_slice(&missing_selector.stdout).expect("error JSON");
    assert_eq!(error["error"]["code"], "E_INVALID_ARGUMENT");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("selector")
    );
}

// ---------------------------------------------------------------------------
// CLI integration for `androidforge build`. Every build is a fake script inside
// an isolated temp project: no Gradle, SDK installation or device is involved.
// ---------------------------------------------------------------------------
mod build_cli {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command as StdCommand;
    use std::sync::OnceLock;

    use assert_cmd::Command;
    use serde_json::Value;

    fn toml_string(value: &str) -> String {
        value.replace('\\', "\\\\").replace('"', "\\\"")
    }

    fn ps_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    fn fake_aapt2() -> PathBuf {
        static PATH: OnceLock<PathBuf> = OnceLock::new();
        PATH.get_or_init(|| {
            let dir = std::env::temp_dir().join(format!(
                "androidforge-build-cli-aapt2-{}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).expect("fake aapt2 dir");
            let source = dir.join("fake_aapt2.rs");
            fs::write(
                &source,
                r#"fn main() {
    println!("package: name='com.example.build' versionCode='7' versionName='1.2.3'");
    println!("sdkVersion:'24' targetSdkVersion:'35'");
}
"#,
            )
            .expect("fake aapt2 source");
            let executable = dir.join(if cfg!(windows) {
                "fake_aapt2.exe"
            } else {
                "fake_aapt2"
            });
            let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
            let status = StdCommand::new(rustc)
                .arg(&source)
                .arg("-o")
                .arg(&executable)
                .status()
                .expect("compile fake aapt2");
            assert!(status.success(), "fake aapt2 compile failed");
            executable
        })
        .clone()
    }

    /// Fake ADB that always reports zero devices. Install/launch therefore
    /// reach the existing device gate without touching any hardware.
    #[cfg(windows)]
    fn fake_empty_adb(dir: &Path) -> PathBuf {
        let path = dir.join("fake-adb.cmd");
        fs::write(
            &path,
            "@echo off\r\necho List of devices attached\r\necho.\r\nexit /b 0\r\n",
        )
        .expect("fake adb");
        path
    }

    #[cfg(not(windows))]
    fn fake_empty_adb(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-adb");
        fs::write(
            &path,
            "#!/bin/sh\nprintf 'List of devices attached\\n\\n'\nexit 0\n",
        )
        .expect("fake adb");
        let mut permissions = fs::metadata(&path).expect("adb metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("adb permissions");
        path
    }

    fn script_argv(dir: &Path, name: &str, windows: &str, unix: &str) -> Vec<String> {
        if cfg!(windows) {
            let path = dir.join(format!("{name}.ps1"));
            fs::write(&path, windows).expect("build script");
            vec![
                "powershell.exe".to_string(),
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                path.to_string_lossy().into_owned(),
            ]
        } else {
            let path = dir.join(format!("{name}.sh"));
            fs::write(&path, unix).expect("build script");
            vec!["sh".to_string(), path.to_string_lossy().into_owned()]
        }
    }

    /// Isolated project with one fake build command, a compiled fake aapt2 and
    /// a fake ADB with no devices.
    fn fixture(
        name: &str,
        windows: &str,
        unix: &str,
    ) -> (tempfile::TempDir, PathBuf, PathBuf, Vec<String>) {
        let temp = tempfile::tempdir().expect("build project");
        let root = temp.path().to_path_buf();
        let home = root.join("home");
        fs::create_dir_all(&home).expect("home");
        let scripts = root.join("scripts");
        fs::create_dir_all(&scripts).expect("scripts");
        let argv = script_argv(&scripts, name, windows, unix);
        let values = argv
            .iter()
            .map(|value| format!("\"{}\"", toml_string(value)))
            .collect::<Vec<_>>()
            .join(", ");
        let adb = fake_empty_adb(&home);
        let toml = format!(
            "adapter_version = 1\n[sdk]\naapt2 = \"{}\"\nadb = \"{}\"\n[project]\nname = \"BuildCli\"\npackage = \"com.example.build\"\n[build]\ndefault_variant = \"debug\"\ntimeout_secs = 30\n[build.commands]\ndebug = [{values}]\n[build.variants]\ndebug = [\"app/build/outputs/apk/debug/*.apk\"]\n",
            toml_string(&fake_aapt2().to_string_lossy()),
            toml_string(&adb.to_string_lossy())
        );
        let config = root.join(".androidforge");
        fs::create_dir_all(&config).expect("config dir");
        fs::write(config.join("project.toml"), toml).expect("project.toml");
        (temp, root, home, argv)
    }

    /// Replace a fixture's placeholder script once its project root is known.
    fn set_script(root: &Path, name: &str, windows: &str, unix: &str) {
        let dir = root.join("scripts");
        if cfg!(windows) {
            fs::write(dir.join(format!("{name}.ps1")), windows).expect("rewrite build script");
        } else {
            fs::write(dir.join(format!("{name}.sh")), unix).expect("rewrite build script");
        }
    }

    fn run(home: &Path, root: &Path, args: &[&str]) -> std::process::Output {
        Command::cargo_bin("androidforge")
            .expect("binary")
            .current_dir(root)
            .env("ANDROIDFORGE_HOME", home)
            .env_remove("ANDROIDFORGE_SESSION")
            .env_remove("ANDROIDFORGE_SERIAL")
            .args(["--project", root.to_str().expect("utf8 root")])
            .args(args)
            .output()
            .expect("build command")
    }

    fn json(output: &std::process::Output) -> Value {
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "parse build JSON: {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    fn success_script(root: &Path) -> (String, String) {
        let apk = root.join("app/build/outputs/apk/debug/app-debug.apk");
        let parent = apk.parent().expect("apk parent");
        let marker = root.join("ran.marker");
        (
            format!(
                "$ErrorActionPreference = 'Stop'\nNew-Item -ItemType Directory -Force -Path {} | Out-Null\nSet-Content -LiteralPath {} -Value 'fake apk bytes' -Encoding ascii\nSet-Content -LiteralPath {} -Value 'ran' -Encoding ascii\nWrite-Output 'BUILD SUCCESSFUL'\n",
                ps_quote(&parent.to_string_lossy()),
                ps_quote(&apk.to_string_lossy()),
                ps_quote(&marker.to_string_lossy())
            ),
            format!(
                "mkdir -p '{}' && printf 'fake apk bytes' > '{}' && echo ran > '{}' && echo 'BUILD SUCCESSFUL'\n",
                parent.display(),
                apk.display(),
                marker.display()
            ),
        )
    }

    #[test]
    fn build_only_runs_fake_script_and_returns_additive_result() {
        let temp = tempfile::tempdir().expect("build project");
        let root = temp.path().to_path_buf();
        let home = root.join("home");
        fs::create_dir_all(&home).expect("home");
        let scripts = root.join("scripts");
        fs::create_dir_all(&scripts).expect("scripts");
        let (windows, unix) = success_script(&root);
        let argv = script_argv(&scripts, "ok", &windows, &unix);
        let values = argv
            .iter()
            .map(|value| format!("\"{}\"", toml_string(value)))
            .collect::<Vec<_>>()
            .join(", ");
        let toml = format!(
            "adapter_version = 1\n[sdk]\naapt2 = \"{}\"\n[project]\nname = \"BuildCli\"\npackage = \"com.example.build\"\n[build]\ndefault_variant = \"debug\"\ntimeout_secs = 30\n[build.commands]\ndebug = [{values}]\n[build.variants]\ndebug = [\"app/build/outputs/apk/debug/*.apk\"]\n",
            toml_string(&fake_aapt2().to_string_lossy())
        );
        let config = root.join(".androidforge");
        fs::create_dir_all(&config).expect("config dir");
        fs::write(config.join("project.toml"), toml).expect("project.toml");

        let output = run(&home, &root, &["build", "--variant", "debug", "--json"]);
        assert!(output.status.success(), "{output:?}");
        let value = json(&output);
        assert_eq!(value["status"], "success", "{value:#}");
        assert_eq!(value["command"], "build");
        let result = &value["result"];
        assert_eq!(result["build"]["status"], "success");
        assert_eq!(result["build"]["variant"], "debug");
        assert_eq!(result["apk"]["package"], "com.example.build");
        assert_eq!(result["apk"]["version_code"], 7);
        assert_eq!(
            result["build"]["argv"],
            serde_json::to_value(&argv).expect("argv JSON")
        );
        assert!(result.get("install").is_none());
        assert!(result.get("launch").is_none());
        assert!(root.join("ran.marker").is_file(), "fake build did not run");
        let log_path = result["build"]["log"]["path"]
            .as_str()
            .expect("log path")
            .to_string();
        assert!(Path::new(&log_path).is_file(), "missing build log artifact");
    }

    #[test]
    fn failed_build_returns_structured_error() {
        let windows = "Write-Output '> Task :app:compileDebugKotlin FAILED'\nWrite-Output 'e: Unresolved reference: missingSymbol'\nWrite-Output 'FAILURE: Build failed with an exception.'\nexit 1\n";
        let unix = "echo '> Task :app:compileDebugKotlin FAILED'\necho 'e: Unresolved reference: missingSymbol'\necho 'FAILURE: Build failed with an exception.'\nexit 1\n";
        let (_temp, root, home, _argv) = fixture("failure", windows, unix);
        let output = run(&home, &root, &["build", "--json"]);
        assert_eq!(output.status.code(), Some(6), "{output:?}");
        let value = json(&output);
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "E_BUILD_FAILED");
        let context = &value["error"]["context"];
        assert_eq!(context["classification"], "gradle_task_failed");
        assert_eq!(context["failed_task"], ":app:compileDebugKotlin");
        assert!(
            context["first_error"]
                .as_str()
                .unwrap_or_default()
                .contains("Unresolved reference")
        );
        assert_eq!(
            context["build_outcome"]["failure"]["kind"],
            "gradle_task_failed"
        );
        assert!(Path::new(context["log_path"].as_str().expect("log path")).is_file());
    }

    #[test]
    fn build_timeout_uses_e_build_timeout() {
        let (_temp, root, home, _argv) =
            fixture("timeout", "Start-Sleep -Seconds 20\n", "sleep 20\n");
        let output = run(&home, &root, &["build", "--timeout", "1s", "--json"]);
        assert_eq!(output.status.code(), Some(5), "{output:?}");
        let value = json(&output);
        assert_eq!(value["error"]["code"], "E_BUILD_TIMEOUT");
        assert_eq!(
            value["error"]["context"]["build_outcome"]["timed_out"],
            true
        );
    }

    #[test]
    fn missing_artifact_uses_e_build_artifact_not_found() {
        let (_temp, root, home, _argv) = fixture(
            "no_artifact",
            "Write-Output 'BUILD SUCCESSFUL'\n",
            "echo 'BUILD SUCCESSFUL'\n",
        );
        let output = run(&home, &root, &["build", "--json"]);
        assert_eq!(output.status.code(), Some(6), "{output:?}");
        let value = json(&output);
        assert_eq!(value["error"]["code"], "E_BUILD_ARTIFACT_NOT_FOUND");
        assert_eq!(
            value["error"]["context"]["build_outcome"]["failure"]["kind"],
            "artifact_not_found"
        );
    }

    #[test]
    fn ambiguous_artifact_uses_e_build_artifact_ambiguous() {
        let (_temp, root, home, _argv) = fixture("ambiguous", "", "");
        let dir = root.join("app/build/outputs/apk/debug");
        fs::create_dir_all(&dir).expect("output dir");
        let first = dir.join("first.apk");
        let second = dir.join("second.apk");
        fs::write(&first, "first").expect("first apk");
        fs::write(&second, "second").expect("second apk");
        let windows = format!(
            "$fixed = [datetime]'2023-11-14T22:13:20Z'\n(Get-Item -LiteralPath {}).LastWriteTimeUtc = $fixed\n(Get-Item -LiteralPath {}).LastWriteTimeUtc = $fixed\nWrite-Output 'BUILD SUCCESSFUL'\n",
            ps_quote(&first.to_string_lossy()),
            ps_quote(&second.to_string_lossy())
        );
        let unix = format!(
            "touch -d '2023-11-14T22:13:20Z' '{}' '{}'\necho 'BUILD SUCCESSFUL'\n",
            first.display(),
            second.display()
        );
        set_script(&root, "ambiguous", &windows, &unix);
        let output = run(&home, &root, &["build", "--json"]);
        assert_eq!(output.status.code(), Some(6), "{output:?}");
        let value = json(&output);
        assert_eq!(value["error"]["code"], "E_BUILD_ARTIFACT_AMBIGUOUS");
    }

    #[test]
    fn dry_run_executes_nothing_and_resolves_no_device() {
        let (_temp, root, home, _argv) = fixture("dry", "", "");
        let (windows, unix) = success_script(&root);
        set_script(&root, "dry", &windows, &unix);
        let output = run(
            &home,
            &root,
            &[
                "--dry-run",
                "build",
                "--install",
                "--launch",
                "--cold",
                "--activity",
                ".MainActivity",
                "--json",
            ],
        );
        assert!(output.status.success(), "{output:?}");
        let value = json(&output);
        assert_eq!(value["status"], "success", "{value:#}");
        let result = &value["result"];
        assert_eq!(result["build"]["status"], "dry_run");
        assert_eq!(result["install"]["dry_run"], true);
        assert_eq!(result["launch"]["dry_run"], true);
        assert_eq!(result["launch"]["cold"], true);
        assert!(result.get("apk").is_none());
        assert!(
            !root.join("ran.marker").exists(),
            "dry-run must not execute"
        );
        assert!(!root.join(".artifacts/build").exists());
        assert!(value.get("session_id").is_none());
        assert!(value.get("device").is_none());
    }

    #[test]
    fn install_and_launch_options_reach_the_device_gate_without_a_session() {
        let (_temp, root, home, _argv) = fixture("install", "", "");
        let (windows, unix) = success_script(&root);
        set_script(&root, "install", &windows, &unix);
        for extra in [
            vec!["--install"],
            vec!["--launch", "--activity", ".MainActivity"],
            vec!["--cold"],
        ] {
            let mut args = vec!["build", "--json"];
            args.extend(extra.iter().copied());
            let output = run(&home, &root, &args);
            assert!(!output.status.success(), "{args:?}: {output:?}");
            let value = json(&output);
            assert!(
                matches!(
                    value["error"]["code"].as_str(),
                    Some("E_DEVICE_NOT_FOUND" | "E_TOOL_NOT_FOUND" | "E_ADB_FAILED")
                ),
                "{args:?}: {value:#}"
            );
        }
        assert!(
            root.join("ran.marker").is_file(),
            "build must run before the device gate"
        );
    }

    #[test]
    fn fresh_with_only_if_changed_is_rejected_before_execution() {
        let (_temp, root, home, _argv) = fixture("fresh", "", "");
        let (windows, unix) = success_script(&root);
        set_script(&root, "fresh", &windows, &unix);
        let output = run(
            &home,
            &root,
            &["build", "--fresh", "--only-if-changed", "--json"],
        );
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        let value = json(&output);
        assert_eq!(value["error"]["code"], "E_INVALID_ARGUMENT");
        assert!(!root.join("ran.marker").exists());
    }
}

// ---------------------------------------------------------------------------
// CLI integration batch E1: AVD/capacity/ports, device scenario, device tags
// and failed-session cleanup. Every operation below is host-only or dry-run;
// no emulator is started and no physical device is contacted.
// ---------------------------------------------------------------------------

fn e1_fake_adb(dir: &std::path::Path) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let path = dir.join("e1-fake-adb.cmd");
        std::fs::write(
            &path,
            "@echo off\r\n\
             if \"%~1\"==\"devices\" (\r\n\
             echo List of devices attached\r\n\
             echo emulator-5554 device product:sdk_gphone model:Android transport_id:1\r\n\
             echo emulator-5556 device product:sdk_gphone model:Android transport_id:2\r\n\
             echo.\r\n\
             )\r\n\
             exit /b 0\r\n",
        )
        .expect("fake adb");
        path
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("e1-fake-adb");
        std::fs::write(
            &path,
            "#!/bin/sh\n\
             if [ \"$1\" = \"devices\" ]; then\n\
             printf 'List of devices attached\\nemulator-5554 device product:sdk_gphone model:Android transport_id:1\\nemulator-5556 device product:sdk_gphone model:Android transport_id:2\\n\\n'\n\
             fi\n\
             exit 0\n",
        )
        .expect("fake adb");
        let mut permissions = std::fs::metadata(&path)
            .expect("fake adb metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("fake adb permissions");
        path
    }
}

fn e1_session(
    home: &std::path::Path,
    id: &str,
    owner: &str,
    serial: &str,
    port: Option<u16>,
    avd: Option<&str>,
    state: androidforge_core::session::SessionState,
) -> androidforge_core::session::Session {
    let mut session = androidforge_core::session::Session::new(
        owner,
        androidforge_core::session::SessionKind::Emulator,
        Some(serial.to_string()),
        avd.map(str::to_string),
        port,
        true,
    );
    session.id = id.to_string();
    session.state = state;
    let dir = home.join("state").join("sessions").join(id);
    std::fs::create_dir_all(&dir).expect("session dir");
    std::fs::write(
        dir.join("session.json"),
        serde_json::to_vec_pretty(&session).expect("session JSON"),
    )
    .expect("write session");
    session
}

#[test]
fn e1_new_commands_expose_help_catalog_and_mcp_entries() {
    for (args, flags) in [
        (vec!["doctor", "--help"], vec!["--capacity"]),
        (vec!["avd", "--help"], vec!["images", "create"]),
        (vec!["avd", "create", "--help"], vec!["--image", "--device"]),
        (vec!["ports", "list", "--help"], vec!["--json"]),
        (
            vec!["devices", "scenario", "--help"],
            vec!["--night", "--locale", "--font", "--network"],
        ),
        (
            vec!["devices", "tag", "--help"],
            vec!["add", "remove", "list", "clear"],
        ),
        (
            vec!["devices", "tag", "add", "--help"],
            vec!["--serial", "--tag"],
        ),
        (
            vec!["session", "gc", "--help"],
            vec!["--forget-terminal", "--owner", "--force"],
        ),
        (vec!["agent", "matrix", "--help"], vec!["--device"]),
    ] {
        let output = binary().args(&args).output().expect("command help");
        assert!(output.status.success(), "help {args:?}: {output:?}");
        let help = String::from_utf8(output.stdout).expect("utf8 help");
        for flag in flags {
            assert!(help.contains(flag), "{flag} missing from {args:?}:\n{help}");
        }
    }

    let catalog = binary()
        .args(["tools", "catalog", "--json"])
        .output()
        .expect("tools catalog");
    assert!(catalog.status.success(), "{catalog:?}");
    let catalog: Value = serde_json::from_slice(&catalog.stdout).expect("catalog JSON");
    let commands = catalog["result"]["commands"].as_array().expect("commands");
    for (path, safety, tool_name) in [
        ("avd images", "read-only", "androidforge_avd_images"),
        ("avd create", "destructive", "androidforge_avd_create"),
        ("ports list", "read-only", "androidforge_ports_list"),
        (
            "devices scenario",
            "device-mutation",
            "androidforge_devices_scenario",
        ),
        (
            "devices tag add",
            "destructive",
            "androidforge_devices_tag_add",
        ),
        (
            "devices tag list",
            "read-only",
            "androidforge_devices_tag_list",
        ),
    ] {
        let entry = commands
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("missing catalog entry {path}"));
        assert_eq!(entry["safety"], safety, "{path}");
        assert_eq!(entry["tool_name"], tool_name, "{path}");
        assert!(
            entry["examples"]
                .as_array()
                .is_some_and(|items| !items.is_empty()),
            "{path} has no example"
        );
    }

    let tools = binary()
        .args(["tools", "schema", "--format", "mcp", "--json"])
        .output()
        .expect("tools schema");
    assert!(tools.status.success(), "{tools:?}");
    let tools: Value = serde_json::from_slice(&tools.stdout).expect("tools JSON");
    let names = tools["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();
    for name in [
        "androidforge_doctor_capacity",
        "androidforge_avd_images",
        "androidforge_avd_create",
        "androidforge_ports_list",
        "androidforge_devices_scenario",
        "androidforge_devices_tag_add",
    ] {
        assert!(names.contains(&name), "missing MCP tool {name}");
    }
}

#[test]
fn e1_avd_create_requires_yes_and_dry_run_executes_nothing() {
    let temp = tempfile::tempdir().expect("temp home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let image = "system-images;android-35;google_apis;x86_64";

    let preview = binary()
        .env("ANDROIDFORGE_HOME", &home)
        .args([
            "avd",
            "create",
            "Test_AVD",
            "--image",
            image,
            "--device",
            "pixel_7",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("avd create dry run");
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let json: Value = serde_json::from_slice(&preview.stdout).expect("JSON");
    assert_eq!(json["status"], "success");
    assert_eq!(json["command"], "avd.create");
    assert_eq!(json["result"]["dry_run"], true);
    assert_eq!(json["result"]["executed"], false);
    assert_eq!(json["result"]["name"], "Test_AVD");
    let argv = json["result"]["argv"].as_array().expect("argv");
    assert_eq!(argv[0], "create");
    assert_eq!(argv[1], "avd");
    assert!(argv.iter().any(|value| value == "Test_AVD"));
    assert!(!home.join("Test_AVD.ini").exists());

    let unconfirmed = binary()
        .env("ANDROIDFORGE_HOME", &home)
        .args([
            "avd", "create", "Test_AVD", "--image", image, "--device", "pixel_7", "--json",
        ])
        .output()
        .expect("unconfirmed avd create");
    let error: Value = serde_json::from_slice(&unconfirmed.stdout).expect("JSON");
    assert_eq!(error["status"], "error");
    assert_eq!(error["error"]["code"], "E_INVALID_ARGUMENT");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--yes")
    );
    assert!(!home.join("Test_AVD.ini").exists());
}

#[test]
fn e1_doctor_capacity_never_fabricates_unknown_values() {
    let temp = tempfile::tempdir().expect("temp home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let output = binary()
        .env("ANDROIDFORGE_HOME", &home)
        .args(["doctor", "--capacity", "--json"])
        .output()
        .expect("doctor capacity");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(json["status"], "success");
    assert_eq!(json["command"], "doctor.capacity");
    assert_eq!(json["result"]["read_only"], true);
    assert_eq!(json["result"]["schema_version"], 1);
    for field in [
        "physical_total",
        "physical_available",
        "commit_limit",
        "commit_available",
        "disk_free",
    ] {
        let reading = &json["result"]["host"][field];
        assert!(
            reading["source"]
                .as_str()
                .is_some_and(|source| !source.is_empty()),
            "{field} has no source: {reading}"
        );
        if reading["value_bytes"].is_null() {
            assert!(
                reading["unavailable_reason"]
                    .as_str()
                    .is_some_and(|reason| !reason.is_empty()),
                "{field} is unknown without a reason: {reading}"
            );
        } else {
            assert!(
                reading["value_bytes"]
                    .as_u64()
                    .is_some_and(|value| value > 0),
                "{field} must not be fabricated as zero: {reading}"
            );
        }
    }
    let estimates = json["result"]["estimates"].as_array().expect("estimates");
    for estimate in estimates {
        assert!(estimate["estimated_disk_bytes"].is_null(), "{estimate}");
    }
    assert!(
        json["result"]["notes"]
            .as_array()
            .is_some_and(|notes| !notes.is_empty())
    );
}

#[test]
fn e1_ports_list_reuses_sessions_for_ownership() {
    use androidforge_core::session::SessionState;

    let temp = tempfile::tempdir().expect("temp home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let session = e1_session(
        &home,
        "ses-ports-1",
        "alice",
        "emulator-5554",
        Some(5554),
        Some("Pixel_Test"),
        SessionState::Ready,
    );
    // A tombstone must never become a port owner.
    e1_session(
        &home,
        "ses-ports-dead",
        "alice",
        "emulator-5556",
        Some(5556),
        None,
        SessionState::Failed,
    );
    let output = binary()
        .env("ANDROIDFORGE_HOME", &home)
        .args(["ports", "list", "--json"])
        .output()
        .expect("ports list");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(json["command"], "ports.list");
    let ports = json["result"]["ports"].as_array().expect("ports");
    assert_eq!(ports.len(), 65);
    let entry = ports
        .iter()
        .find(|entry| entry["console_port"] == 5554)
        .expect("console port 5554");
    assert_eq!(entry["serial"], "emulator-5554");
    assert_eq!(entry["adb_port"], 5555);
    assert_eq!(entry["grpc_port"], 8554);
    assert_eq!(entry["owner"], "alice");
    assert_eq!(entry["session_id"], session.id);
    assert_eq!(entry["avd"], "Pixel_Test");
    assert!(
        ["free", "allocated", "occupied"].contains(&entry["state"].as_str().unwrap_or_default())
    );
    let dead = ports
        .iter()
        .find(|entry| entry["console_port"] == 5556)
        .expect("console port 5556");
    assert!(dead["session_id"].is_null());
    let summary = &json["result"]["summary"];
    assert_eq!(
        summary["free"].as_u64().unwrap_or(0)
            + summary["allocated"].as_u64().unwrap_or(0)
            + summary["occupied"].as_u64().unwrap_or(0),
        ports.len() as u64
    );
}

#[test]
fn e1_devices_scenario_dry_run_plans_without_a_session() {
    let temp = tempfile::tempdir().expect("temp home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    for (args, scenario, risk) in [
        (vec!["--night", "yes"], "night:yes", "data"),
        (vec!["--locale", "fr-FR"], "locale:fr-FR", "data"),
        (vec!["--network", "offline"], "network:offline", "network"),
    ] {
        let mut argv = vec!["devices", "scenario"];
        argv.extend(args);
        argv.extend(["--dry-run", "--json"]);
        let output = binary()
            .env("ANDROIDFORGE_HOME", &home)
            .env_remove("ANDROIDFORGE_SESSION")
            .args(&argv)
            .output()
            .expect("scenario dry run");
        assert!(
            output.status.success(),
            "args {argv:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: Value = serde_json::from_slice(&output.stdout).expect("JSON");
        assert_eq!(json["status"], "success", "{json:#}");
        assert_eq!(json["command"], "devices.scenario");
        assert_eq!(json["result"]["applied"], false);
        assert_eq!(json["result"]["plan"]["scenario"], scenario);
        assert_eq!(json["result"]["plan"]["risk"], risk);
        assert_eq!(json["result"]["plan"]["dry_run"], true);
        assert_eq!(json["result"]["plan"]["will_mutate"], false);
        assert!(
            !json["result"]["plan"]["restore_keys"]
                .as_array()
                .expect("restore keys")
                .is_empty()
        );
    }
    let none = binary()
        .env("ANDROIDFORGE_HOME", &home)
        .args(["devices", "scenario", "--dry-run", "--json"])
        .output()
        .expect("scenario without a flag");
    assert_eq!(none.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&none.stdout).expect("JSON");
    assert_eq!(error["error"]["code"], "E_INVALID_ARGUMENT");
}

#[test]
fn e1_devices_tag_add_remove_list_clear_round_trip() {
    let temp = tempfile::tempdir().expect("temp home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let run = |args: &[&str]| {
        let output = binary()
            .env("ANDROIDFORGE_HOME", &home)
            .args(args)
            .output()
            .expect("device tag command");
        assert!(
            output.status.success(),
            "args {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).expect("JSON")
    };
    let added = run(&[
        "devices",
        "tag",
        "add",
        "--serial",
        "emulator-5554",
        "--tag",
        "nightly",
        "--json",
    ]);
    assert_eq!(added["command"], "devices.tag.add");
    assert_eq!(added["result"]["changed"], true);
    assert_eq!(added["result"]["tags"], serde_json::json!(["nightly"]));
    run(&[
        "devices",
        "tag",
        "add",
        "--serial",
        "emulator-5556",
        "--tag",
        "nightly",
        "--json",
    ]);
    let listed = run(&["devices", "tag", "list", "--json"]);
    assert_eq!(listed["result"]["entries"][0]["serial"], "emulator-5554");
    assert_eq!(listed["result"]["entries"][1]["serial"], "emulator-5556");
    let removed = run(&[
        "devices",
        "tag",
        "remove",
        "--serial",
        "emulator-5554",
        "--tag",
        "nightly",
        "--json",
    ]);
    assert_eq!(removed["result"]["changed"], true);
    let cleared = run(&[
        "devices",
        "tag",
        "clear",
        "--serial",
        "emulator-5556",
        "--json",
    ]);
    assert_eq!(cleared["result"]["changed"], true);
    let empty = run(&["devices", "tag", "list", "--json"]);
    assert_eq!(empty["result"]["entries"], serde_json::json!([]));

    let preview = run(&[
        "devices",
        "tag",
        "add",
        "--serial",
        "emulator-5554",
        "--tag",
        "pixel",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(preview["result"]["dry_run"], true);
    let after = run(&["devices", "tag", "list", "--json"]);
    assert_eq!(after["result"]["entries"], serde_json::json!([]));

    let invalid = binary()
        .env("ANDROIDFORGE_HOME", &home)
        .args([
            "devices",
            "tag",
            "add",
            "--serial",
            "emulator-5554",
            "--tag",
            "BAD TAG",
            "--json",
        ])
        .output()
        .expect("invalid tag");
    assert_eq!(invalid.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&invalid.stdout).expect("JSON");
    assert_eq!(error["error"]["code"], "E_INVALID_ARGUMENT");
}

#[test]
fn e1_matrix_tag_resolution_dedupes_explicit_serials() {
    let temp = tempfile::tempdir().expect("temp home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let adb = e1_fake_adb(&home);
    let project = temp.path().join("MatrixProject");
    project_with_defaults(&project);
    let run = |args: &[&str]| {
        let output = binary()
            .current_dir(&project)
            .env("ANDROIDFORGE_HOME", &home)
            .env("ANDROIDFORGE_ADB", &adb)
            .env_remove("ANDROIDFORGE_SESSION")
            .args(args)
            .output()
            .expect("matrix command");
        assert!(
            output.status.success(),
            "args {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).expect("JSON")
    };
    run(&[
        "devices",
        "tag",
        "add",
        "--serial",
        "emulator-5554",
        "--tag",
        "nightly",
        "--json",
    ]);
    run(&[
        "devices",
        "tag",
        "add",
        "--serial",
        "emulator-5556",
        "--tag",
        "nightly",
        "--json",
    ]);

    let dry = run(&[
        "agent",
        "matrix",
        "--device",
        "tag:nightly",
        "--serial",
        "emulator-5554",
        "--flow",
        "conformance",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry["command"], "agent.matrix");
    let targets = dry["result"]["targets"].as_array().expect("targets");
    assert_eq!(targets.len(), 2, "{dry:#}");
    let labels = targets
        .iter()
        .map(|target| target["label"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(labels.contains(&"emulator-5554"));
    assert!(labels.contains(&"emulator-5556"));
    let resolution = &dry["result"]["device_resolution"];
    assert_eq!(
        resolution["tags"]["nightly"],
        serde_json::json!(["emulator-5554", "emulator-5556"])
    );

    let unknown = binary()
        .current_dir(&project)
        .env("ANDROIDFORGE_HOME", &home)
        .env("ANDROIDFORGE_ADB", &adb)
        .args([
            "agent",
            "matrix",
            "--device",
            "tag:missing",
            "--flow",
            "conformance",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("unknown tag");
    assert_eq!(unknown.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&unknown.stdout).expect("JSON");
    assert_eq!(error["error"]["code"], "E_INVALID_ARGUMENT");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("no device is tagged")
    );
}

// ---------------------------------------------------------------------------
// CLI integration batch E2: fixture declarative-state aliases and named
// network mock scenarios. All tests are host-only or dry-run; no device,
// emulator, ADB server or host proxy setting is touched.
// ---------------------------------------------------------------------------

fn e2_project() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().expect("temp project");
    let adapter = temp.path().join(".androidforge");
    std::fs::create_dir_all(adapter.join("fixtures")).expect("fixtures directory");
    std::fs::write(
        adapter.join("project.toml"),
        "adapter_version = 1\n[project]\nname = \"E2Test\"\npackage = \"com.example.e2\"\n",
    )
    .expect("adapter");
    std::fs::write(
        adapter.join("fixtures").join("state-demo.json"),
        r#"{
            "version": 1,
            "name": "state-demo",
            "state": {"prefs": {"values": {"onboarding_done": "true"}}},
            "steps": [
                {"name": "apply", "action": {"type": "apply_state"}},
                {"name": "reset", "phase": "cleanup", "action": {"type": "reset_state"}}
            ]
        }"#,
    )
    .expect("fixture");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    (temp, home)
}

fn e2_json(project: &std::path::Path, home: &std::path::Path, args: &[&str]) -> Value {
    let output = binary()
        .current_dir(project)
        .env("ANDROIDFORGE_HOME", home)
        .env_remove("ANDROIDFORGE_SESSION")
        .args(args)
        .output()
        .expect("run E2 command");
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "parse {args:?}: {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn e2_fixture_state_aliases_are_explicit_and_dry_run_is_host_only() {
    let (temp, home) = e2_project();
    let project = temp.path();
    for (action, args) in [
        (
            "apply_state",
            vec!["fixture", "apply", "state-demo", "--dry-run", "--json"],
        ),
        (
            "reset_state",
            vec!["fixture", "reset", "state-demo", "--dry-run", "--json"],
        ),
    ] {
        let json = e2_json(project, &home, &args);
        assert_eq!(json["status"], "success", "{json:#}");
        assert_eq!(
            json["command"],
            if action == "apply_state" {
                "fixture.apply"
            } else {
                "fixture.reset"
            }
        );
        assert_eq!(json["result"]["dry_run"], true);
        assert_eq!(json["result"]["action"], action);
        assert_eq!(json["result"]["state_declared"], true);
        assert_eq!(json["result"]["package"], "com.example.e2");
        assert!(
            json["result"]["fixture"]
                .as_str()
                .is_some_and(|path| !path.is_empty())
        );
    }

    // Unknown fixtures are reported without a session or device lookup.
    let unknown = e2_json(
        project,
        &home,
        &["fixture", "apply", "missing", "--dry-run", "--json"],
    );
    assert_eq!(unknown["status"], "error");
    assert_eq!(unknown["error"]["code"], "E_FIXTURE_INVALID");
    assert!(
        unknown["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("missing")
    );
}

#[test]
fn e2_network_mock_scenario_catalog_lookup_and_host_only_guards() {
    let (temp, home) = e2_project();
    let project = temp.path();
    let catalog = e2_json(project, &home, &["network", "mock", "scenarios", "--json"]);
    assert_eq!(catalog["status"], "success", "{catalog:#}");
    assert_eq!(catalog["command"], "network.mock.scenarios");
    let scenarios = catalog["result"]["scenarios"]
        .as_array()
        .expect("scenarios");
    assert_eq!(scenarios.len(), 4);
    let names = scenarios
        .iter()
        .map(|entry| entry["name"].as_str().unwrap_or(""))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec!["newer-release", "offline", "bad-digest", "no-APK"]
    );
    let offline = scenarios
        .iter()
        .find(|entry| entry["name"] == "offline")
        .expect("offline");
    assert_eq!(offline["mode"], "offline");
    assert!(
        offline["fixture_file"]
            .as_str()
            .is_some_and(|path| path.contains("offline.json"))
    );

    let planned = e2_json(
        project,
        &home,
        &[
            "network",
            "mock",
            "serve",
            "--scenario",
            "newer-release",
            "--port",
            "18081",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(planned["status"], "success", "{planned:#}");
    assert_eq!(planned["result"]["scenario"], "newer-release");
    assert_eq!(planned["result"]["scenario_mode"], "serve");
    assert_eq!(planned["result"]["routes"], 2);
    assert_eq!(planned["result"]["max_requests"], 64);
    assert_eq!(planned["result"]["bind"], "127.0.0.1");

    let offline_plan = e2_json(
        project,
        &home,
        &[
            "network",
            "mock",
            "serve",
            "--scenario",
            "offline",
            "--port",
            "18081",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(offline_plan["status"], "success", "{offline_plan:#}");
    assert_eq!(offline_plan["result"]["scenario_mode"], "offline");
    assert_eq!(offline_plan["result"]["routes"], 0);

    let unknown = e2_json(
        project,
        &home,
        &[
            "network",
            "mock",
            "serve",
            "--scenario",
            "does-not-exist",
            "--port",
            "18081",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(unknown["status"], "error");
    assert_eq!(unknown["error"]["code"], "E_INVALID_ARGUMENT");
    assert!(
        unknown["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown network mock scenario")
    );
    assert!(
        unknown["error"]["suggestions"]
            .as_array()
            .is_some_and(|items| items
                .iter()
                .any(|item| { item.as_str().unwrap_or_default().contains("newer-release") }))
    );

    for (extra, expected) in [
        (vec!["--reverse"], "host-only"),
        (vec!["--bind", "0.0.0.0"], "loopback"),
    ] {
        let mut args = vec![
            "network",
            "mock",
            "serve",
            "--scenario",
            "offline",
            "--port",
            "18081",
        ];
        args.extend(extra);
        args.extend(["--dry-run", "--json"]);
        let refused = e2_json(project, &home, &args);
        assert_eq!(refused["status"], "error", "{refused:#}");
        assert_eq!(refused["error"]["code"], "E_INVALID_ARGUMENT");
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains(expected),
            "{refused:#}"
        );
    }
}

#[test]
fn e2_offline_scenario_serves_no_listener_and_writes_an_empty_log() {
    let (temp, home) = e2_project();
    let project = temp.path();
    let log = temp.path().join("requests.ndjson");
    let log_arg = log.to_string_lossy().into_owned();
    let json = e2_json(
        project,
        &home,
        &[
            "network",
            "mock",
            "serve",
            "--scenario",
            "offline",
            "--port",
            "18081",
            "--log",
            &log_arg,
            "--json",
        ],
    );
    assert_eq!(json["status"], "success", "{json:#}");
    assert_eq!(json["result"]["scenario"], "offline");
    assert_eq!(json["result"]["scenario_mode"], "offline");
    assert_eq!(json["result"]["stopped_by"], "offline");
    assert_eq!(json["result"]["requests"], 0);
    assert_eq!(json["result"]["routes"], 0);
    assert_eq!(json["result"]["bind"], "127.0.0.1");
    assert_eq!(json["result"]["reverse"], false);
    assert!(log.is_file());
    assert_eq!(std::fs::read(&log).expect("empty log").len(), 0);
}

#[test]
fn e2_fixture_and_scenario_commands_expose_help_catalog_and_mcp_entries() {
    for (args, flags) in [
        (vec!["fixture", "--help"], vec!["apply", "reset"]),
        (
            vec!["fixture", "apply", "--help"],
            vec!["--package", "--var"],
        ),
        (
            vec!["network", "mock", "--help"],
            vec!["scenarios", "serve"],
        ),
        (
            vec!["network", "mock", "serve", "--help"],
            vec!["--scenario", "--reverse"],
        ),
    ] {
        let output = binary().args(&args).output().expect("help output");
        assert!(output.status.success(), "{args:?}: {output:?}");
        let help = String::from_utf8(output.stdout).expect("utf8 help");
        for flag in flags {
            assert!(help.contains(flag), "{flag} missing from {args:?}:\n{help}");
        }
    }

    let (temp, home) = e2_project();
    let catalog = e2_json(temp.path(), &home, &["tools", "catalog", "--json"]);
    let commands = catalog["result"]["commands"].as_array().expect("commands");
    for (path, safety, tool_name) in [
        ("fixture apply", "destructive", "androidforge_fixture_apply"),
        ("fixture reset", "destructive", "androidforge_fixture_reset"),
        (
            "network mock scenarios",
            "read-only",
            "androidforge_network_mock_scenarios",
        ),
        (
            "network mock serve",
            "long-running",
            "androidforge_network_mock_serve",
        ),
    ] {
        let entry = commands
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("missing catalog entry {path}"));
        assert_eq!(entry["safety"], safety, "{path}");
        assert_eq!(entry["tool_name"], tool_name, "{path}");
        assert!(
            entry["examples"]
                .as_array()
                .is_some_and(|items| !items.is_empty()),
            "{path} has no example"
        );
    }

    let tools = e2_json(
        temp.path(),
        &home,
        &["tools", "schema", "--format", "mcp", "--json"],
    );
    let definitions = tools["result"]["tools"].as_array().expect("tools");
    for name in [
        "androidforge_fixture_apply",
        "androidforge_fixture_reset",
        "androidforge_network_mock_scenarios",
        "androidforge_network_mock_serve",
    ] {
        assert!(
            definitions.iter().any(|tool| tool["name"] == name),
            "missing MCP tool {name}"
        );
    }
    let serve = definitions
        .iter()
        .find(|tool| tool["name"] == "androidforge_network_mock_serve")
        .expect("mock serve tool");
    let properties = serve["inputSchema"]["properties"]
        .as_object()
        .expect("serve properties");
    assert!(properties.contains_key("scenario"));
    assert!(properties.contains_key("reverse"));
}

// ---------------------------------------------------------------------------
// CLI batch F1: install confirmation flags, a11y audit, named baselines,
// contrast. Every test below is fake-ADB or host-only; no device is touched.
// ---------------------------------------------------------------------------

fn f1_png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        let data = (0..width * height)
            .flat_map(|_| rgb.iter().copied())
            .collect::<Vec<_>>();
        writer.write_image_data(&data).expect("png data");
    }
    bytes
}

/// Fake ADB: one canned UIAutomator XML plus one canned screenshot PNG.
#[cfg(windows)]
fn f1_fake_adb(dir: &std::path::Path, xml: &str, png: &[u8]) -> std::path::PathBuf {
    std::fs::write(dir.join("ui.xml"), xml).expect("fake ui xml");
    std::fs::write(dir.join("screen.png"), png).expect("fake screenshot");
    let path = dir.join("f1-adb.cmd");
    std::fs::write(
        &path,
        "@echo off\r\n\
         setlocal enabledelayedexpansion\r\n\
         set \"all=%*\"\r\n\
         echo !all! | findstr /I \"devices\" >nul\r\n\
         if !errorlevel! equ 0 (\r\n\
         echo List of devices attached\r\n\
         echo emulator-5554 device product:sdk_gphone model:Android transport_id:1\r\n\
         echo.\r\n\
         exit /b 0\r\n\
         )\r\n\
         echo !all! | findstr /I \"screencap\" >nul\r\n\
         if !errorlevel! equ 0 (\r\n\
         type \"%~dp0screen.png\"\r\n\
         exit /b 0\r\n\
         )\r\n\
         echo !all! | findstr /I \"exec-out\" >nul\r\n\
         if !errorlevel! equ 0 (\r\n\
         type \"%~dp0ui.xml\"\r\n\
         exit /b 0\r\n\
         )\r\n\
         echo !all! | findstr /I \"uiautomator\" >nul\r\n\
         if !errorlevel! equ 0 (\r\n\
         echo OK\r\n\
         exit /b 0\r\n\
         )\r\n\
         echo !all! | findstr /I \"wm size\" >nul\r\n\
         if !errorlevel! equ 0 (\r\n\
         echo Physical display size: 1080x2400\r\n\
         exit /b 0\r\n\
         )\r\n\
         exit /b 0\r\n",
    )
    .expect("fake adb");
    path
}

#[cfg(not(windows))]
fn f1_fake_adb(dir: &std::path::Path, xml: &str, png: &[u8]) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(dir.join("ui.xml"), xml).expect("fake ui xml");
    std::fs::write(dir.join("screen.png"), png).expect("fake screenshot");
    let path = dir.join("f1-adb");
    std::fs::write(
        &path,
        "#!/bin/sh\n\
         case \"$*\" in\n\
           *devices*) printf 'List of devices attached\\nemulator-5554 device product:sdk_gphone model:Android transport_id:1\\n\\n' ;;\n\
           *screencap*) cat \"$(dirname \"$0\")/screen.png\" ;;\n\
           *exec-out*) cat \"$(dirname \"$0\")/ui.xml\" ;;\n\
           *uiautomator*) echo OK ;;\n\
           *wm\\ size*) echo 'Physical display size: 1080x2400' ;;\n\
         esac\n\
         exit 0\n",
    )
    .expect("fake adb");
    let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("permissions");
    path
}

fn f1_fixture(
    xml: &str,
    png: &[u8],
) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = tempfile::tempdir().expect("temp project");
    let project = temp.path().join("F1Project");
    let _ = project_with_defaults(&project);
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let adb = f1_fake_adb(&home, xml, png);
    (temp, project, adb)
}

fn f1_run(
    home: &std::path::Path,
    project: &std::path::Path,
    adb: &std::path::Path,
    args: &[&str],
) -> Value {
    let output = binary()
        .current_dir(project)
        .env("ANDROIDFORGE_HOME", home)
        .env("ANDROIDFORGE_ADB", adb)
        .env_remove("ANDROIDFORGE_SESSION")
        .args([
            "--project",
            project.to_str().unwrap(),
            "--serial",
            "emulator-5554",
        ])
        .args(args)
        .output()
        .expect("f1 command");
    let stdout = String::from_utf8(output.stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "parse {args:?}: {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn f1_validate(name: &str, value: &Value) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("schema")
        .join(name);
    let schema: Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read schema")).expect("parse schema");
    let validator = jsonschema::validator_for(&schema).expect("compile schema");
    if let Err(error) = validator.validate(value) {
        panic!("{name} rejected {value:#}: {error}");
    }
}

#[test]
fn f1_new_commands_are_documented_in_help_catalog_and_mcp() {
    for (args, flags) in [
        (
            vec!["app", "install", "--help"],
            vec![
                "--wait-for-confirmation",
                "--confirm-text",
                "--confirmation-timeout",
            ],
        ),
        (
            vec!["ui", "audit", "a11y", "--help"],
            vec![
                "--contrast",
                "--text-rgb",
                "--density",
                "--no-touch-targets",
            ],
        ),
        (
            vec!["capture", "baseline", "save", "--help"],
            vec!["--name", "--source", "--output"],
        ),
        (
            vec!["capture", "baseline", "compare", "--help"],
            vec!["--name", "--current", "--region", "--max-diff-ratio"],
        ),
        (
            vec!["capture", "contrast", "--help"],
            vec!["--rgb", "--at", "--region", "--threshold"],
        ),
    ] {
        let output = binary().args(&args).output().expect("help");
        assert!(output.status.success(), "help {args:?}: {output:?}");
        let help = String::from_utf8(output.stdout).expect("utf8 help");
        for flag in flags {
            assert!(help.contains(flag), "{flag} missing from {args:?}:\n{help}");
        }
    }

    let catalog: Value = serde_json::from_slice(
        &binary()
            .args(["tools", "catalog", "--json"])
            .output()
            .expect("catalog")
            .stdout,
    )
    .expect("catalog json");
    let commands = catalog["result"]["commands"].as_array().expect("commands");
    for (path, tool_name, flags) in [
        (
            "ui audit a11y",
            "androidforge_ui_audit_a11y",
            vec!["contrast", "text-rgb"],
        ),
        (
            "capture baseline save",
            "androidforge_capture_baseline_save",
            vec!["name", "source"],
        ),
        (
            "capture baseline compare",
            "androidforge_capture_baseline_compare",
            vec!["name", "current"],
        ),
        (
            "capture contrast",
            "androidforge_capture_contrast",
            vec!["rgb", "at", "region"],
        ),
    ] {
        let entry = commands
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("missing catalog entry {path}"));
        assert_eq!(entry["safety"], "read-only", "{path}");
        assert_eq!(entry["tool_name"], tool_name, "{path}");
        let arguments = entry["arguments"].as_array().expect("arguments");
        for flag in flags {
            assert!(
                arguments.iter().any(|argument| argument["long"] == flag),
                "{path} is missing {flag}"
            );
        }
    }
    let install = commands
        .iter()
        .find(|entry| entry["path"] == "app install")
        .expect("app install");
    assert_eq!(install["safety"], "device-mutation");
    // Confirmation flags are global, so they appear in `app install --help`
    // (checked above) and in the curated MCP tool rather than as local catalog
    // arguments. The curated catalog example advertises the opt-in path.
    assert!(
        install["examples"]
            .as_array()
            .expect("install examples")
            .iter()
            .any(|example| example
                .as_str()
                .is_some_and(|text| text.contains("--wait-for-confirmation"))),
        "app install catalog example is missing --wait-for-confirmation"
    );

    let tools: Value = serde_json::from_slice(
        &binary()
            .args(["tools", "schema", "--format", "mcp", "--json"])
            .output()
            .expect("tools schema")
            .stdout,
    )
    .expect("tools json");
    let definitions = tools["result"]["tools"].as_array().expect("tools array");
    for name in [
        "androidforge_ui_audit_a11y",
        "androidforge_capture_baseline_save",
        "androidforge_capture_baseline_update",
        "androidforge_capture_baseline_compare",
        "androidforge_capture_baseline_list",
        "androidforge_capture_contrast",
    ] {
        assert!(
            definitions.iter().any(|tool| tool["name"] == name),
            "missing MCP tool {name}"
        );
    }
    let install_tool = definitions
        .iter()
        .find(|tool| tool["name"] == "androidforge_app_install")
        .expect("app install MCP tool");
    for key in [
        "wait_for_confirmation",
        "confirm_text",
        "confirmation_timeout",
    ] {
        assert!(
            install_tool["inputSchema"]["properties"].get(key).is_some(),
            "app install MCP schema missing {key}"
        );
    }
}

const F1_A11Y_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<hierarchy rotation="0">
  <node index="0" text="" resource-id="com.example:id/unlabelled" class="android.widget.Button" package="com.example" content-desc="" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[10,10][40,40]" />
  <node index="1" text="Text" resource-id="com.example:id/text" class="android.widget.TextView" package="com.example" content-desc="" clickable="false" enabled="true" focused="false" scrollable="false" bounds="[0,0][2,2]" />
</hierarchy>
"#;

#[test]
fn f1_ui_audit_a11y_reports_findings_and_validates_against_schema() {
    let png = f1_png(2, 2, [255, 255, 255]);
    let (_temp, project, adb) = f1_fixture(F1_A11Y_XML, &png);
    let home = adb.parent().expect("adb parent");
    let output = f1_run(home, &project, &adb, &["ui", "audit", "a11y", "--json"]);
    assert_eq!(output["status"], "success", "{output:#}");
    assert_eq!(output["command"], "ui.audit.a11y");
    assert_eq!(output["result"]["counts"]["error"], 1); // missing content description
    assert_eq!(output["result"]["counts"]["warning"], 1); // tiny touch target
    assert!(
        output["result"]["findings"]
            .as_array()
            .expect("findings")
            .len()
            >= 2
    );
    f1_validate("a11y.schema.json", &output["result"]);
}

/// Minimal terminal/running run report used by the retention CLI tests. No
/// device is involved: the planner only reads `report.json` and directory sizes.
fn write_retention_run(root: &std::path::Path, run_id: &str, status: &str, tags: &[&str]) {
    let dir = root.join(".artifacts").join("runs").join(run_id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("payload.bin"), vec![b'x'; 16]).unwrap();
    let report = serde_json::json!({
        "schema_version": 1,
        "run_id": run_id,
        "command": "test.command",
        "project": "Example",
        "project_root": root.to_string_lossy(),
        "status": status,
        "exit_code": if status == "running" { 0 } else { 1 },
        "started_at": "2020-01-01T00:00:00Z",
        "duration_ms": 0,
        "tags": tags,
    });
    std::fs::write(
        dir.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

#[test]
fn maintenance_clean_keep_tag_dry_runs_until_yes_and_protects_runs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    project_with_defaults(root);
    write_retention_run(root, "run-keep", "passed", &["release"]);
    write_retention_run(root, "run-plain", "passed", &[]);
    write_retention_run(root, "run-running", "running", &[]);
    write_retention_run(root, "run-active", "passed", &[]);

    // Write the pointer into the same ANDROIDFORGE_HOME the child sees. Doing
    // this directly avoids mutating process-wide environment variables while
    // other integration tests run in parallel.
    let active_dir = temp.path().join("state").join("active-runs");
    std::fs::create_dir_all(&active_dir).unwrap();
    std::fs::write(
        active_dir.join("ses-test.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "run_id": "run-active",
            "session_id": "ses-test",
            "project_root": root.to_string_lossy(),
            "created_at": "2020-01-01T00:00:00Z",
        }))
        .unwrap(),
    )
    .unwrap();

    let clean = |extra: &[&str]| -> Value {
        let mut command = binary();
        command
            .current_dir(root)
            .env("ANDROIDFORGE_HOME", temp.path())
            .env_remove("ANDROIDFORGE_SESSION")
            .args(["maintenance", "clean", "--runs", "--keep-last", "0"]);
        command.args(extra);
        command.arg("--json");
        let output = command.output().expect("maintenance clean");
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "parse maintenance clean: {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    };

    // Without --yes, --dry-run or --keep-tag the destructive command is refused.
    let blocked = clean(&[]);
    assert_eq!(blocked["status"], "error", "{blocked:#}");
    assert_eq!(blocked["error"]["code"], "E_INVALID_ARGUMENT");

    // --keep-tag without --yes is an implicit dry run with a full audit plan.
    let preview = clean(&["--keep-tag", "release"]);
    assert_eq!(preview["status"], "success", "{preview:#}");
    assert_eq!(preview["command"], "maintenance.clean");
    assert_eq!(preview["result"]["dry_run"], true);
    assert_eq!(preview["result"]["runs"]["dry_run"], true);
    let plan = &preview["result"]["retention"];
    assert_eq!(plan["schema_version"], 1);
    assert_eq!(plan["keep_tags"], serde_json::json!(["release"]));
    assert_eq!(plan["executed"], false);
    let deletions = plan["deletions"].as_array().expect("deletions");
    let deleted: Vec<&str> = deletions
        .iter()
        .map(|entry| entry["run_id"].as_str().expect("run id"))
        .collect();
    assert_eq!(deleted, vec!["run-plain"], "{plan:#}");
    let kept = plan["kept_runs"].as_array().expect("kept runs");
    assert!(kept.iter().any(|entry| {
        entry["run_id"] == "run-keep"
            && entry["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("release")
    }));
    assert!(
        kept.iter()
            .any(|entry| { entry["run_id"] == "run-running" && entry["reason"] == "running run" })
    );
    assert!(kept.iter().any(|entry| {
        entry["run_id"] == "run-active" && entry["reason"] == "active run pointer"
    }));
    assert!(
        root.join(".artifacts/runs/run-plain").is_dir(),
        "dry run must not delete"
    );

    // --yes applies the same plan; protected runs survive.
    let applied = clean(&["--keep-tag", "release", "--yes"]);
    assert_eq!(applied["status"], "success", "{applied:#}");
    assert_eq!(applied["result"]["dry_run"], false);
    let plan = &applied["result"]["retention"];
    assert_eq!(plan["executed"], true);
    assert_eq!(plan["deletions"][0]["run_id"], "run-plain");
    assert!(plan["reclaimed_bytes"].as_u64().unwrap() > 0);
    assert!(!root.join(".artifacts/runs/run-plain").exists());
    assert!(root.join(".artifacts/runs/run-keep").is_dir());
    assert!(root.join(".artifacts/runs/run-running").is_dir());
    assert!(root.join(".artifacts/runs/run-active").is_dir());
}

#[test]
fn f1_ui_audit_a11y_contrast_merges_region_findings() {
    let png = f1_png(2, 2, [255, 255, 255]);
    let (_temp, project, adb) = f1_fixture(F1_A11Y_XML, &png);
    let home = adb.parent().expect("adb parent");
    let failed = f1_run(
        home,
        &project,
        &adb,
        &[
            "ui",
            "audit",
            "a11y",
            "--contrast",
            "--text-rgb",
            "#808080",
            "--json",
        ],
    );
    assert_eq!(failed["status"], "success", "{failed:#}");
    assert!(
        failed["result"]["counts"]["low_contrast"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
    assert_eq!(failed["result"]["contrast"]["nodes_sampled"], 1);
    f1_validate("a11y.schema.json", &failed["result"]);

    let passed = f1_run(
        home,
        &project,
        &adb,
        &[
            "ui",
            "audit",
            "a11y",
            "--contrast",
            "--text-rgb",
            "#000000",
            "--json",
        ],
    );
    assert_eq!(passed["result"]["counts"]["low_contrast"], 0);
}

#[test]
fn f1_capture_contrast_with_local_png_passes_and_fails() {
    let temp = tempfile::tempdir().expect("temp");
    let project = temp.path().join("ContrastProject");
    let _ = project_with_defaults(&project);
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let png = temp.path().join("white.png");
    std::fs::write(&png, f1_png(4, 4, [255, 255, 255])).expect("png");
    let source = png.to_str().unwrap();

    let passed = f1_run(
        &home,
        &project,
        std::path::Path::new("unused"),
        &[
            "capture",
            "contrast",
            "--rgb",
            "#000000",
            "--region",
            "0,0,4,4",
            "--majority",
            "--source",
            source,
            "--json",
        ],
    );
    assert_eq!(passed["status"], "success", "{passed:#}");
    assert_eq!(passed["result"]["passed"], true);
    assert_eq!(passed["result"]["background"], "#FFFFFF");

    let failed = f1_run(
        &home,
        &project,
        std::path::Path::new("unused"),
        &[
            "capture", "contrast", "--rgb", "#808080", "--at", "1,1", "--source", source, "--json",
        ],
    );
    assert_eq!(failed["status"], "error", "{failed:#}");
    assert_eq!(failed["error"]["code"], "E_UI_ASSERTION_FAILED");
    assert!(
        failed["error"]["context"]["result"]["finding"]["rule"] == "low_contrast",
        "{failed:#}"
    );
}

#[test]
fn f1_capture_baseline_save_compare_and_update_with_local_pngs() {
    let temp = tempfile::tempdir().expect("temp");
    let project = temp.path().join("BaselineProject");
    let _ = project_with_defaults(&project);
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let first = temp.path().join("first.png");
    let second = temp.path().join("second.png");
    std::fs::write(&first, f1_png(2, 2, [10, 20, 30])).expect("first png");
    std::fs::write(&second, f1_png(2, 2, [200, 210, 220])).expect("second png");
    let unused = std::path::Path::new("unused");
    let first_path = first.to_str().unwrap();
    let second_path = second.to_str().unwrap();

    let saved = f1_run(
        &home,
        &project,
        unused,
        &[
            "capture",
            "baseline",
            "save",
            "--name",
            "home_screen",
            "--source",
            first_path,
            "--json",
        ],
    );
    assert_eq!(saved["status"], "success", "{saved:#}");
    assert_eq!(saved["result"]["entry"]["name"], "home_screen");
    f1_validate("visual-baseline.schema.json", &saved["result"]["entry"]);

    let duplicate = f1_run(
        &home,
        &project,
        unused,
        &[
            "capture",
            "baseline",
            "save",
            "--name",
            "home_screen",
            "--source",
            first_path,
            "--json",
        ],
    );
    assert_eq!(duplicate["status"], "error", "{duplicate:#}");
    assert_eq!(duplicate["error"]["code"], "E_INVALID_ARGUMENT");

    let matched = f1_run(
        &home,
        &project,
        unused,
        &[
            "capture",
            "baseline",
            "compare",
            "--name",
            "home_screen",
            "--current",
            first_path,
            "--json",
        ],
    );
    assert_eq!(matched["status"], "success", "{matched:#}");
    assert_eq!(matched["result"]["comparison"]["passed"], true);

    let changed = f1_run(
        &home,
        &project,
        unused,
        &[
            "capture",
            "baseline",
            "compare",
            "--name",
            "home_screen",
            "--current",
            second_path,
            "--region",
            "0,0,2,2",
            "--json",
        ],
    );
    assert_eq!(changed["status"], "error", "{changed:#}");
    assert_eq!(changed["error"]["code"], "E_UI_ASSERTION_FAILED");
    assert_eq!(
        changed["error"]["context"]["comparison"]["diff"]["changed_pixels"],
        4
    );

    let updated = f1_run(
        &home,
        &project,
        unused,
        &[
            "capture",
            "baseline",
            "update",
            "--name",
            "home_screen",
            "--source",
            second_path,
            "--json",
        ],
    );
    assert_eq!(updated["result"]["updated"], true, "{updated:#}");
    let recompare = f1_run(
        &home,
        &project,
        unused,
        &[
            "capture",
            "baseline",
            "compare",
            "--name",
            "home_screen",
            "--current",
            second_path,
            "--json",
        ],
    );
    assert_eq!(
        recompare["result"]["comparison"]["passed"], true,
        "{recompare:#}"
    );
    let listed = f1_run(
        &home,
        &project,
        unused,
        &["capture", "baseline", "list", "--json"],
    );
    assert_eq!(listed["result"]["baselines"][0]["name"], "home_screen");
}

fn scenario_run_json(home: &std::path::Path, project: &std::path::Path, args: &[&str]) -> Value {
    let output = binary()
        .current_dir(project)
        .env("ANDROIDFORGE_HOME", home)
        .env_remove("ANDROIDFORGE_SESSION")
        .args(["--project", project.to_str().unwrap()])
        .args(args)
        .output()
        .expect("scenario command");
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "parse {args:?}: {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn scenario_project() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = tempfile::tempdir().expect("temp project");
    let project = temp.path().join("ScenarioProject");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    project_with_defaults(&project);
    (temp, project, home)
}

const SCENARIO_TOML: &str = r#"
version = 1
name = "smoke"
description = "host-agnostic smoke scenario"
[repeat]
count = 2
[[steps]]
type = "launch"
package = "com.example.contexttest"
[[steps]]
type = "assert"
[steps.expect]
text = "Save"
"#;

const SCENARIO_JSON: &str = r#"{"version":1,"name":"json-smoke","repeat":{"count":3},"steps":[{"type":"screenshot","name":"home"},{"type":"text","selector":{"text":"Email"},"value":"user@example.com","on_error":"continue"}]}"#;

#[test]
fn scenario_validate_parses_toml_json_and_sniffs_json_body() {
    let (_temp, project, home) = scenario_project();
    let toml_path = project.join("smoke.toml");
    std::fs::write(&toml_path, SCENARIO_TOML).unwrap();
    let json_path = project.join("smoke.json");
    std::fs::write(&json_path, SCENARIO_JSON).unwrap();
    let sniff_path = project.join("sniff.toml");
    std::fs::write(&sniff_path, SCENARIO_JSON).unwrap();

    let toml = scenario_run_json(
        &home,
        &project,
        &[
            "scenario",
            "validate",
            toml_path.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(toml["status"], "success", "{toml:#}");
    assert_eq!(toml["command"], "scenario.validate");
    assert_eq!(toml["result"]["name"], "smoke");
    assert_eq!(toml["result"]["repeat"], 2);
    assert_eq!(toml["result"]["steps"].as_array().unwrap().len(), 2);
    assert_eq!(
        toml["result"]["steps"][0]["audited_operation"],
        "app.launch"
    );

    let json = scenario_run_json(
        &home,
        &project,
        &[
            "scenario",
            "validate",
            json_path.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(json["result"]["name"], "json-smoke");
    assert_eq!(json["result"]["repeat"], 3);
    assert_eq!(json["result"]["steps"][1]["on_error"], "continue");

    let sniffed = scenario_run_json(
        &home,
        &project,
        &[
            "scenario",
            "validate",
            sniff_path.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(sniffed["result"]["name"], "json-smoke", "{sniffed:#}");
}

#[test]
fn scenario_run_dry_run_plans_without_device_or_consent() {
    let (_temp, project, home) = scenario_project();
    let path = project.join("smoke.toml");
    std::fs::write(&path, SCENARIO_TOML).unwrap();

    // No device, no session, no --yes and a mutating launch step: a dry run
    // must succeed, plan every step and never dispatch.
    let value = scenario_run_json(
        &home,
        &project,
        &[
            "scenario",
            "run",
            path.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(value["status"], "success", "{value:#}");
    assert_eq!(value["result"]["dry_run"], true);
    assert_eq!(value["result"]["iterations"].as_array().unwrap().len(), 2);
    for iteration in value["result"]["iterations"].as_array().unwrap() {
        for step in iteration["steps"].as_array().unwrap() {
            assert_eq!(step["status"], "planned", "{value:#}");
        }
    }
    assert!(
        value.get("run_id").is_none(),
        "dry run must not create a run"
    );
    assert!(
        value["result"]["summary"]["flake"]["flaky_steps"]
            .as_array()
            .is_none_or(Vec::is_empty)
    );
}

#[test]
fn scenario_run_repeat_override_and_unsafe_step() {
    let (_temp, project, home) = scenario_project();
    let path = project.join("smoke.toml");
    std::fs::write(&path, SCENARIO_TOML).unwrap();
    let repeated = scenario_run_json(
        &home,
        &project,
        &[
            "scenario",
            "run",
            path.to_str().unwrap(),
            "--dry-run",
            "--repeat",
            "5",
            "--json",
        ],
    );
    assert_eq!(repeated["result"]["plan"]["repeat"], 5, "{repeated:#}");
    assert_eq!(
        repeated["result"]["iterations"].as_array().unwrap().len(),
        5
    );
    assert_eq!(repeated["result"]["summary"]["iterations"], 5);

    let unsafe_path = project.join("unsafe.json");
    std::fs::write(
        &unsafe_path,
        r#"{"version":1,"name":"unsafe","steps":[{"type":"shell","command":"id"}]}"#,
    )
    .unwrap();
    let rejected = scenario_run_json(
        &home,
        &project,
        &[
            "scenario",
            "validate",
            unsafe_path.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(rejected["status"], "error", "{rejected:#}");
    assert_eq!(rejected["error"]["code"], "E_CONFIG_INVALID");
    assert!(
        serde_json::to_string(&rejected)
            .unwrap()
            .contains("unsafe scenario step")
    );
}

#[test]
fn scenario_run_with_fake_ui_honours_step_policy_and_records_run() {
    let png = f1_png(2, 2, [255, 255, 255]);
    let (_temp, project, adb) = f1_fixture(FAKE_UI_XML, &png);
    let home = adb.parent().expect("adb parent");
    let pass_path = project.join("pass.toml");
    std::fs::write(
        &pass_path,
        "version = 1\nname = \"pass-smoke\"\n[repeat]\ncount = 1\n[[steps]]\ntype = \"assert\"\n[steps.expect]\ntext = \"Save\"\n",
    )
    .unwrap();
    let pass = f1_run(
        home,
        &project,
        &adb,
        &[
            "scenario",
            "run",
            pass_path.to_str().unwrap(),
            "--yes",
            "--json",
        ],
    );
    assert_eq!(pass["status"], "success", "{pass:#}");
    assert_eq!(pass["result"]["summary"]["passed"], 1);
    assert_eq!(
        pass["result"]["iterations"][0]["steps"][0]["status"],
        "success"
    );
    assert!(pass.get("run_id").is_some(), "real run must be recorded");

    // A failing step with `continue` still runs the following step, and the
    // scenario is reported as failed overall.
    let fail_path = project.join("fail.toml");
    std::fs::write(
        &fail_path,
        r#"
version = 1
name = "fail-smoke"
[repeat]
count = 1
[[steps]]
type = "assert"
on_error = "continue"
[steps.expect]
text = "Missing"
[[steps]]
type = "screenshot"
name = "after"
"#,
    )
    .unwrap();
    let failed = f1_run(
        home,
        &project,
        &adb,
        &[
            "scenario",
            "run",
            fail_path.to_str().unwrap(),
            "--yes",
            "--json",
        ],
    );
    assert_eq!(failed["status"], "failed", "{failed:#}");
    assert_eq!(
        failed["result"]["iterations"][0]["steps"][0]["status"],
        "failed"
    );
    assert_eq!(
        failed["result"]["iterations"][0]["steps"][1]["status"],
        "success"
    );
    assert_eq!(failed["result"]["summary"]["failed"], 1);

    // The default `fail` policy skips later steps; every planned step still
    // appears in the recorded run report.
    let abort_path = project.join("abort.toml");
    std::fs::write(
        &abort_path,
        r#"
version = 1
name = "abort-smoke"
[[steps]]
type = "assert"
[steps.expect]
text = "Missing"
[[steps]]
type = "screenshot"
name = "never"
"#,
    )
    .unwrap();
    let aborted = f1_run(
        home,
        &project,
        &adb,
        &[
            "scenario",
            "run",
            abort_path.to_str().unwrap(),
            "--yes",
            "--json",
        ],
    );
    assert_eq!(aborted["status"], "failed", "{aborted:#}");
    assert_eq!(
        aborted["result"]["iterations"][0]["steps"][1]["status"],
        "skipped"
    );
    let run_id = aborted["run_id"].as_str().expect("run id");
    let report_path = project
        .join(".artifacts")
        .join("runs")
        .join(run_id)
        .join("report.json");
    let report: Value = serde_json::from_slice(
        &std::fs::read(&report_path)
            .unwrap_or_else(|error| panic!("read {report_path:?}: {error}")),
    )
    .expect("report json");
    assert_eq!(report["steps"].as_array().unwrap().len(), 2, "{report:#}");
    assert_eq!(report["steps"][1]["status"], "skipped", "{report:#}");
}

#[test]
fn scenario_run_requires_yes_for_mutating_steps() {
    let png = f1_png(2, 2, [255, 255, 255]);
    let (_temp, project, adb) = f1_fixture(FAKE_UI_XML, &png);
    let home = adb.parent().expect("adb parent");
    let path = project.join("mutate.toml");
    std::fs::write(
        &path,
        r#"
version = 1
name = "mutating"
[[steps]]
type = "tap"
[steps.selector]
text = "Save"
"#,
    )
    .unwrap();
    let blocked = f1_run(
        home,
        &project,
        &adb,
        &["scenario", "run", path.to_str().unwrap(), "--json"],
    );
    assert_eq!(blocked["status"], "error", "{blocked:#}");
    assert_eq!(
        blocked["error"]["code"],
        "E_PHYSICAL_DEVICE_MUTATION_BLOCKED"
    );
    assert!(serde_json::to_string(&blocked).unwrap().contains("consent"));
}

// ---------------------------------------------------------------------------
// Issue #3 P0: artifact secret scanning and forced bundle redaction
// ---------------------------------------------------------------------------

fn g1_project() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("temp project");
    let config = temp.path().join(".androidforge");
    std::fs::create_dir_all(&config).expect("adapter dir");
    std::fs::write(
        config.join("project.toml"),
        "adapter_version = 1\n[project]\nname = \"ScanTest\"\npackage = \"com.example.scan\"\n[redaction]\nenabled = false\n[diagnostics]\nredact = false\n",
    )
    .expect("adapter");
    std::fs::create_dir_all(temp.path().join("home")).expect("home");
    temp
}

fn g1_json(home: &std::path::Path, project: &std::path::Path, args: &[&str]) -> Value {
    let output = binary()
        .current_dir(project)
        .env("ANDROIDFORGE_HOME", home)
        .env_remove("ANDROIDFORGE_SESSION")
        .args(args)
        .arg("--json")
        .output()
        .expect("cli");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "parse {args:?}: {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn artifacts_scan_secrets_finds_fixture_kinds_without_echoing_values() {
    let temp = g1_project();
    let store = temp.path().join(".artifacts").join("runs").join("run-1");
    std::fs::create_dir_all(store.join("logs")).expect("run dir");
    let google = format!("AIza{}", "x".repeat(35));
    let openai = format!("sk-{}", "a".repeat(30));
    let github = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let aws = "AKIAIOSFODNN7EXAMPLE";
    std::fs::write(
        store.join("logs/fixture.txt"),
        format!("google={google}\nopenai={openai}\ngithub={github}\naws={aws}\n"),
    )
    .expect("fixture");
    // Binary media and oversized text must be disclosed but not scanned.
    std::fs::write(store.join("shot.png"), format!("\0PNG {google}")).expect("binary");
    let oversized = format!("safe line\n{}", google.repeat(200_000));
    std::fs::write(store.join("logs/big.txt"), oversized).expect("oversized");

    let output = g1_json(
        &temp.path().join("home"),
        temp.path(),
        &["artifacts", "scan-secrets"],
    );
    assert_eq!(output["status"], "success", "{output:#}");
    let result = &output["result"];
    let kinds = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["kind"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    for expected in [
        "google_api_key",
        "openai_key",
        "github_token",
        "aws_access_key",
    ] {
        assert!(
            kinds.iter().any(|kind| kind == expected),
            "missing {expected}: {output:#}"
        );
    }
    assert_eq!(result["findings_total"], 4, "{output:#}");
    assert!(result["files_skipped_binary"].as_u64().unwrap_or(0) >= 1);
    assert!(result["files_skipped_oversized"].as_u64().unwrap_or(0) >= 1);
    assert_eq!(result["truncated"], true, "{output:#}");
    for secret in [&google, &openai, github, aws] {
        assert!(
            !serde_json::to_string(result).unwrap().contains(secret),
            "a secret value leaked"
        );
    }
    for finding in result["findings"].as_array().unwrap() {
        assert!(finding.get("path").is_some());
        assert!(finding.get("line").is_some());
        assert!(finding.get("start").is_some());
        assert!(finding.get("end").is_some());
        assert!(
            finding["preview"]
                .as_str()
                .unwrap_or_default()
                .contains("REDACTED")
        );
    }
}

#[test]
fn report_bundle_redact_secrets_forces_redaction_and_refuses_unredactable_payload() {
    use androidforge_core::model::ArtifactRef;
    use androidforge_core::run::{RunReport, RunStatus};

    let temp = g1_project();
    let home = temp.path().join("home");
    let run = temp
        .path()
        .join(".artifacts")
        .join("runs")
        .join("run-secret");
    std::fs::create_dir_all(&run).expect("run dir");
    let google = format!("AIza{}", "x".repeat(35));
    // A screenshot's bytes are never text-redacted, so a plain-text secret in
    // an opted-in media payload must make the bundle refuse to ship.
    std::fs::write(run.join("screen.png"), format!("raw {google}\n")).expect("media");
    let mut report = RunReport::new(
        "run-secret",
        "test",
        "ScanTest",
        temp.path().to_str().unwrap(),
    );
    report.status = RunStatus::Passed;
    report
        .artifacts
        .push(ArtifactRef::new("screenshot", "screen.png"));
    std::fs::write(
        run.join("report.json"),
        serde_json::to_vec_pretty(&report).expect("report"),
    )
    .expect("report file");

    let output_path = temp.path().join("refused.zip");
    let refused = g1_json(
        &home,
        temp.path(),
        &[
            "report",
            "bundle",
            "run-secret",
            "--redact-secrets",
            "--include-screenshots",
            "--output",
            output_path.to_str().unwrap(),
        ],
    );
    assert_eq!(refused["status"], "error", "{refused:#}");
    assert_eq!(refused["error"]["code"], "E_UNSUPPORTED_OPERATION");
    assert_eq!(refused["exit_code"], 3, "{refused:#}");
    assert!(
        refused["error"]["context"]["findings_total"]
            .as_u64()
            .unwrap_or(0)
            > 0
    );
    assert!(refused["error"]["context"]["kinds"].is_object());
    assert!(
        !serde_json::to_string(&refused).unwrap().contains(&google),
        "the error leaked a value"
    );
    assert!(!output_path.exists(), "a partial bundle was written");

    // Without the flag the 1.0.0 behavior is preserved: no refusal, no write
    // refusal, and no additive manifest fields.
    let legacy = g1_json(
        &home,
        temp.path(),
        &[
            "report",
            "bundle",
            "run-secret",
            "--include-screenshots",
            "--format",
            "json",
            "--output",
            temp.path().join("legacy.json").to_str().unwrap(),
        ],
    );
    assert_eq!(legacy["status"], "success", "{legacy:#}");
    assert!(temp.path().join("legacy.json").exists());
    assert!(legacy["result"]["manifest"].get("redact_secrets").is_none());
}

#[test]
fn tools_catalog_and_mcp_schema_expose_the_new_safety_surface() {
    let temp = g1_project();
    let home = temp.path().join("home");
    let catalog = g1_json(&home, temp.path(), &["tools", "catalog"]);
    let commands = catalog["result"]["commands"]
        .as_array()
        .expect("catalog commands");
    for path in [
        "artifacts scan-secrets",
        "devices shell",
        "devices input tap",
    ] {
        assert!(
            commands.iter().any(|command| command["path"] == path),
            "catalog is missing {path}"
        );
    }
    for path in ["devices shell", "devices input tap"] {
        let command = commands
            .iter()
            .find(|command| command["path"] == path)
            .expect("catalog entry");
        assert_eq!(command["safety"], "device-mutation", "{path}");
    }
    let schema = g1_json(&home, temp.path(), &["tools", "schema", "--format", "mcp"]);
    let tools = schema["result"]["tools"].as_array().expect("MCP tools");
    for name in [
        "androidforge_artifacts_scan_secrets",
        "androidforge_devices_shell",
        "androidforge_devices_input_tap",
    ] {
        assert!(
            tools.iter().any(|tool| tool["name"] == name),
            "MCP is missing {name}"
        );
    }
    let shell = tools
        .iter()
        .find(|tool| tool["name"] == "androidforge_devices_shell")
        .expect("device shell MCP tool");
    for physical in ["allow_physical_mutation", "allow_physical_input"] {
        assert!(
            shell["inputSchema"]["properties"].get(physical).is_none(),
            "MCP advertises {physical}"
        );
    }
}

#[test]
fn jsonl_emits_minified_ndjson_with_a_final_envelope() {
    let temp = tempfile::tempdir().unwrap();
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["schema", "list", "--jsonl"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let values = ndjson_values(&output);
    assert_eq!(values.len(), 1, "one envelope line for an ordinary command");
    assert_eq!(values[0]["schema_version"], 1);
    assert_eq!(values[0]["status"], "success");
    assert_eq!(values[0]["command"], "schema.list");
    let text = std::str::from_utf8(&output).unwrap();
    assert!(!text.contains("\n  "), "NDJSON must be minified: {text}");

    // Default `--json` keeps its pretty shape when the flag is absent.
    let pretty = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["schema", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(std::str::from_utf8(&pretty).unwrap().contains("\n  "));

    // `--jsonl` still works with an output profile and keeps its shape.
    let compact = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["--output-profile", "compact", "schema", "list", "--jsonl"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let compact_values = ndjson_values(&compact);
    assert_eq!(compact_values.len(), 1);
    assert_eq!(compact_values[0]["profile"], "compact");
    assert_eq!(compact_values[0]["result"], values[0]["result"]);
}

#[test]
fn fields_trims_only_the_result_and_rejects_bad_keys() {
    let temp = tempfile::tempdir().unwrap();
    for args in [
        vec!["schema", "list", "--json", "--fields", "schemas"],
        vec!["--fields", "schemas", "schema", "list", "--json"],
        vec!["--fields=schemas", "schema", "list", "--json"],
    ] {
        let output = binary()
            .current_dir(temp.path())
            .env("ANDROIDFORGE_HOME", temp.path())
            .args(&args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["status"], "success");
        assert_eq!(json["command"], "schema.list");
        assert!(json["result"].is_object());
        assert_eq!(json["result"].as_object().unwrap().len(), 1);
        assert!(json["result"]["schemas"].is_array());
    }

    let unknown = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["schema", "list", "--json", "--fields", "not_a_key"])
        .assert()
        .failure()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&unknown).unwrap();
    assert_eq!(json["status"], "error");
    assert_eq!(json["error"]["code"], "E_INVALID_ARGUMENT");
    let text = serde_json::to_string(&json).unwrap();
    assert!(
        text.contains("schemas"),
        "available keys are listed: {text}"
    );
    assert!(
        text.contains("--output-profile compact|brief"),
        "envelope trimming hint is present: {text}"
    );
}

#[test]
fn fields_without_any_keys_fails_before_the_command_executes() {
    let temp = tempfile::tempdir().unwrap();
    for (fields, with_stats) in [("", false), (", ,", false), ("", true)] {
        let mut args = vec![
            "run",
            "start",
            "--name",
            "must-not-run",
            "--json",
            "--fields",
            fields,
        ];
        if with_stats {
            args.push("--stats");
        }
        let output = binary()
            .current_dir(temp.path())
            .env("ANDROIDFORGE_HOME", temp.path())
            .args(&args)
            .output()
            .expect("empty fields run");
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        let json: Value = serde_json::from_slice(&output.stdout).expect("error envelope");
        assert_eq!(json["status"], "error");
        assert_eq!(json["error"]["code"], "E_INVALID_ARGUMENT");
        assert!(json.get("result").is_none());
        let text = serde_json::to_string(&json).unwrap();
        assert!(
            text.contains("--fields result-key[,result-key...]"),
            "missing key syntax suggestion: {text}"
        );
        if with_stats {
            assert_stats_match(&stats_from_stderr(&output.stderr), &output.stdout);
        }
    }
    // `run start` would create this directory, so its absence proves the
    // command was never dispatched.
    assert!(!temp.path().join(".artifacts").exists());
}

#[test]
fn parse_error_under_jsonl_is_one_minified_line_with_stats() {
    let temp = tempfile::tempdir().unwrap();
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["emualtor", "strt", "--jsonl", "--stats"])
        .output()
        .expect("jsonl parse error");
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let text = std::str::from_utf8(&output.stdout).expect("utf8 stdout");
    assert!(!text.contains("\n  "), "NDJSON must be minified: {text}");
    let values = ndjson_values(&output.stdout);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["status"], "error");
    assert_eq!(values[0]["error"]["code"], "E_INVALID_COMMAND");
    assert_stats_match(&stats_from_stderr(&output.stderr), &output.stdout);
}

#[test]
fn stats_reports_exact_stdout_bytes_on_stderr_without_changing_stdout() {
    let temp = tempfile::tempdir().unwrap();
    for profile in [None, Some("compact"), Some("brief")] {
        let mut args = vec!["schema", "list", "--json", "--stats"];
        if let Some(profile) = profile {
            args.extend(["--output-profile", profile]);
        }
        let output = binary()
            .current_dir(temp.path())
            .env("ANDROIDFORGE_HOME", temp.path())
            .args(&args)
            .output()
            .expect("stats run");
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice::<Value>(&output.stdout).expect("stdout stays JSON");
        assert_stats_match(&stats_from_stderr(&output.stderr), &output.stdout);
    }

    // Errors report the bytes actually written (the JSON error envelope).
    let error = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["emualtor", "strt", "--json", "--stats"])
        .output()
        .expect("error stats run");
    assert_eq!(error.status.code(), Some(2));
    let envelope: Value = serde_json::from_slice(&error.stdout).expect("error envelope");
    assert_eq!(envelope["status"], "error");
    assert_stats_match(&stats_from_stderr(&error.stderr), &error.stdout);

    // --quiet does not suppress the stats line; in human mode stdout is empty.
    let quiet = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["schema", "list", "--quiet", "--stats"])
        .output()
        .expect("quiet stats run");
    assert!(quiet.status.success());
    assert!(quiet.stdout.is_empty(), "{quiet:?}");
    assert_stats_match(&stats_from_stderr(&quiet.stderr), &quiet.stdout);
}

#[test]
fn jsonl_scenario_run_streams_iterations_before_the_summary() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("stream.toml");
    std::fs::write(
        &manifest,
        "version = 1\nname = \"streamed\"\n[repeat]\ncount = 2\n[[steps]]\ntype = \"assert\"\n[steps.expect]\ntext = \"Ready\"\n",
    )
    .unwrap();
    let path = manifest.to_str().unwrap();
    let output = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["scenario", "run", path, "--dry-run", "--jsonl"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let values = ndjson_values(&output);
    assert_eq!(values.len(), 3, "two iterations plus one summary envelope");
    for (index, value) in values[..2].iter().enumerate() {
        assert_eq!(value["event"], "iteration");
        assert_eq!(value["iteration"], serde_json::json!(index as u64 + 1));
        assert_eq!(value["status"], "passed");
        assert!(value["duration_ms"].is_number());
        assert!(value["steps"].is_array());
        assert!(value.get("result").is_none(), "no duplicated full result");
    }
    assert_eq!(values[2]["schema_version"], 1);
    assert_eq!(values[2]["status"], "success");
    assert_eq!(values[2]["command"], "scenario.run");
    assert_eq!(values[2]["result"]["summary"]["iterations"], 2);

    let without = binary()
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", temp.path())
        .args(["scenario", "run", path, "--dry-run", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let without: Value = serde_json::from_slice(&without).expect("single JSON envelope");
    assert_eq!(without["schema_version"], 1);
    assert_eq!(without["result"]["summary"]["iterations"], 2);
}

#[test]
fn observe_semantics_and_test_tag_flags_are_advertised_but_not_presentation_flags() {
    let observe = binary()
        .args(["ui", "observe", "--help"])
        .output()
        .expect("observe help");
    let help = String::from_utf8(observe.stdout).expect("utf8 help");
    assert!(help.contains("--semantics"), "{help}");

    let catalog = binary()
        .args(["tools", "catalog", "--json"])
        .output()
        .expect("catalog");
    let catalog: Value = serde_json::from_slice(&catalog.stdout).expect("catalog JSON");
    let entry = catalog["result"]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == "ui observe")
        .expect("ui observe catalog entry")
        .clone();
    assert!(
        entry["arguments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|argument| argument["long"] == "semantics")
    );

    let tools = binary()
        .args(["tools", "schema", "--format", "mcp", "--json"])
        .output()
        .expect("tools schema");
    let tools: Value = serde_json::from_slice(&tools.stdout).expect("tools JSON");
    let definitions = tools["result"]["tools"].as_array().unwrap();
    let observe_tool = definitions
        .iter()
        .find(|tool| tool["name"] == "androidforge_ui_observe")
        .expect("observe MCP tool");
    assert!(observe_tool["inputSchema"]["properties"]["semantics"].is_object());
    let text = serde_json::to_string(definitions).unwrap();
    for presentation in ["--jsonl", "--fields", "--stats", "jsonl", "stats"] {
        assert!(
            !text.contains(&format!("\"{presentation}\"")),
            "MCP must not expose {presentation}"
        );
    }
}
