use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

const PHYSICAL_SERIAL: &str = "R58M123456A";

fn temp_project() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("temp project");
    let config = temp.path().join(".androidforge");
    fs::create_dir_all(&config).expect("adapter dir");
    fs::write(
        config.join("project.toml"),
        "adapter_version = 1\n[project]\nname = \"SafetyTest\"\npackage = \"com.example.safety\"\n",
    )
    .expect("adapter");
    temp
}

fn run_json(home: &Path, temp: &Path, args: &[&str]) -> Value {
    let output = Command::cargo_bin("androidforge")
        .expect("binary")
        .current_dir(temp)
        .env("ANDROIDFORGE_HOME", home)
        .env_remove("ANDROIDFORGE_SESSION")
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run {args:?}: {error}"));
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "parse {args:?}: {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn run_raw(home: &Path, temp: &Path, args: &[&str]) -> (String, String) {
    let output = Command::cargo_bin("androidforge")
        .expect("binary")
        .current_dir(temp)
        .env("ANDROIDFORGE_HOME", home)
        .env_remove("ANDROIDFORGE_SESSION")
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run {args:?}: {error}"));
    (
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn e1_select_session(home: &Path, session: &androidforge_core::session::Session) {
    let dir = home.join("state").join("sessions").join(&session.id);
    fs::create_dir_all(&dir).expect("session dir");
    fs::write(
        dir.join("session.json"),
        serde_json::to_vec_pretty(session).expect("session JSON"),
    )
    .expect("write session");
    let pointer_dir = home.join(".androidforge");
    fs::create_dir_all(&pointer_dir).expect("pointer dir");
    fs::write(pointer_dir.join("current-session"), &session.id).expect("select session");
}

/// Scenario controls must use the narrow risk flag: data settings require
/// --allow-physical-data and network changes require --allow-physical-network.
/// A dry run neither checks consent nor touches the device.
#[test]
fn device_scenario_respects_data_and_network_risk() {
    use androidforge_core::session::{Session, SessionKind, SessionState};

    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let adb = fake_adb(&home);
    let mut session = Session::new(
        "tester",
        SessionKind::Attached,
        Some(PHYSICAL_SERIAL.to_string()),
        None,
        None,
        true,
    );
    session.state = SessionState::Ready;
    e1_select_session(&home, &session);

    for (flag, expected_suggestion) in [
        ("--night", "--allow-physical-data"),
        ("--locale", "--allow-physical-data"),
        ("--font", "--allow-physical-data"),
        ("--network", "--allow-physical-network"),
    ] {
        let value = match flag {
            "--night" => vec![flag, "yes"],
            "--locale" => vec![flag, "fr-FR"],
            "--font" => vec![flag, "1.15"],
            _ => vec![flag, "offline"],
        };
        let mut args = vec!["devices", "scenario", "--session", &session.id];
        args.extend(&value);
        args.push("--json");
        let blocked = run_device_json(&home, temp.path(), &adb, &args);
        assert_blocked(&blocked, expected_suggestion);
    }

    // An unrelated narrow flag must not unlock the network risk.
    let mut wrong_flag = vec![
        "devices",
        "scenario",
        "--session",
        &session.id,
        "--network",
        "offline",
        "--allow-physical-data",
        "--json",
    ];
    let blocked = run_device_json(&home, temp.path(), &adb, &wrong_flag);
    assert_blocked(&blocked, "--allow-physical-network");

    // Dry runs are pure: no consent flag and no device mutation.
    wrong_flag.insert(2, "--dry-run");
    let preview = run_device_json(&home, temp.path(), &adb, &wrong_flag);
    assert_eq!(preview["status"], "success", "{preview:#}");
    assert_eq!(preview["result"]["plan"]["will_mutate"], false);
    assert_eq!(preview["result"]["applied"], false);
}

/// Failed-session cleanup only forgets tombstones owned by the caller (or by
/// anyone with --force), refuses live sessions and never steals a foreign lock.
#[test]
fn session_gc_forget_terminal_requires_owner_and_refuses_live_locks() {
    use androidforge_core::session::{DeviceLock, Session, SessionKind, SessionState};

    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");

    let mut alice = Session::new(
        "alice",
        SessionKind::Emulator,
        Some("emulator-5554".to_string()),
        Some("Pixel_Test".to_string()),
        Some(5554),
        true,
    );
    alice.id = "ses-failed-alice".to_string();
    alice.state = SessionState::Failed;
    e1_select_session(&home, &alice);

    let mut bob = Session::new(
        "bob",
        SessionKind::Emulator,
        Some("emulator-5556".to_string()),
        None,
        Some(5556),
        true,
    );
    bob.id = "ses-failed-bob".to_string();
    bob.state = SessionState::Abandoned;
    e1_select_session(&home, &bob);

    let mut live = Session::new(
        "alice",
        SessionKind::Attached,
        Some("emulator-5558".to_string()),
        None,
        None,
        true,
    );
    live.id = "ses-live-alice".to_string();
    live.state = SessionState::Ready;
    e1_select_session(&home, &live);

    let preview = run_json(
        &home,
        temp.path(),
        &[
            "session",
            "gc",
            "--forget-terminal",
            "--owner",
            "alice",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(preview["status"], "success", "{preview:#}");
    let would = preview["result"]["forget_terminal"]["would_forget"]
        .as_array()
        .expect("would_forget")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(would, vec!["ses-failed-alice"]);
    // A dry run changed nothing.
    let alice_path = home.join("state/sessions/ses-failed-alice/session.json");
    let stored: Value =
        serde_json::from_slice(&fs::read(&alice_path).expect("session")).expect("JSON");
    assert_eq!(stored["state"], "failed");

    // No owner and no force is refused.
    let refused = run_json(
        &home,
        temp.path(),
        &["session", "gc", "--forget-terminal", "--json"],
    );
    assert_eq!(refused["status"], "error");
    assert_eq!(refused["error"]["code"], "E_INVALID_ARGUMENT");

    let applied = run_json(
        &home,
        temp.path(),
        &[
            "session",
            "gc",
            "--forget-terminal",
            "--owner",
            "alice",
            "--json",
        ],
    );
    assert_eq!(applied["status"], "success", "{applied:#}");
    let forgotten = applied["result"]["forget_terminal"]["forgotten"]
        .as_array()
        .expect("forgotten");
    assert_eq!(forgotten.len(), 1);
    assert_eq!(forgotten[0], "ses-failed-alice");
    let stored: Value =
        serde_json::from_slice(&fs::read(&alice_path).expect("session")).expect("JSON");
    assert_eq!(stored["state"], "released");
    let still_live: Value = serde_json::from_slice(
        &fs::read(home.join("state/sessions/ses-live-alice/session.json")).expect("session"),
    )
    .expect("JSON");
    assert_eq!(still_live["state"], "ready");

    // With --force, tombstones of other owners are forgotten too; the live
    // session is still excluded because it is not terminal.
    let forced = run_json(
        &home,
        temp.path(),
        &["session", "gc", "--forget-terminal", "--force", "--json"],
    );
    assert_eq!(forced["status"], "success", "{forced:#}");
    let forgotten = forced["result"]["forget_terminal"]["forgotten"]
        .as_array()
        .expect("forgotten")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(forgotten.contains(&"ses-failed-bob"));
    assert!(!forgotten.contains(&"ses-live-alice"));

    // A tombstone whose device lock belongs to a different session is refused
    // and the foreign lock is left untouched, even with --force.
    let mut foreign = Session::new(
        "carol",
        SessionKind::Emulator,
        Some("emulator-5554".to_string()),
        None,
        Some(5554),
        true,
    );
    foreign.id = "ses-failed-carol".to_string();
    foreign.state = SessionState::Failed;
    e1_select_session(&home, &foreign);
    let lock_path = home.join("state/locks/devices/emulator-5554.json");
    fs::create_dir_all(lock_path.parent().expect("lock dir")).expect("lock dir");
    fs::write(
        &lock_path,
        serde_json::to_vec_pretty(&DeviceLock {
            schema_version: 1,
            session_id: "ses-live-alice".to_string(),
            serial: "emulator-5554".to_string(),
            owner: "alice".to_string(),
            acquired_at: chrono::Utc::now().to_rfc3339(),
        })
        .expect("lock JSON"),
    )
    .expect("write lock");
    let before = fs::read(&lock_path).expect("lock before");
    let blocked = run_json(
        &home,
        temp.path(),
        &[
            "session",
            "gc",
            "--forget-terminal",
            "--owner",
            "carol",
            "--force",
            "--json",
        ],
    );
    assert_eq!(blocked["status"], "success", "{blocked:#}");
    let failures = blocked["result"]["forget_terminal"]["failures"]
        .as_array()
        .expect("failures");
    assert!(
        failures.iter().any(|failure| {
            failure["session_id"] == "ses-failed-carol" && failure["code"] == "E_DEVICE_BUSY"
        }),
        "{failures:?}"
    );
    assert_eq!(fs::read(&lock_path).expect("lock after"), before);
}

#[test]
fn scoped_physical_flags_are_global_and_accepted() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let flags = [
        "--allow-physical-app-control",
        "--allow-physical-data",
        "--allow-physical-network",
        "--allow-physical-install",
        "--allow-physical-input",
        "--allow-physical-mutation",
    ];
    for flag in flags {
        let value = run_json(&home, temp.path(), &["config", "show", "--json", flag]);
        assert_eq!(value["status"], "success", "{flag}: {value:#}");
    }
    let (stdout, _) = run_raw(&home, temp.path(), &["--help"]);
    for flag in flags {
        assert!(stdout.contains(flag), "--help is missing {flag}: {stdout}");
    }
}

#[test]
fn mcp_tool_catalog_never_advertises_physical_capability_flags() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let (stdout, stderr) = run_raw(
        &home,
        temp.path(),
        &["tools", "schema", "--format", "json", "--json"],
    );
    let value: Value = serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!("parse tools schema: {error}\nstdout: {stdout}\nstderr: {stderr}")
    });
    assert_eq!(value["status"], "success", "{value:#}");
    let serialized = serde_json::to_string(&value).expect("serialize");
    for flag in [
        "allow_physical_mutation",
        "allow_physical_app_control",
        "allow_physical_data",
        "allow_physical_network",
        "allow_physical_install",
        "allow_physical_input",
    ] {
        assert!(
            !serialized.contains(flag),
            "MCP tool catalog advertises {flag}: {serialized}"
        );
    }
}

