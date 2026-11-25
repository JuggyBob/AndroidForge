//! Narrow, audited operations backing `devices shell` and `devices input tap`.
//!
//! `devices shell --read-only` (also reachable through the retained `device`
//! alias) is allowlisted and needs no physical grant. Without `--read-only` it
//! needs the master physical mutation grant only. `devices input tap` uses the
//! existing narrow `Risk::Input` consent path and `UiManager::tap_xy`; there is
//! no shell or input bypass here.

use androidforge_core::error::{ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use serde_json::json;

use crate::cli::{DeviceInputCommand, DeviceInputTapArgs, DeviceShellArgs};
use crate::commands::common::success;
use crate::context::{CliContext, Risk};

/// Conservative read-only prefixes used only when `--allow` is omitted.
const DEFAULT_ALLOW: &[&str] = &[
    "dumpsys",
    "getprop",
    "settings get",
    "pm list",
    "pm path",
    "cmd package list",
    "cmd package path",
    "ls",
    "stat",
    "df",
    "du",
    "date",
    "wm size",
    "wm density",
    "logcat -d",
    "id",
    "whoami",
    "uname",
];

/// Single tokens that turn an otherwise read-looking command into a mutation.
/// These are refused even if an operator adds an overly broad `--allow`.
const ALWAYS_DENY_TOKENS: &[&str] = &[
    "reset",
    "clear",
    "set",
    "put",
    "delete",
    "remove",
    "kill",
    "killall",
    "force-stop",
    "reboot",
    "shutdown",
    "install",
    "uninstall",
    "disable",
    "enable",
    "grant",
    "revoke",
    "wipe",
    "format",
    "su",
    "sh",
    "rm",
    "mv",
    "cp",
    "chmod",
    "chown",
    "input",
    "am",
    "start",
    "stop",
    "broadcast",
];

const MAX_SHELL_OUTPUT: usize = 16 * 1024 * 1024;

/// `logcat` write/rotation flags. `-d` is read-only, but these args clear the
/// buffer or write/rotate a file on the device.
const LOGCAT_WRITE_FLAGS: &[&str] = &[
    "-c",
    "--clear",
    "-f",
    "--file",
    "-r",
    "--rotate",
    "-n",
    "--rotate-count",
];

fn date_read_only(args: &[String]) -> bool {
    let mut format_seen = false;
    for arg in args {
        if arg == "-u" || arg == "--utc" {
            continue;
        }
        if arg.starts_with('+') && arg.len() > 1 && !format_seen {
            format_seen = true;
            continue;
        }
        return false;
    }
    true
}

fn wm_read_only(argv: &[String]) -> bool {
    argv.len() == 2 && matches!(argv[1].as_str(), "size" | "density")
}

fn tokens(prefix: &str) -> Vec<String> {
    prefix
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>()
}

fn starts_with(argv: &[String], prefix: &[String]) -> bool {
    prefix.len() <= argv.len() && argv.iter().zip(prefix).all(|(a, b)| a == b)
}

fn contains(argv: &[String], prefix: &[String]) -> bool {
    if prefix.is_empty() {
        return false;
    }
    argv.windows(prefix.len()).any(|window| window == prefix)
}

/// Validate the read-only contract before any ADB call. Returns the effective
/// allow prefixes (the conservative default when `--allow` was omitted).
fn validate_read_only(argv: &[String], allow: &[String], deny: &[String]) -> Result<Vec<String>> {
    if argv.is_empty() || argv.iter().any(|arg| arg.trim().is_empty()) {
        return Err(ForgeError::invalid_argument(
            "devices shell requires a non-empty command after `--`",
        )
        .with_suggestion(
            "Example: androidforge devices shell --read-only --session <ID> -- dumpsys gfxinfo com.example",
        ));
    }
    for denied in ALWAYS_DENY_TOKENS {
        if argv.iter().any(|arg| arg == denied) {
            return Err(ForgeError::invalid_argument(format!(
                "read-only shell refuses the mutating token '{denied}'"
            ))
            .with_suggestion("Use an explicitly allowlisted read-only diagnostic command."));
        }
    }
    // Some commands accept mutating extra arguments that do not look like a
    // mutating token. Validate their whole argument shape, not just the prefix.
    if argv.first().is_some_and(|arg| arg == "date") && !date_read_only(&argv[1..]) {
        return Err(ForgeError::invalid_argument(
            "read-only shell allows only `date`, `date -u`/`--utc` and one `+FORMAT` argument",
        )
        .with_suggestion(
            "Setting the clock or passing any other argument is a mutation; use the master grant.",
        ));
    }
    if argv.first().is_some_and(|arg| arg == "wm") && !wm_read_only(argv) {
        return Err(ForgeError::invalid_argument(
            "read-only shell allows only exactly `wm size` or `wm density`",
        )
        .with_suggestion(
            "Changing display size, density, rotation or overscan is a mutation; use the master grant.",
        ));
    }
    if argv.first().is_some_and(|arg| arg == "logcat")
        && argv
            .iter()
            .skip(1)
            .any(|arg| LOGCAT_WRITE_FLAGS.contains(&arg.as_str()))
    {
        return Err(ForgeError::invalid_argument(
            "read-only shell refuses logcat clear/write/rotation flags",
        )
        .with_suggestion("Use `logcat -d` with bounded read-only filters."));
    }
    for prefix in deny {
        let prefix = tokens(prefix);
        if prefix.is_empty() {
            return Err(ForgeError::invalid_argument(
                "--deny prefix cannot be empty",
            ));
        }
        if starts_with(argv, &prefix) || contains(argv, &prefix) {
            return Err(ForgeError::invalid_argument(format!(
                "shell command is refused by --deny '{}'",
                prefix.join(" ")
            ))
            .with_suggestion("Remove the denied prefix from the command or change --deny."));
        }
    }
    let effective = if allow.is_empty() {
        DEFAULT_ALLOW
            .iter()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>()
    } else {
        allow.to_vec()
    };
    let matched = effective
        .iter()
        .find(|prefix| starts_with(argv, &tokens(prefix)));
    if matched.is_none() {
        let allowed = effective.join(" | ");
        return Err(ForgeError::invalid_argument(format!(
            "read-only shell command '{}' is not allowlisted",
            argv.join(" ")
        ))
        .with_suggestion(format!(
            "Use one of these prefixes: {allowed}; or pass an explicit --allow <prefix>."
        ))
        .with_suggestion(
            "Use `devices shell` without --read-only only with --allow-physical-mutation.",
        ));
    }
    Ok(effective)
}

pub async fn shell(ctx: &CliContext, args: &DeviceShellArgs) -> Result<CommandSuccess> {
    if args.command.is_empty() {
        return Err(ForgeError::invalid_argument(
            "devices shell requires a command after `--`",
        )
        .with_suggestion(
            "Example: androidforge devices shell --read-only --session <ID> -- getprop ro.build.version.release",
        ));
    }
    let effective_allow = if args.read_only {
        validate_read_only(&args.command, &args.allow, &args.deny)?
    } else if args.allow.is_empty() {
        Vec::new()
    } else {
        args.allow.clone()
    };
    let timeout = ctx.parse_timeout(
        args.timeout.as_deref(),
        ctx.config.file.timeouts.adb_secs.max(1),
    )?;
    if ctx.dry_run {
        return Ok(success(
            "devices.shell",
            json!({
                "dry_run": true,
                "read_only": args.read_only,
                "argv": args.command,
                "allow": effective_allow,
                "deny": args.deny,
                "timeout_secs": timeout.as_secs(),
                "requires_physical_mutation": !args.read_only,
                "would_execute": true,
                "note": "No device was resolved and no shell command was run.",
            }),
            format!(
                "Would run shell argv {:?} (read_only={}, timeout={}s). No device was contacted.\n",
                args.command,
                args.read_only,
                timeout.as_secs()
            ),
        ));
    }
    let session = ctx.require_session()?;
    let device = ctx.resolve_device(Some(&session)).await?;
    // Unrestricted shell keeps the master-only gate: no scoped flag and no new
    // risk variant can authorize it.
    if !args.read_only {
        ctx.ensure_physical_mutation_allowed(&device, "running an unrestricted shell command")?;
    }
    let serial = device.serial.clone();
    let argv = args.command.clone();
    let read_only = args.read_only;
    let deny = args.deny.clone();
    execute_in_run!(
        ctx,
        "devices.shell",
        Some("devices shell"),
        Some(&session),
        Some(&device),
        true,
        |run| async {
            let adb = ctx.adb()?;
            let output = adb
                .shell_args(&serial, &argv, Some(timeout), Some(MAX_SHELL_OUTPUT))
                .await?;
            let redactor = ctx.redactor();
            let stdout = redactor.redact(&output.stdout_text());
            let stderr = redactor.redact(&output.stderr_text());
            let truncated = output.stdout_truncated || output.stderr_truncated;
            let result = json!({
                "read_only": read_only,
                "argv": argv,
                "allow": effective_allow,
                "deny": deny,
                "timeout_secs": timeout.as_secs(),
                "output_limit_bytes": MAX_SHELL_OUTPUT,
                "output_truncated": truncated,
                "timed_out": output.timed_out,
                "exit_code": output.exit_code,
                "redaction_enabled": redactor.is_enabled(),
                "stdout": stdout,
                "stderr": stderr,
                "classification": if read_only { "read-only" } else { "mutating" },
            });
            let human = format!(
                "Shell {} on {} (argv {:?}, truncated={}, redaction={}):\n{}\n",
                if read_only { "read-only" } else { "mutating" },
                serial,
                argv,
                truncated,
                redactor.is_enabled(),
                result["stdout"].as_str().unwrap_or_default()
            );
            let _ = run;
            Ok(success("devices.shell", result, human)
                .with_session(session.id.clone())
                .with_device(serial.clone()))
        },
    )
    .await
}

pub async fn input(ctx: &CliContext, command: &DeviceInputCommand) -> Result<CommandSuccess> {
    match command {
        DeviceInputCommand::Tap(args) => input_tap(ctx, args).await,
    }
}

async fn input_tap(ctx: &CliContext, args: &DeviceInputTapArgs) -> Result<CommandSuccess> {
    if args.x < 0 || args.y < 0 {
        return Err(ForgeError::invalid_argument(
            "devices input tap coordinates must be non-negative device pixels",
        )
        .with_suggestion("Use real device pixels in the current display orientation."));
    }
    if ctx.dry_run {
        return Ok(success(
            "devices.input.tap",
            json!({
                "dry_run": true,
                "x": args.x,
                "y": args.y,
                "via": "UiManager::tap_xy",
                "risk": "Risk::Input",
                "would_execute": true,
                "note": "No device was resolved and no input was injected.",
            }),
            format!(
                "Would tap device pixel ({}, {}) via UiManager::tap_xy. No device was contacted.\n",
                args.x, args.y
            ),
        ));
    }
    let session = ctx.require_session()?;
    let device = ctx.resolve_device(Some(&session)).await?;
    ctx.ensure_risk_allowed(&device, Risk::Input, "injecting input")?;
    let serial = device.serial.clone();
    let x = args.x;
    let y = args.y;
    execute_in_run!(
        ctx,
        "devices.input.tap",
        Some("devices input tap"),
        Some(&session),
        Some(&device),
        true,
        |run| async {
            let ui = ctx.ui_manager()?;
            let display = ui.tap_xy(&serial, x, y).await?;
            let display_size =
                display.map(|(width, height)| json!({"width": width, "height": height}));
            let receipt = json!({
                "action": "tap",
                "x": x,
                "y": y,
                "via": "UiManager::tap_xy",
                "risk": "Risk::Input",
                "dispatched": true,
                "display_size": display_size,
            });
            let result = json!({
                "x": x,
                "y": y,
                "display_size": display_size,
                "receipt": receipt,
            });
            let human = format!(
                "Tapped device pixel ({x}, {y}) on {serial}; display {:?}.\n",
                display_size
            );
            let _ = run;
            Ok(success("devices.input.tap", result, human)
                .with_session(session.id.clone())
                .with_device(serial.clone()))
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn default_allowlist_covers_read_only_diagnostics() {
        for command in [
            vec!["dumpsys", "gfxinfo", "com.example"],
            vec!["getprop", "ro.build.version.release"],
            vec!["settings", "get", "secure", "location_mode"],
            vec!["pm", "list", "packages"],
            vec!["pm", "path", "com.example"],
            vec!["cmd", "package", "list", "packages"],
            vec!["cmd", "package", "path", "com.example"],
            vec!["ls", "-l", "/sdcard"],
            vec!["stat", "/sdcard"],
            vec!["df", "-h"],
            vec!["du", "-sh", "/sdcard"],
            vec!["date"],
            vec!["date", "+%s"],
            vec!["date", "-u"],
            vec!["date", "-u", "+%s"],
            vec!["wm", "size"],
            vec!["wm", "density"],
            vec!["logcat", "-d", "-t", "50"],
            vec!["id"],
            vec!["whoami"],
            vec!["uname", "-a"],
        ] {
            assert!(
                validate_read_only(&argv(&command), &[], &[]).is_ok(),
                "default allowlist refused {command:?}"
            );
        }
        for command in [
            vec!["dumpsys", "gfxinfo", "com.example", "reset"],
            vec!["settings", "put", "secure", "location_mode", "0"],
            vec!["pm", "clear", "com.example"],
            vec!["input", "tap", "10", "20"],
            vec!["reboot"],
            vec!["cat", "/etc/passwd"],
            vec!["sh", "-c", "id"],
        ] {
            assert!(
                validate_read_only(&argv(&command), &[], &[]).is_err(),
                "default allowlist accepted {command:?}"
            );
        }
    }

    #[test]
    fn mutating_date_and_wm_arguments_are_refused() {
        // A bare prefix match is not enough: each of these mutates the device.
        for command in [
            vec!["date", "010100002026"],
            vec!["date", "-s", "010100002026"],
            vec!["date", "--set", "010100002026"],
            vec!["date", "+%s", "010100002026"],
            vec!["wm", "size", "1080x1920"],
            vec!["wm", "density", "420"],
            vec!["wm", "user-rotation", "1"],
            vec!["logcat", "-d", "-c"],
            vec!["logcat", "-d", "-f", "/sdcard/log.txt"],
        ] {
            let error = validate_read_only(&argv(&command), &[], &[]).unwrap_err();
            assert_eq!(
                error.code(),
                androidforge_core::error::ErrorCode::InvalidArgument
            );
            assert!(
                error.message().contains("read-only"),
                "{command:?}: {}",
                error.message()
            );
        }
        for command in [
            vec!["date"],
            vec!["date", "+%s"],
            vec!["date", "-u"],
            vec!["date", "--utc", "+%Y-%m-%d"],
            vec!["wm", "size"],
            vec!["wm", "density"],
        ] {
            assert!(
                validate_read_only(&argv(&command), &[], &[]).is_ok(),
                "read form refused: {command:?}"
            );
        }
    }

    #[test]
    fn deny_and_custom_allow_are_enforced() {
        let command = argv(&["dumpsys", "gfxinfo", "com.example"]);
        assert!(
            validate_read_only(&command, &[], &["dumpsys".to_string()]).is_err(),
            "--deny was ignored"
        );
        assert!(
            validate_read_only(&command, &["getprop".to_string()], &[]).is_err(),
            "custom allow was ignored"
        );
        assert!(
            validate_read_only(&command, &["dumpsys".to_string()], &[]).is_ok(),
            "custom allow refused a matching prefix"
        );
        assert!(validate_read_only(&[], &[], &[]).is_err());
    }
}
