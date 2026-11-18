use std::fs;
use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

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