#[test]
fn build_is_host_read_only_and_its_mcp_tool_cannot_mutate_devices() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");

    let catalog = run_json(&home, temp.path(), &["tools", "catalog", "--json"]);
    let commands = catalog["result"]["commands"]
        .as_array()
        .expect("catalog commands");
    let build = commands
        .iter()
        .find(|command| command["path"] == "build")
        .expect("build catalog entry");
    assert_eq!(build["safety"], "read-only", "{build:#}");
    // The existing device paths keep their original classification.
    for path in ["app install", "app launch"] {
        let command = commands
            .iter()
            .find(|command| command["path"] == path)
            .unwrap_or_else(|| panic!("missing {path}"));
        assert_eq!(command["safety"], "device-mutation", "{command:#}");
    }

    let schema = run_json(
        &home,
        temp.path(),
        &["tools", "schema", "--format", "json", "--json"],
    );
    let tools = schema["result"]["tools"].as_array().expect("tools");
    let build_tool = tools
        .iter()
        .find(|tool| tool["name"] == "androidforge_build")
        .expect("build tool");
    let properties = build_tool["inputSchema"]["properties"]
        .as_object()
        .expect("build properties");
    for key in [
        "install",
        "launch",
        "cold",
        "fresh",
        "downgrade",
        "only_if_changed",
        "session",
        "serial",
        "confirm",
        "allow_physical_mutation",
        "allow_physical_install",
        "allow_physical_app_control",
    ] {
        assert!(!properties.contains_key(key), "build tool exposes {key}");
    }
}

#[test]
fn bounds_commands_remain_read_only_in_catalog_and_mcp() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");

    let catalog = run_json(&home, temp.path(), &["tools", "catalog", "--json"]);
    let commands = catalog["result"]["commands"]
        .as_array()
        .expect("catalog commands");
    for (path, tool_name) in [
        ("ui bounds", "androidforge_ui_bounds"),
        ("ui assert-bounds", "androidforge_ui_assert_bounds"),
        ("ui wait", "androidforge_ui_wait"),
    ] {
        let entry = commands
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("missing catalog entry {path}"));
        assert_eq!(entry["safety"], "read-only", "{path}");
        assert_eq!(entry["tool_name"], tool_name, "{path}");
    }

    let tools = run_json(
        &home,
        temp.path(),
        &["tools", "schema", "--format", "mcp", "--json"],
    );
    let definitions = tools["result"]["tools"]
        .as_array()
        .expect("MCP tool definitions");
    for name in ["androidforge_ui_bounds", "androidforge_ui_assert_bounds"] {
        let tool = definitions
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing MCP tool {name}"));
        assert!(
            tool["description"]
                .as_str()
                .unwrap_or_default()
                .contains("Read-only")
        );
        let properties = tool["inputSchema"]["properties"]
            .as_object()
            .expect("tool properties");
        for key in ["allow_physical_mutation", "allow_physical_input", "confirm"] {
            assert!(
                !properties.contains_key(key),
                "{name} MCP schema advertises {key}"
            );
        }
    }
}

