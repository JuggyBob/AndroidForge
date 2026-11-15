use std::time::Duration;

use androidforge_process::{CommandSpec, ProcessError, Runner};

#[tokio::test]
async fn captures_stdout_and_stderr_without_console_encoding() {
    let output = Runner::new(false)
        .run(CommandSpec::new("rustc").arg("--version"))
        .await
        .expect("rustc");
    assert!(output.success);
    assert!(output.stdout_text().contains("rustc"));
    assert!(!output.stdout_truncated);
}

#[cfg(windows)]
#[tokio::test]
async fn path_with_spaces_parentheses_and_unicode() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("path with spaces (测试)");
    std::fs::create_dir_all(&directory).unwrap();
    let ping = directory.join("ping.exe");
    std::fs::copy(r"C:\Windows\System32\ping.exe", &ping).unwrap();
    let output = Runner::new(false)
        .run(
            CommandSpec::new(ping.to_string_lossy().into_owned())
                .args(["-n", "1", "127.0.0.1"])
                .timeout(Duration::from_secs(10)),
        )
        .await
        .expect("ping");
    assert!(output.success);
}

#[cfg(windows)]
#[tokio::test]
async fn timeout_terminates_a_process_and_reports_it() {
    let ping = r"C:\Windows\System32\ping.exe";
    let error = Runner::new(false)
        .run(
            CommandSpec::new(ping)
                .args(["-n", "20", "127.0.0.1"])
                .timeout(Duration::from_millis(250)),
        )
        .await
        .expect("runner returns bounded output on timeout")
        .require_success()
        .expect_err("timeout expected");
    match error {
        ProcessError::Timeout { .. } => {}
        other => panic!("expected timeout, got {other:?}"),
    }
}

#[cfg(windows)]
#[tokio::test]
async fn output_capture_is_bounded() {
    let ping = r"C:\Windows\System32\ping.exe";
    let output = Runner::new(false)
        .run(
            CommandSpec::new(ping)
                .args(["-n", "3", "127.0.0.1"])
                .timeout(Duration::from_secs(10))
                .max_stdout(32),
        )
        .await
        .expect("ping");
    assert!(output.stdout_truncated);
    assert_eq!(output.stdout.len(), 32);
}
