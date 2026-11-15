use std::io::Write;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::output::{CommandSuccess, write_raw_stdout};
use androidforge_process::{decode_text, looks_like_text};
use base64::Engine;

use crate::cli::AdbArgs;
use crate::context::CliContext;

pub async fn run(ctx: &CliContext, args: &AdbArgs) -> Result<CommandSuccess> {
    let mut arguments = args.args.clone();
    if arguments.first().map(String::as_str) == Some("--") {
        arguments.remove(0);
    }
    if arguments.is_empty() {
        return Err(ForgeError::invalid_argument(
            "adb requires a command such as `shell dumpsys jobscheduler`",
        ));
    }
    if arguments.iter().any(|argument| argument == "--json") {
        return Err(ForgeError::invalid_argument(
            "the global --json flag must come before `adb`; raw arguments are passed to the device",
        )
        .with_suggestion("Use `androidforge --json adb ...` or put --json before `adb`."));
    }
    let session = ctx.load_session(true)?;
    let mut serial = session
        .as_ref()
        .and_then(|session| session.serial.clone())
        .or_else(|| ctx.serial_arg.clone())
        .or_else(|| std::env::var("ANDROIDFORGE_SERIAL").ok())
        .filter(|serial| !serial.trim().is_empty());
    if is_global_adb_command(&arguments) {
        serial = None;
    }
    if let Some(serial) = &serial {
        androidforge_core::session::check_serial_ownership(
            &ctx.paths,
            serial,
            session.as_ref().map(|session| session.id.as_str()),
        )?;
    }
    let timeout = ctx.parse_timeout(args.timeout.as_deref(), ctx.config.file.timeouts.adb_secs)?;
    let adb = ctx.adb()?;
    let output = adb
        .raw_passthrough(
            serial.as_deref(),
            &arguments,
            Some(timeout),
            Some(64 * 1024 * 1024),
        )
        .await?;
    if !ctx.json && !ctx.quiet {
        let _ = write_raw_stdout(&output.stdout);
        if !output.stderr.is_empty() {
            let mut stderr = std::io::stderr().lock();
            let _ = stderr.write_all(&output.stderr);
            let _ = stderr.flush();
        }
    }
    let stdout = if looks_like_text(&output.stdout) {
        Some(decode_text(&output.stdout))
    } else {
        None
    };
    let stdout_base64 = if stdout.is_none() && !output.stdout.is_empty() {
        Some(base64::engine::general_purpose::STANDARD.encode(&output.stdout))
    } else {
        None
    };
    let result = serde_json::json!({
        "unsafe": true,
        "command": args.args,
        "serial": serial,
        "exit_code": output.exit_code,
        "timed_out": output.timed_out,
        "stdout_encoding": if stdout.is_some() { "utf-8" } else if stdout_base64.is_some() { "base64" } else { "empty" },
        "stdout": stdout,
        "stdout_base64": stdout_base64,
        "stderr": output.stderr_text(),
        "duration_ms": output.duration_ms,
        "warning": "Raw ADB passthrough bypasses AndroidForge ownership safety beyond the session check. It is intentionally unsafe."
    });
    let mut success = CommandSuccess::new("adb", result, String::new())
        .with_optional_session(session.map(|session| session.id.clone()))
        .with_optional_device(serial);
    success.exit_code = if output.timed_out {
        ErrorCode::CommandTimeout.exit_code()
    } else {
        output
            .exit_code
            .unwrap_or(if output.success { 0 } else { 6 })
    };
    Ok(success)
}

fn is_global_adb_command(args: &[String]) -> bool {
    matches!(
        args.first().map(String::as_str),
        Some(
            "devices"
                | "version"
                | "start-server"
                | "kill-server"
                | "reconnect"
                | "disconnect"
                | "help"
                | "wait-for-device"
        )
    )
}