#[test]
fn metadata_only_bundle_excludes_run_artifacts() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let root = temp.path();
    let project = root.to_string_lossy().to_string();

    let junit = root.join("passing.xml");
    fs::write(
        &junit,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <testsuite name=\"safety\" tests=\"1\" failures=\"0\" errors=\"0\" skipped=\"0\" time=\"0.1\">\n\
         <testcase classname=\"safety.Smoke\" name=\"passes\" time=\"0.1\"/>\n\
         </testsuite>\n",
    )
    .expect("junit");
    let imported = run_json(
        &home,
        root,
        &[
            "report",
            "import",
            "junit",
            junit.to_str().expect("junit path"),
            "--project",
            &project,
            "--json",
        ],
    );
    assert_eq!(imported["status"], "success", "{imported:#}");
    let run_id = imported["result"]["run_id"]
        .as_str()
        .or_else(|| imported["run_id"].as_str())
        .expect("run id")
        .to_string();

    let bundle = run_json(
        &home,
        root,
        &[
            "report",
            "bundle",
            &run_id,
            "--project",
            &project,
            "--format",
            "json",
            "--metadata-only",
            "--json",
        ],
    );
    assert_eq!(bundle["status"], "success", "{bundle:#}");
    let entries = bundle["result"]["manifest"]["entries"]
        .as_array()
        .expect("entries");
    assert!(
        !entries.is_empty(),
        "the manifest still has summary entries"
    );
    for entry in entries {
        let kind = entry["kind"].as_str().unwrap_or_default();
        assert!(
            !matches!(
                kind,
                "evidence" | "screenshot" | "recording" | "logcat" | "hierarchy"
            ),
            "metadata-only bundle included {kind}: {entry:#}"
        );
    }
    let excluded = bundle["result"]["manifest"]["excluded"]
        .as_array()
        .expect("excluded");
    assert!(
        excluded.iter().any(|item| item["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("metadata-only")),
        "metadata-only exclusions should be explicit: {excluded:?}"
    );
}

#[test]
fn unknown_nested_adapter_fields_fail_validation() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let adapter = temp.path().join(".androidforge/project.toml");
    let mut text = fs::read_to_string(&adapter).expect("read adapter");
    text.push_str("unknown_key = true\n");
    fs::write(&adapter, text).expect("write adapter");
    let output = Command::cargo_bin("androidforge")
        .expect("binary")
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", &home)
        .args([
            "config",
            "validate",
            "--project",
            temp.path().to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("run config validate");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let value: Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(value["status"], "error", "{value:#}");
    assert_eq!(value["error"]["code"], "E_CONFIG_INVALID");
}

/// `app data export` must default to the managed artifacts/export area and
/// refuse caller-chosen in-project paths, existing files and `..` traversal
/// unless the caller explicitly opts in. All cases are dry runs: no ADB
/// mutation and no physical-data consent are needed.
#[test]
fn app_data_export_destination_policy_is_enforced_by_the_cli() {
    use androidforge_core::session::{Session, SessionKind, SessionState};

    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let adb = fake_adb(&home);
    let mut session = Session::new(
        "tester",
        SessionKind::Attached,
        Some(PHYSICAL_SERIAL.to_string()),
        None,
        None,
        true,
    );
    session.state = SessionState::Ready;
    e1_select_session(&home, &session);
    let session_id = session.id.clone();
    let project = temp.path().to_string_lossy().into_owned();
    let outside = tempfile::tempdir().expect("outside temp");
    let output = outside.path().join("backup.tar");
    let output_arg = output.to_string_lossy().into_owned();
    let legacy_arg = outside
        .path()
        .join("legacy.tar")
        .to_string_lossy()
        .into_owned();
    let inside_arg = temp
        .path()
        .join("inside.tar")
        .to_string_lossy()
        .into_owned();

    let export = |extra: &[&str]| -> Value {
        let mut args = vec![
            "app",
            "data",
            "export",
            "com.example.app",
            "--session",
            session_id.as_str(),
            "--project",
            project.as_str(),
        ];
        args.extend_from_slice(extra);
        args.push("--json");
        run_device_json(&home, temp.path(), &adb, &args)
    };

    let preview = export(&["--dry-run"]);
    assert_eq!(preview["status"], "success", "{preview:#}");
    assert_eq!(preview["command"], "app.data-export");
    assert_eq!(preview["result"]["dry_run"], true);
    assert_eq!(preview["result"]["source"], "default");
    assert_eq!(preview["result"]["in_export_area"], true);
    let default_path = preview["result"]["path"].as_str().expect("default path");
    assert!(default_path.contains("export"), "{default_path}");
    assert!(default_path.contains("com.example.app"), "{default_path}");

    let explicit = export(&["--output", &output_arg, "--dry-run"]);
    assert_eq!(explicit["status"], "success", "{explicit:#}");
    assert_eq!(explicit["result"]["source"], "explicit");
    assert_eq!(explicit["result"]["path"], output_arg);
    assert_eq!(explicit["result"]["in_project"], false);
    assert_eq!(explicit["result"]["overwrite"], false);

    let legacy = export(&[&legacy_arg, "--dry-run"]);
    assert_eq!(legacy["status"], "success", "{legacy:#}");
    assert_eq!(legacy["result"]["source"], "explicit");
    assert_eq!(legacy["result"]["path"], legacy_arg);

    let refused = export(&["--output", &inside_arg, "--dry-run"]);
    assert_eq!(refused["status"], "error", "{refused:#}");
    assert_eq!(refused["error"]["code"], "E_INVALID_ARGUMENT");
    assert_eq!(
        refused["error"]["context"]["reason"], "destination_inside_project",
        "{refused:#}"
    );
    let overridden = export(&["--output", &inside_arg, "--allow-in-project", "--dry-run"]);
    assert_eq!(overridden["status"], "success", "{overridden:#}");
    assert_eq!(overridden["result"]["in_project"], true);

    fs::write(&output, b"old").expect("existing archive");
    let exists = export(&["--output", &output_arg, "--dry-run"]);
    assert_eq!(exists["status"], "error", "{exists:#}");
    assert_eq!(
        exists["error"]["context"]["reason"], "destination_exists",
        "{exists:#}"
    );
    let replaced = export(&["--output", &output_arg, "--overwrite", "--dry-run"]);
    assert_eq!(replaced["status"], "success", "{replaced:#}");
    assert_eq!(replaced["result"]["overwrite"], true);

    let traversal = export(&["--output", "../escape.tar", "--dry-run"]);
    assert_eq!(traversal["status"], "error", "{traversal:#}");
    assert_eq!(
        traversal["error"]["context"]["reason"], "destination_traversal",
        "{traversal:#}"
    );
}

/// With an externally configured `artifact_dir`, the default export destination
/// is outside the project root and is reported as such.
#[test]
fn app_data_export_default_follows_external_artifact_dir() {
    use androidforge_core::session::{Session, SessionKind, SessionState};

    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let adb = fake_adb(&home);
    let mut session = Session::new(
        "tester",
        SessionKind::Attached,
        Some(PHYSICAL_SERIAL.to_string()),
        None,
        None,
        true,
    );
    session.state = SessionState::Ready;
    e1_select_session(&home, &session);
    let external = tempfile::tempdir().expect("external artifacts");
    let artifact_dir = external.path().to_string_lossy().replace('\\', "/");
    let config = temp.path().join(".androidforge/project.toml");
    let mut text = fs::read_to_string(&config).expect("config");
    text.push_str(&format!("artifact_dir = \"{artifact_dir}\"\n"));
    fs::write(&config, text).expect("write config");

    let preview = run_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "app",
            "data",
            "export",
            "com.example.app",
            "--session",
            session.id.as_str(),
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(preview["status"], "success", "{preview:#}");
    assert_eq!(preview["result"]["source"], "default");
    assert_eq!(preview["result"]["in_export_area"], true);
    assert_eq!(preview["result"]["in_project"], false, "{preview:#}");
}

/// Minimal fake `adb`: reports one non-emulator device for `devices -l` and
/// accepts every other invocation without touching hardware. Gate checks reject
/// before any shell command is dispatched, so the tests never need a real
/// device or an Android SDK.
#[cfg(windows)]
fn fake_adb(dir: &Path) -> PathBuf {
    let path = dir.join("fake-adb.cmd");
    fs::write(
        &path,
        format!(
            "@echo off\r\n\
             if \"%~1\"==\"devices\" (\r\n\
             echo List of devices attached\r\n\
             echo {PHYSICAL_SERIAL} device product:test model:TestDevice transport_id:1\r\n\
             echo.\r\n\
             exit /b 0\r\n\
             )\r\n\
             exit /b 0\r\n"
        ),
    )
    .expect("fake adb");
    path
}

#[cfg(not(windows))]
fn fake_adb(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("fake-adb");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"devices\" ]; then\n\
             printf 'List of devices attached\\n{PHYSICAL_SERIAL} device product:test model:TestDevice transport_id:1\\n\\n'\n\
             fi\n\
             exit 0\n"
        ),
    )
    .expect("fake adb");
    let mut permissions = fs::metadata(&path)
        .expect("fake adb metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("fake adb permissions");
    path
}

fn run_device_json(home: &Path, temp: &Path, adb: &Path, args: &[&str]) -> Value {
    let output = Command::cargo_bin("androidforge")
        .expect("binary")
        .current_dir(temp)
        .env("ANDROIDFORGE_HOME", home)
        .env("ANDROIDFORGE_ADB", adb)
        .env_remove("ANDROIDFORGE_SESSION")
        .args(args)
        .arg("--serial")
        .arg(PHYSICAL_SERIAL)
        .output()
        .unwrap_or_else(|error| panic!("run {args:?}: {error}"));
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "parse {args:?}: {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn assert_blocked(value: &Value, flag: &str) {
    assert_eq!(
        value["status"], "error",
        "expected a physical-device gate error: {value:#}"
    );
    assert_eq!(
        value["error"]["code"], "E_PHYSICAL_DEVICE_MUTATION_BLOCKED",
        "{value:#}"
    );
    let suggestions = value["error"]["suggestions"]
        .as_array()
        .expect("suggestions");
    assert!(
        suggestions
            .iter()
            .any(|item| item.as_str().unwrap_or_default().contains(flag)),
        "error did not suggest {flag}: {value:#}"
    );
}

/// New audit/baseline/contrast commands stay read-only even when a physical
/// serial is selected; OEM confirmation never weakens the install gate.
#[test]
fn f1_audit_safety_classifications_and_install_gate_are_preserved() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let adb = fake_adb(&home);

    let catalog = run_json(&home, temp.path(), &["tools", "catalog", "--json"]);
    let commands = catalog["result"]["commands"]
        .as_array()
        .expect("catalog commands");
    for path in [
        "ui audit a11y",
        "capture baseline save",
        "capture baseline update",
        "capture baseline compare",
        "capture baseline list",
        "capture contrast",
    ] {
        let command = commands
            .iter()
            .find(|command| command["path"] == path)
            .unwrap_or_else(|| panic!("missing {path}"));
        assert_eq!(command["safety"], "read-only", "{path}");
    }
    let install = commands
        .iter()
        .find(|command| command["path"] == "app install")
        .expect("app install");
    assert_eq!(install["safety"], "device-mutation");

    let schema = run_json(
        &home,
        temp.path(),
        &["tools", "schema", "--format", "json", "--json"],
    );
    let tools = schema["result"]["tools"].as_array().expect("tools");
    let install_tool = tools
        .iter()
        .find(|tool| tool["name"] == "androidforge_app_install")
        .expect("app install tool");
    let properties = install_tool["inputSchema"]["properties"]
        .as_object()
        .expect("install properties");
    assert!(properties.contains_key("wait_for_confirmation"));
    assert!(properties.contains_key("confirm_text"));
    assert!(properties.contains_key("confirm"));
    for flag in ["allow_physical_mutation", "allow_physical_install"] {
        assert!(!properties.contains_key(flag), "MCP exposes {flag}");
    }

    // A host-only contrast check needs no physical consent flag.
    let png = temp.path().join("white.png");
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer
            .write_image_data(&[255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255])
            .expect("png data");
    }
    fs::write(&png, bytes).expect("png");
    let project_arg = temp.path().to_string_lossy().into_owned();
    let png_arg = png.to_string_lossy().into_owned();
    let strs = [
        "--project",
        project_arg.as_str(),
        "capture",
        "contrast",
        "--rgb",
        "#000000",
        "--source",
        png_arg.as_str(),
        "--json",
    ];
    let output = Command::cargo_bin("androidforge")
        .expect("binary")
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", &home)
        .env("ANDROIDFORGE_ADB", &adb)
        .env_remove("ANDROIDFORGE_SESSION")
        .args(strs)
        .arg("--serial")
        .arg(PHYSICAL_SERIAL)
        .output()
        .expect("contrast");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let value: Value = serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "parse contrast: {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(value["status"], "success", "{value:#}");
    assert!(
        !strs.iter().any(|arg| arg.contains("allow-physical")),
        "no physical consent flag was passed"
    );
}

