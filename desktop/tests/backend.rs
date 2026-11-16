use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use androidforge_desktop::backend::DesktopError;
use androidforge_desktop::backend::cli_runner::{CliRunner, Invocation};
use androidforge_desktop::backend::compatibility;
use androidforge_desktop::backend::process_manager::{self, OperationEvent};
use androidforge_desktop::models::DeviceList;

struct MockCli {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn mock_cli() -> &'static MockCli {
    static MOCK: OnceLock<MockCli> = OnceLock::new();
    MOCK.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("mock.rs");
        let exe = dir.path().join(if cfg!(windows) {
            "androidforge.exe"
        } else {
            "androidforge"
        });
        std::fs::write(&source, MOCK_SOURCE).unwrap();
        let status = std::process::Command::new("rustc")
            .args(["--edition", "2021", "-o"])
            .arg(&exe)
            .arg(&source)
            .status()
            .expect("rustc is required for desktop backend integration tests");
        assert!(status.success(), "could not compile mock CLI");
        MockCli {
            _dir: dir,
            path: exe,
        }
    })
}

fn runner() -> CliRunner {
    CliRunner::new(mock_cli().path.clone())
}

#[test]
fn probes_a_matching_cli_and_parses_schema_support() {
    let info = compatibility::probe(&runner()).unwrap();
    assert_eq!(info.version, "0.2.0");
    assert_eq!(info.report_schema, 1);
}

#[test]
fn parses_typed_command_results_and_keeps_stderr_separate() {
    let output = runner()
        .run_success(&Invocation::new(["devices", "list"]).timeout(Duration::from_secs(10)))
        .unwrap();
    assert!(output.stderr.contains("mock stderr"));
    let devices: DeviceList =
        androidforge_desktop::backend::json_parser::parse_result(&output.envelope).unwrap();
    assert_eq!(devices.count, 1);
    assert_eq!(devices.devices[0].serial, "emulator-5554");
}

#[test]
fn preserves_structured_error_envelopes() {
    let error = runner()
        .run_success(&Invocation::new(["fail"]).timeout(Duration::from_secs(10)))
        .unwrap_err();
    match error {
        DesktopError::CliError { code, message, raw } => {
            assert_eq!(code, "E_TEST_FAILURE");
            assert!(message.contains("intentional"));
            assert!(raw.contains("E_TEST_FAILURE"));
        }
        other => panic!("expected structured CLI error, got {other:?}"),
    }
}

#[test]
fn rejects_invalid_json_without_panicking() {
    let error = runner()
        .run(&Invocation::new(["badjson"]).timeout(Duration::from_secs(10)))
        .unwrap_err();
    assert!(matches!(error, DesktopError::InvalidJson { .. }));
}

#[test]
fn times_out_and_kills_a_hung_cli() {
    let started = std::time::Instant::now();
    let error = runner()
        .run(&Invocation::new(["hang"]).timeout(Duration::from_millis(250)))
        .unwrap_err();
    assert!(matches!(error, DesktopError::CliTimeout { .. }));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn streams_output_and_finishes_an_operation() {
    let operation = process_manager::spawn_operation(
        &runner(),
        &Invocation::new(["stream"]).timeout(Duration::from_secs(10)),
        1,
        "stream",
    )
    .unwrap();
    let mut stderr = Vec::new();
    let mut finished = None;
    for _ in 0..100 {
        while let Ok(event) = operation.receiver.try_recv() {
            match event {
                OperationEvent::Stdout(_) => {}
                OperationEvent::Stderr(line) => stderr.push(line),
                OperationEvent::Finished(outcome) => finished = Some(outcome),
            }
        }
        if finished.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let outcome = finished.expect("operation should finish");
    assert!(outcome.success);
    assert!(stderr.iter().any(|line| line.contains("one")));
    assert!(outcome.envelope.is_some());
}

#[test]
fn cancels_a_running_operation_and_reports_cancelled() {
    let operation = process_manager::spawn_operation(
        &runner(),
        &Invocation::new(["hang"]).timeout(Duration::from_secs(60)),
        2,
        "hang",
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(operation.cancel(), "the process tree should be terminated");
    let mut finished = None;
    for _ in 0..100 {
        while let Ok(event) = operation.receiver.try_recv() {
            if let OperationEvent::Finished(outcome) = event {
                finished = Some(outcome);
            }
        }
        if finished.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let outcome = finished.expect("cancelled operation should finish");
    assert!(outcome.cancelled);
    assert!(!outcome.success);
    assert!(outcome.duration_ms < 10_000);
}

#[test]
fn missing_explicit_executable_is_reported() {
    let error = androidforge_desktop::backend::cli_locator::resolve(Some(Path::new(
        "definitely-missing-androidforge.exe",
    )))
    .unwrap_err();
    assert!(matches!(error, DesktopError::CliNotFound { .. }));
}

const MOCK_SOURCE: &str = r##"
use std::time::Duration;

fn emit(text: &str) {
    println!("{text}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--version") {
        println!("androidforge 0.2.0");
        return;
    }
    if args.iter().any(|arg| arg == "schema") {
        emit(r#"{"schema_version":1,"status":"success","command":"schema.list","exit_code":0,"result":{"schemas":["androidforge.schema.json","adapter.schema.json","fixture.schema.json","hook.schema.json","conformance.schema.json"]},"duration_ms":1}"#);
        return;
    }
    if args.iter().any(|arg| arg == "devices") {
        eprintln!("mock stderr");
        emit(r#"{"schema_version":1,"status":"success","command":"devices.list","exit_code":0,"result":{"count":1,"devices":[{"serial":"emulator-5554","state":"device","model":"Mock","manufacturer":"Test","android_version":"16","api_level":36,"architecture":"x86_64","kind":"emulator"}]},"duration_ms":1}"#);
        return;
    }
    if args.iter().any(|arg| arg == "fail") {
        emit(r#"{"schema_version":1,"status":"error","command":"fail","exit_code":7,"error":{"code":"E_TEST_FAILURE","message":"intentional mock failure","recoverable":true,"suggestions":["inspect"]},"duration_ms":1}"#);
        std::process::exit(7);
    }
    if args.iter().any(|arg| arg == "badjson") {
        println!("this is not json");
        return;
    }
    if args.iter().any(|arg| arg == "hang") {
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    if args.iter().any(|arg| arg == "stream") {
        for line in ["one", "two", "three"] {
            eprintln!("{line}");
            std::thread::sleep(Duration::from_millis(20));
        }
        emit(r#"{"schema_version":1,"status":"success","command":"stream","exit_code":0,"result":{},"duration_ms":1}"#);
        return;
    }
    emit(r#"{"schema_version":1,"status":"success","command":"unknown","exit_code":0,"result":{},"duration_ms":1}"#);
}
"##;