#[test]
fn ui_input_is_gated_on_physical_devices() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let adb = fake_adb(&home);
    let tap = ["ui", "tap", "--x", "10", "--y", "10", "--json"];

    let blocked = run_device_json(&home, temp.path(), &adb, &tap);
    assert_blocked(&blocked, "--allow-physical-input");
    assert!(
        blocked["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("tapping the screen"),
        "{}",
        blocked["error"]["message"]
    );

    let unrelated = run_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "ui",
            "tap",
            "--x",
            "10",
            "--y",
            "10",
            "--allow-physical-data",
            "--json",
        ],
    );
    assert_blocked(&unrelated, "--allow-physical-input");

    let mut scoped = Vec::from(tap);
    scoped.insert(2, "--allow-physical-input");
    let allowed = run_device_json(&home, temp.path(), &adb, &scoped);
    assert_eq!(allowed["status"], "success", "{allowed:#}");

    let mut master = Vec::from(tap);
    master.insert(2, "--allow-physical-mutation");
    let superset = run_device_json(&home, temp.path(), &adb, &master);
    assert_eq!(superset["status"], "success", "{superset:#}");

    let mut dry = Vec::from(tap);
    dry.insert(2, "--dry-run");
    let preview = run_device_json(&home, temp.path(), &adb, &dry);
    assert_eq!(preview["status"], "success", "{preview:#}");
    assert_eq!(preview["result"]["dry_run"], true, "{preview:#}");
}

#[test]
fn network_reverse_is_gated_on_physical_devices() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let adb = fake_adb(&home);
    let add = ["network", "reverse", "add", "tcp:18081", "--json"];

    let blocked = run_device_json(&home, temp.path(), &adb, &add);
    assert_blocked(&blocked, "--allow-physical-network");

    let unrelated = run_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "network",
            "reverse",
            "add",
            "tcp:18081",
            "--allow-physical-input",
            "--json",
        ],
    );
    assert_blocked(&unrelated, "--allow-physical-network");

    let mut scoped = Vec::from(add);
    scoped.insert(2, "--allow-physical-network");
    let allowed = run_device_json(&home, temp.path(), &adb, &scoped);
    assert_eq!(allowed["status"], "success", "{allowed:#}");

    let mut dry = Vec::from(add);
    dry.insert(2, "--dry-run");
    let preview = run_device_json(&home, temp.path(), &adb, &dry);
    assert_eq!(preview["status"], "success", "{preview:#}");
    assert_eq!(preview["result"]["dry_run"], true, "{preview:#}");
}

/// Fixture declarative state mutates app-private data. The CLI must opt in to
/// data mutation only after the physical-device gate, and a dry run must not
/// require that consent or contact ADB at all.
#[test]
fn fixture_state_requires_data_consent_and_dry_run_is_pure() {
    use androidforge_core::session::{Session, SessionKind, SessionState};

    let temp = temp_project();
    let fixtures = temp.path().join(".androidforge/fixtures");
    fs::create_dir_all(&fixtures).expect("fixtures directory");
    fs::write(
        fixtures.join("state-demo.json"),
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
    fs::create_dir_all(&home).expect("home");
    let adb = fake_adb(&home);

    let mut session = Session::new(
        "tester",
        SessionKind::Attached,
        Some(PHYSICAL_SERIAL.to_string()),
        None,
        None,
        true,
    );
    session.state = SessionState::Ready;
    e1_select_session(&home, &session);

    let apply = [
        "fixture",
        "apply",
        "state-demo",
        "--session",
        session.id.as_str(),
        "--json",
    ];
    let blocked = run_device_json(&home, temp.path(), &adb, &apply);
    assert_blocked(&blocked, "--allow-physical-data");

    let mut wrong_flag = Vec::from(apply);
    wrong_flag.insert(3, "--allow-physical-network");
    let unrelated = run_device_json(&home, temp.path(), &adb, &wrong_flag);
    assert_blocked(&unrelated, "--allow-physical-data");

    // A dry run validates the pack and plans the action without a data-risk
    // flag; no session is even required for the planning path.
    let mut dry = Vec::from(apply);
    dry.insert(3, "--dry-run");
    let preview = run_device_json(&home, temp.path(), &adb, &dry);
    assert_eq!(preview["status"], "success", "{preview:#}");
    assert_eq!(preview["command"], "fixture.apply");
    assert_eq!(preview["result"]["dry_run"], true);
    assert_eq!(preview["result"]["action"], "apply_state");
    assert_eq!(preview["result"]["state_declared"], true);

    let dry_reset = vec![
        "fixture",
        "reset",
        "state-demo",
        "--session",
        session.id.as_str(),
        "--dry-run",
        "--json",
    ];
    let reset_preview = run_device_json(&home, temp.path(), &adb, &dry_reset);
    assert_eq!(reset_preview["status"], "success", "{reset_preview:#}");
    assert_eq!(reset_preview["result"]["action"], "reset_state");

    // Reset is destructive: the scoped data flag alone is not enough without
    // an explicit --yes confirmation.
    let unconfirmed = run_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "fixture",
            "reset",
            "state-demo",
            "--session",
            session.id.as_str(),
            "--allow-physical-data",
            "--json",
        ],
    );
    assert_eq!(unconfirmed["status"], "error", "{unconfirmed:#}");
    assert_eq!(unconfirmed["error"]["code"], "E_INVALID_ARGUMENT");
    assert!(
        unconfirmed["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not confirmed")
            || unconfirmed["error"]["suggestions"]
                .as_array()
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| item.as_str().unwrap_or_default().contains("--yes"))
                }),
        "{unconfirmed:#}"
    );
}

/// A scenario's coarse consent gate and its per-step scoped gates are both
/// preserved. An unrelated scoped flag cannot unlock an input step.
#[test]
fn f2_scenario_run_preserves_physical_gates() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let adb = fake_adb(&home);
    let path = temp.path().join("tap.toml");
    fs::write(
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
    .expect("scenario manifest");
    let scenario = path.to_str().expect("scenario path");

    // No --yes: the core refuses every mutating scenario before dispatch.
    let blocked = run_device_json(
        &home,
        temp.path(),
        &adb,
        &["scenario", "run", scenario, "--json"],
    );
    assert_eq!(blocked["status"], "error", "{blocked:#}");
    assert_eq!(
        blocked["error"]["code"], "E_PHYSICAL_DEVICE_MUTATION_BLOCKED",
        "{blocked:#}"
    );
    assert!(
        serde_json::to_string(&blocked).unwrap().contains("consent"),
        "{blocked:#}"
    );

    // --yes plus an unrelated scoped flag passes the coarse gate but the tap
    // is still blocked by its own Risk::Input gate.
    let wrong_scope = run_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "scenario",
            "run",
            scenario,
            "--yes",
            "--allow-physical-data",
            "--json",
        ],
    );
    assert_eq!(wrong_scope["status"], "failed", "{wrong_scope:#}");
    let step = &wrong_scope["result"]["iterations"][0]["steps"][0];
    assert_eq!(
        step["error_code"], "E_PHYSICAL_DEVICE_MUTATION_BLOCKED",
        "{wrong_scope:#}"
    );
    assert!(
        step["error_message"]
            .as_str()
            .unwrap_or_default()
            .contains("--allow-physical-mutation"),
        "{wrong_scope:#}"
    );

    // The narrow input flag passes the gate; the step may still fail for UI
    // reasons against the fake device, but never for consent.
    let allowed = run_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "scenario",
            "run",
            scenario,
            "--yes",
            "--allow-physical-input",
            "--json",
        ],
    );
    let allowed_step = &allowed["result"]["iterations"][0]["steps"][0];
    assert_ne!(
        allowed_step["error_code"].as_str().unwrap_or_default(),
        "E_PHYSICAL_DEVICE_MUTATION_BLOCKED",
        "{allowed:#}"
    );

    // Dry run neither checks consent nor forwards a device mutation flag.
    let dry = run_device_json(
        &home,
        temp.path(),
        &adb,
        &["scenario", "run", scenario, "--dry-run", "--json"],
    );
    assert_eq!(dry["status"], "success", "{dry:#}");
    assert_eq!(dry["result"]["dry_run"], true);
    assert_eq!(
        dry["result"]["iterations"][0]["steps"][0]["status"],
        "planned"
    );
}

// ---------------------------------------------------------------------------
// Issue #3 P0: read-only shell, gated input, dry-run and logcat redaction
// ---------------------------------------------------------------------------

fn fake_adb_logcat(dir: &Path, log: &Path) -> PathBuf {
    let google = format!("AIza{}", "x".repeat(35));
    let openai = format!("sk-{}", "a".repeat(30));
    let line1 = format!("11-25 12:00:00.123  1234  1234 I TestTag: google {google}");
    let line2 = format!("11-25 12:00:01.123  1234  1234 I TestTag: openai {openai}");
    #[cfg(windows)]
    {
        let path = dir.join("fake-logcat-adb.cmd");
        fs::write(
            &path,
            format!(
                "@echo off\r\n\
                 echo %* >> \"{}\"\r\n\
                 if \"%~1\"==\"devices\" (\r\n\
                 echo List of devices attached\r\n\
                 echo {PHYSICAL_SERIAL} device product:test model:TestDevice transport_id:1\r\n\
                 echo.\r\n\
                 exit /b 0\r\n\
                 )\r\n\
                 echo %* | findstr /i \"logcat\" >nul\r\n\
                 if not errorlevel 1 (\r\n\
                 echo {line1}\r\n\
                 echo {line2}\r\n\
                 )\r\n\
                 exit /b 0\r\n",
                log.display()
            ),
        )
        .expect("fake adb");
        path
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-logcat-adb");
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = \"devices\" ]; then\nprintf 'List of devices attached\\n{PHYSICAL_SERIAL} device product:test model:TestDevice transport_id:1\\n\\n'\nexit 0\nfi\ncase \"$*\" in\n  *logcat*) printf '%s\\n' '{}' '{}' ;;\nesac\nexit 0\n",
                log.display(),
                line1,
                line2
            ),
        )
        .expect("fake adb");
        let mut permissions = fs::metadata(&path)
            .expect("fake adb metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("fake adb permissions");
        path
    }
}

fn g1_session(home: &Path) -> androidforge_core::session::Session {
    use androidforge_core::session::{Session, SessionKind, SessionState};
    let mut session = Session::new(
        "tester",
        SessionKind::Attached,
        Some(PHYSICAL_SERIAL.to_string()),
        None,
        None,
        true,
    );
    session.id = "ses-g1-device".to_string();
    session.state = SessionState::Ready;
    e1_select_session(home, &session);
    session
}

/// Like `run_json`, but with the fake ADB exported and without appending a
/// trailing `--serial` (which would be swallowed by a trailing-var-arg shell
/// command after `--`).
fn g1_device_json(home: &Path, temp: &Path, adb: &Path, args: &[&str]) -> Value {
    let output = Command::cargo_bin("androidforge")
        .expect("binary")
        .current_dir(temp)
        .env("ANDROIDFORGE_HOME", home)
        .env("ANDROIDFORGE_ADB", adb)
        .env_remove("ANDROIDFORGE_SESSION")
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run {args:?}: {error}"));
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "parse {args:?}: {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn devices_shell_read_only_allowlist_and_deny_are_enforced() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let adb = fake_adb_logcat(temp.path(), &temp.path().join("adb-invocations.log"));
    let session = g1_session(&home);
    let id = session.id.as_str();

    let ok = g1_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "device",
            "shell",
            "--read-only",
            "--session",
            id,
            "--json",
            "--",
            "dumpsys",
            "gfxinfo",
            "com.example",
        ],
    );
    assert_eq!(ok["status"], "success", "{ok:#}");
    assert_eq!(ok["command"], "devices.shell", "{ok:#}");
    assert_eq!(ok["result"]["read_only"], true);
    assert_eq!(ok["result"]["argv"][0], "dumpsys");
    assert_eq!(ok["result"]["output_truncated"], false);
    assert_eq!(ok["result"]["redaction_enabled"], true);
    assert_eq!(ok["result"]["classification"], "read-only");

    for (command, reason) in [
        (
            vec!["settings", "put", "secure", "x", "1"],
            "mutating token",
        ),
        (vec!["cat", "/etc/passwd"], "not allowlisted"),
        (vec!["reboot"], "mutating token"),
    ] {
        let mut args = vec!["device", "shell", "--read-only", "--json"];
        args.extend(command.iter().copied());
        let blocked = run_json(&home, temp.path(), &args);
        assert_eq!(blocked["status"], "error", "{blocked:#}");
        assert_eq!(blocked["error"]["code"], "E_INVALID_ARGUMENT");
        assert!(
            serde_json::to_string(&blocked)
                .unwrap()
                .to_ascii_lowercase()
                .contains(reason),
            "{blocked:#} does not mention {reason}"
        );
    }

    let denied = run_json(
        &home,
        temp.path(),
        &[
            "device",
            "shell",
            "--read-only",
            "--json",
            "--deny",
            "dumpsys",
            "--",
            "dumpsys",
            "gfxinfo",
            "com.example",
        ],
    );
    assert_eq!(denied["error"]["code"], "E_INVALID_ARGUMENT", "{denied:#}");

    let custom = run_json(
        &home,
        temp.path(),
        &[
            "device",
            "shell",
            "--read-only",
            "--json",
            "--allow",
            "getprop",
            "--",
            "dumpsys",
            "gfxinfo",
            "com.example",
        ],
    );
    assert_eq!(custom["error"]["code"], "E_INVALID_ARGUMENT", "{custom:#}");
    let allowed = g1_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "device",
            "shell",
            "--read-only",
            "--json",
            "--allow",
            "dumpsys",
            "--session",
            id,
            "--",
            "dumpsys",
            "gfxinfo",
            "com.example",
        ],
    );
    assert_eq!(allowed["status"], "success", "{allowed:#}");
}

#[test]
fn devices_shell_mutation_needs_master_grant_and_tap_needs_input_risk() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let adb = fake_adb_logcat(temp.path(), &temp.path().join("adb-invocations.log"));
    let session = g1_session(&home);
    let id = session.id.as_str();

    let blocked = g1_device_json(
        &home,
        temp.path(),
        &adb,
        &["device", "shell", "--json", "--session", id, "--", "id"],
    );
    assert_blocked(&blocked, "--allow-physical-mutation");

    let wrong_scoped = g1_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "device",
            "shell",
            "--json",
            "--session",
            id,
            "--allow-physical-input",
            "--",
            "id",
        ],
    );
    assert_blocked(&wrong_scoped, "--allow-physical-mutation");

    let allowed = g1_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "device",
            "shell",
            "--json",
            "--session",
            id,
            "--allow-physical-mutation",
            "--",
            "id",
        ],
    );
    assert_eq!(allowed["status"], "success", "{allowed:#}");
    assert_eq!(allowed["result"]["classification"], "mutating");

    let tap_blocked = g1_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "device",
            "input",
            "tap",
            "540",
            "1200",
            "--session",
            id,
            "--json",
        ],
    );
    assert_blocked(&tap_blocked, "--allow-physical-input");
    let tap_allowed = g1_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "device",
            "input",
            "tap",
            "540",
            "1200",
            "--session",
            id,
            "--allow-physical-input",
            "--json",
        ],
    );
    assert_eq!(tap_allowed["status"], "success", "{tap_allowed:#}");
    assert_eq!(
        tap_allowed["command"], "devices.input.tap",
        "{tap_allowed:#}"
    );
    assert_eq!(tap_allowed["result"]["receipt"]["dispatched"], true);
    assert!(tap_allowed["result"].get("display_size").is_some());
}

#[test]
fn device_dry_run_prints_intent_and_never_invokes_adb() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let log = temp.path().join("adb-invocations.log");
    let adb = fake_adb_logcat(temp.path(), &log);

    let shell = g1_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "device",
            "shell",
            "--dry-run",
            "--read-only",
            "--json",
            "--",
            "dumpsys",
            "gfxinfo",
            "com.example",
        ],
    );
    assert_eq!(shell["status"], "success", "{shell:#}");
    assert_eq!(shell["result"]["dry_run"], true);
    assert_eq!(shell["result"]["would_execute"], true);
    assert_eq!(shell["result"]["argv"][0], "dumpsys");

    let tap = g1_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "device",
            "input",
            "tap",
            "540",
            "1200",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(tap["status"], "success", "{tap:#}");
    assert_eq!(tap["result"]["x"], 540);
    assert_eq!(tap["result"]["y"], 1200);
    assert_eq!(tap["result"]["would_execute"], true);

    assert!(
        !log.exists()
            || fs::read_to_string(&log)
                .unwrap_or_default()
                .trim()
                .is_empty(),
        "dry run invoked ADB"
    );
}

#[test]
fn logcat_tail_query_and_watch_redact_literal_prefixes() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let adb = fake_adb_logcat(temp.path(), &temp.path().join("adb-invocations.log"));
    let session = g1_session(&home);
    let id = session.id.as_str();
    let google = format!("AIza{}", "x".repeat(35));
    let openai = format!("sk-{}", "a".repeat(30));

    let tail = g1_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "diagnostics",
            "logcat",
            "tail",
            "--session",
            id,
            "--redact-patterns",
            "AIza,sk-",
            "--json",
        ],
    );
    assert_eq!(tail["status"], "success", "{tail:#}");
    assert_eq!(tail["result"]["redaction_applied"], true);
    let tail_text = serde_json::to_string(&tail).unwrap();
    assert!(!tail_text.contains(&google));
    assert!(!tail_text.contains(&openai));
    let run_id = tail["run_id"].as_str().expect("run id");
    let stored = fs::read_to_string(
        temp.path()
            .join(".artifacts/runs")
            .join(run_id)
            .join("logs/logcat-tail.txt"),
    )
    .expect("stored tail");
    assert!(!stored.contains(&google), "stored capture leaked a secret");
    assert!(stored.contains("[REDACTED]"));

    let query = g1_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "diagnostics",
            "logcat",
            "query",
            "--session",
            id,
            "--redact-patterns",
            "AIza,sk-",
            "--json",
        ],
    );
    assert_eq!(query["status"], "success", "{query:#}");
    assert_eq!(query["result"]["redaction_applied"], true);
    let records = query["result"]["records"].as_array().expect("records");
    assert!(!records.is_empty());
    for record in records {
        let message = record["message"].as_str().unwrap_or_default();
        assert!(!message.contains(&google));
        assert!(!message.contains(&openai));
        assert!(message.contains("[REDACTED]"));
    }

    let output = Command::cargo_bin("androidforge")
        .expect("binary")
        .current_dir(temp.path())
        .env("ANDROIDFORGE_HOME", &home)
        .env("ANDROIDFORGE_ADB", &adb)
        .env_remove("ANDROIDFORGE_SESSION")
        .args([
            "diagnostics",
            "logcat",
            "watch",
            "--session",
            id,
            "--redact-patterns",
            "AIza,sk-",
            "--timeout",
            "2s",
            "--max-events",
            "4",
        ])
        .output()
        .expect("watch");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(&google), "watch leaked a secret: {stdout}");
    assert!(!stdout.contains(&openai), "watch leaked a secret: {stdout}");
    let mut events = 0usize;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let event: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("watch emitted non-NDJSON {line:?}: {error}"));
        assert_eq!(event["redaction_applied"], true);
        events += 1;
    }
    assert!(events >= 2, "watch emitted {events} event(s): {stdout}");
}

#[test]
fn devices_is_canonical_and_device_alias_is_retained() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let adb = fake_adb(temp.path());
    let session = g1_session(&home);

    let canonical = g1_device_json(&home, temp.path(), &adb, &["devices", "list", "--json"]);
    assert_eq!(canonical["status"], "success", "{canonical:#}");
    assert_eq!(canonical["command"], "devices.list");
    let alias = g1_device_json(&home, temp.path(), &adb, &["device", "list", "--json"]);
    assert_eq!(alias["status"], "success", "{alias:#}");
    assert_eq!(
        alias["command"], "devices.list",
        "alias changed the canonical name"
    );
    assert_eq!(alias["result"]["count"], canonical["result"]["count"]);

    let shell_alias = g1_device_json(
        &home,
        temp.path(),
        &adb,
        &[
            "device",
            "shell",
            "--read-only",
            "--json",
            "--session",
            &session.id,
            "--",
            "getprop",
            "ro.build.version.release",
        ],
    );
    assert_eq!(shell_alias["status"], "success", "{shell_alias:#}");
    assert_eq!(shell_alias["command"], "devices.shell");
}

#[test]
fn read_only_shell_rejects_mutating_date_and_wm_args_before_adb() {
    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let log = temp.path().join("adb-invocations.log");
    let adb = fake_adb_logcat(temp.path(), &log);

    for command in [
        vec!["date", "010100002026"],
        vec!["wm", "size", "1080x1920"],
        vec!["wm", "density", "420"],
    ] {
        let mut args = vec!["device", "shell", "--read-only", "--json", "--"];
        args.extend(command.iter().copied());
        let blocked = g1_device_json(&home, temp.path(), &adb, &args);
        assert_eq!(blocked["status"], "error", "{blocked:#}");
        assert_eq!(
            blocked["error"]["code"], "E_INVALID_ARGUMENT",
            "{blocked:#}"
        );
    }
    assert!(
        !log.exists()
            || fs::read_to_string(&log)
                .unwrap_or_default()
                .trim()
                .is_empty(),
        "a rejected read-only command invoked ADB"
    );
}

/// Fake ADB that records every invocation and answers enough for a performance
/// run to reach `am start -W`. Both a physical and an emulator serial are
/// listed so the CLI can classify each independently.
#[cfg(windows)]
fn fake_adb_performance(dir: &Path, log: &Path) -> PathBuf {
    let path = dir.join("fake-perf-adb.cmd");
    fs::write(
        &path,
        format!(
            "@echo off\r\n\
             echo %* >> \"{}\"\r\n\
             if \"%~1\"==\"devices\" (\r\n\
             echo List of devices attached\r\n\
             echo {PHYSICAL_SERIAL} device product:test model:TestDevice transport_id:1\r\n\
             echo emulator-5554 device product:sdk_gphone model:Android transport_id:2\r\n\
             echo.\r\n\
             exit /b 0\r\n\
             )\r\n\
             echo %* | findstr /i \"pm path\" >nul\r\n\
             if not errorlevel 1 (\r\n\
             echo package:/data/app/com.example.app/base.apk\r\n\
             exit /b 0\r\n\
             )\r\n\
             echo %* | findstr /i \"am start\" >nul\r\n\
             if not errorlevel 1 (\r\n\
             echo Status: OK\r\n\
             echo TotalTime: 100\r\n\
             echo WaitTime: 120\r\n\
             echo ThisTime: 90\r\n\
             exit /b 0\r\n\
             )\r\n\
             exit /b 0\r\n",
            log.display()
        ),
    )
    .expect("fake adb");
    path
}

#[cfg(not(windows))]
fn fake_adb_performance(dir: &Path, log: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("fake-perf-adb");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$*\" >> '{}'\n\
             if [ \"$1\" = \"devices\" ]; then\n\
             printf 'List of devices attached\\n{PHYSICAL_SERIAL} device product:test model:TestDevice transport_id:1\\nemulator-5554 device product:sdk_gphone model:Android transport_id:2\\n\\n'\n\
             exit 0\n\
             fi\n\
             case \"$*\" in\n\
             \t*pm\\ path*) echo package:/data/app/com.example.app/base.apk ;;\n\
             \t*am\\ start*) printf 'Status: OK\\nTotalTime: 100\\nWaitTime: 120\\nThisTime: 90\\n' ;;\n\
             esac\n\
             exit 0\n",
            log.display()
        ),
    )
    .expect("fake adb");
    let mut permissions = fs::metadata(&path)
        .expect("fake adb metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("fake adb permissions");
    path
}

fn performance_json(home: &Path, temp: &Path, adb: &Path, serial: &str, args: &[String]) -> Value {
    let output = Command::cargo_bin("androidforge")
        .expect("binary")
        .current_dir(temp)
        .env("ANDROIDFORGE_HOME", home)
        .env("ANDROIDFORGE_ADB", adb)
        .env_remove("ANDROIDFORGE_SESSION")
        .args(args)
        .arg("--serial")
        .arg(serial)
        .output()
        .unwrap_or_else(|error| panic!("run {args:?}: {error}"));
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "parse {args:?}: {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// Startup measurement launches the app, so a physical device must opt in to
/// app control before `am start` (or `--cold`'s `am force-stop`) is dispatched.
#[test]
fn diagnostics_performance_requires_app_control_on_physical_devices() {
    use androidforge_core::session::{Session, SessionKind, SessionState};

    let temp = temp_project();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let log = temp.path().join("perf-adb-invocations.log");
    let adb = fake_adb_performance(temp.path(), &log);

    let mut session = Session::new(
        "tester",
        SessionKind::Attached,
        Some(PHYSICAL_SERIAL.to_string()),
        None,
        None,
        true,
    );
    session.id = "ses-perf-physical".to_string();
    session.state = SessionState::Ready;
    e1_select_session(&home, &session);

    let args = |extra: &[&str]| -> Vec<String> {
        let mut args = vec![
            "diagnostics".to_string(),
            "performance".to_string(),
            "com.example.app".to_string(),
            "--session".to_string(),
            session.id.clone(),
            "--activity".to_string(),
            "com.example.app/.MainActivity".to_string(),
            "--settle".to_string(),
            "1ms".to_string(),
        ];
        args.extend(extra.iter().map(|value| value.to_string()));
        args.push("--json".to_string());
        args
    };

    let blocked = performance_json(&home, temp.path(), &adb, PHYSICAL_SERIAL, &args(&[]));
    assert_blocked(&blocked, "--allow-physical-app-control");
    assert!(
        blocked["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("collecting startup performance"),
        "{}",
        blocked["error"]["message"]
    );
    let invocations = fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !invocations.contains("am start"),
        "gate ran after app launch: {invocations}"
    );

    // A dry run needs no consent and never launches the app.
    let preview = performance_json(
        &home,
        temp.path(),
        &adb,
        PHYSICAL_SERIAL,
        &args(&["--dry-run"]),
    );
    assert_eq!(preview["status"], "success", "{preview:#}");
    assert_eq!(preview["result"]["dry_run"], true);
    let invocations = fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !invocations.contains("am start"),
        "dry run launched the app: {invocations}"
    );

    // The narrow grant permits the launch; `--cold` is covered by it too.
    let allowed = performance_json(
        &home,
        temp.path(),
        &adb,
        PHYSICAL_SERIAL,
        &args(&["--allow-physical-app-control", "--cold"]),
    );
    assert_eq!(allowed["status"], "success", "{allowed:#}");
    assert!(allowed.get("error").is_none(), "{allowed:#}");
    let invocations = fs::read_to_string(&log).unwrap_or_default();
    assert!(invocations.contains("am start"), "{invocations}");
    assert!(
        invocations.contains("force-stop"),
        "cold measurement did not force-stop: {invocations}"
    );

    // The superset covers the same risk.
    let superset = performance_json(
        &home,
        temp.path(),
        &adb,
        PHYSICAL_SERIAL,
        &args(&["--allow-physical-mutation"]),
    );
    assert_eq!(superset["status"], "success", "{superset:#}");
    assert!(superset.get("error").is_none(), "{superset:#}");

    // Emulators need no grant.
    let emulator_args = [
        "diagnostics",
        "performance",
        "com.example.app",
        "--activity",
        "com.example.app/.MainActivity",
        "--settle",
        "1ms",
        "--json",
    ]
    .iter()
    .map(|value| value.to_string())
    .collect::<Vec<_>>();
    let emulator = performance_json(&home, temp.path(), &adb, "emulator-5554", &emulator_args);
    assert_eq!(emulator["status"], "success", "{emulator:#}");
    assert!(emulator.get("error").is_none(), "{emulator:#}");
}
