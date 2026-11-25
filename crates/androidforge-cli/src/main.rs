mod cli;
mod compact;
#[macro_use]
mod run_scope;
mod commands;
mod context;
mod junit;

use std::ffi::OsString;
use std::time::Instant;

use androidforge_core::error::{ErrorCode, ForgeError};
use androidforge_core::output::{OutputOptions, emit_error, emit_success, write_stats};
use clap::Parser;
use clap::error::ErrorKind;
use commands::canonical_name;

fn main() {
    // The orchestration future nests process capture, UI waits, fixture steps
    // and diagnostic capture. Give it a generous stack instead of relying on
    // the Windows main-thread default, which is too small for deeply nested
    // async state machines.
    let code = std::thread::Builder::new()
        .name("androidforge-main".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            runtime.block_on(run())
        })
        .map(|handle| handle.join().unwrap_or(101))
        .unwrap_or(101);
    std::process::exit(code);
}

async fn run() -> i32 {
    let raw_args: Vec<OsString> = std::env::args_os().collect();
    let json_requested = raw_args
        .iter()
        .any(|arg| arg == "--json" || arg == "--jsonl");
    let jsonl_requested = raw_args.iter().any(|arg| arg == "--jsonl");
    let stats_requested = raw_args.iter().any(|arg| arg == "--stats");
    let args = preprocess(raw_args);
    let started = Instant::now();
    let mut cli = match cli::Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) => {
            let kind = error.kind();
            if matches!(kind, ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                print!("{error}");
                return 0;
            }
            let command = "androidforge";
            let message = first_line(&error.to_string());
            let code = if matches!(
                kind,
                ErrorKind::InvalidSubcommand
                    | ErrorKind::UnknownArgument
                    | ErrorKind::InvalidValue
                    | ErrorKind::InvalidUtf8
            ) {
                ErrorCode::InvalidCommand
            } else {
                ErrorCode::InvalidArgument
            };
            let mut forge = ForgeError::new(code, message).command(command);
            for line in error.to_string().lines() {
                let line = line.trim();
                if line.to_ascii_lowercase().contains("tip:") {
                    forge = forge.with_suggestion(line.trim_start_matches("tip: ").to_string());
                }
            }
            forge = forge
                .with_suggestion("Run `androidforge --help` or `androidforge <command> --help`.");
            if json_requested {
                let mut options = OutputOptions::new(true, false);
                if jsonl_requested {
                    options = options.with_jsonl();
                }
                if stats_requested {
                    options = options.with_stats();
                }
                let _ = emit_error(
                    options,
                    command,
                    &forge,
                    started.elapsed().as_millis() as u64,
                    None,
                    None,
                    None,
                );
            } else {
                eprintln!("{error}");
                if stats_requested {
                    write_stats(0, 0);
                }
            }
            return forge.exit_code();
        }
    };
    if matches!(
        &cli.command,
        cli::Command::Mcp {
            command: cli::McpCommand::Serve
        }
    ) {
        return match commands::mcp::serve_stdio() {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("[androidforge] mcp: {}", error.message());
                error.exit_code()
            }
        };
    }
    if let cli::Command::Diagnostics {
        command:
            cli::DiagnosticsCommand::Logcat {
                command: cli::LogcatCommand::Watch(args),
            },
    } = &cli.command
    {
        return match commands::diagnostics::stream_logcat_watch(&cli, args).await {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("[androidforge] logcat watch: {}", error.message());
                error.exit_code()
            }
        };
    }
    if let cli::Command::Run {
        command: cli::RunCommand::Events(args),
    } = &cli.command
    {
        return match commands::report::stream_events(&cli, args).await {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("[androidforge] run events: {}", error.message());
                error.exit_code()
            }
        };
    }
    // `--jsonl` implies `--json` for every downstream command, not just for
    // the final envelope writer.
    if cli.jsonl {
        cli.json = true;
    }
    let command_name = canonical_name(&cli.command);
    let plan = compact::resolve(&cli);
    let mut output = OutputOptions::new(cli.json || cli.jsonl, cli.quiet);
    if cli.jsonl {
        output = output.with_jsonl();
    }
    if cli.stats {
        output = output.with_stats();
    }
    let requested_fields = cli.fields.as_ref().map(|fields| {
        fields
            .split(',')
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    if plan.is_compact() {
        output = output.with_profile(plan.profile.as_str());
    }
    if let Some(keys) = requested_fields {
        if keys.is_empty() {
            // Fail closed before dispatch: an empty keep-set would otherwise
            // trim every successful result to `{}` and hide command output.
            let error = ForgeError::new(
                ErrorCode::InvalidArgument,
                "--fields requires at least one result key",
            )
            .command(command_name.clone())
            .with_suggestion("--fields result-key[,result-key...]".to_string())
            .with_suggestion(
                "Use --output-profile compact|brief to trim the envelope instead of result keys."
                    .to_string(),
            );
            let _ = emit_error(
                output,
                &command_name,
                &error,
                started.elapsed().as_millis() as u64,
                None,
                None,
                None,
            );
            return error.exit_code();
        }
        output = output.with_fields(keys);
    }
    match commands::execute(&cli).await {
        Ok(mut success) => {
            if success.duration_ms == 0 {
                success.duration_ms = started.elapsed().as_millis() as u64;
            }
            if cli.run.is_some() && success.run_id.is_none() {
                eprintln!(
                    "[androidforge] warning: --run was provided but {} did not append a step",
                    success.command
                );
            }
            compact::apply_success(&mut success, plan);
            let exit_code = success.exit_code;
            if let Err(error) = emit_success(output.clone(), &success) {
                let _ = emit_error(
                    output,
                    &command_name,
                    &error,
                    started.elapsed().as_millis() as u64,
                    success.session_id.clone(),
                    success.device_serial.clone(),
                    success.run_id.clone(),
                );
                return error.exit_code();
            }
            exit_code
        }
        Err(mut error) => {
            let used_run = error.body().context.contains_key("run_id");
            if cli.run.is_some() && !used_run {
                eprintln!(
                    "[androidforge] warning: --run was provided but {} did not append a step",
                    command_name
                );
            }
            compact::apply_error(&mut error, plan);
            let duration = started.elapsed().as_millis() as u64;
            let session = error.body().session_id.clone();
            let device = error.body().device_serial.clone();
            let run = error
                .body()
                .context
                .get("run_id")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            if let Err(write_error) = emit_error(
                output,
                &command_name,
                &error,
                duration,
                session,
                device,
                run,
            ) {
                eprintln!("[androidforge] could not emit structured error: {write_error}");
            }
            error.exit_code()
        }
    }
}

fn first_line(value: &str) -> String {
    value
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(value)
        .trim()
        .to_string()
}

fn preprocess(mut args: Vec<OsString>) -> Vec<OsString> {
    if args.is_empty() {
        return args;
    }
    let Some(index) = top_level_command_index(&args) else {
        return args;
    };
    let top = args[index].to_string_lossy().to_string();
    if top == "adb"
        && args
            .last()
            .map(|arg| arg.to_string_lossy() == "--json")
            .unwrap_or(false)
    {
        // Agents commonly append the global flag. Raw `adb` is a trailing
        // passthrough, so move only a final --json before the command.
        args.pop();
        args.insert(index, OsString::from("--json"));
        return args;
    }
    if top == "run" {
        let next = args
            .get(index + 1)
            .map(|arg| arg.to_string_lossy().to_string());
        let is_subcommand = matches!(
            next.as_deref(),
            Some(
                "start"
                    | "end"
                    | "status"
                    | "exec"
                    | "checkpoint"
                    | "resume"
                    | "events"
                    | "--help"
                    | "-h"
            )
        );
        if !is_subcommand {
            args.insert(index + 1, OsString::from("exec"));
        }
    } else if top == "diagnostics"
        && let Some(logcat_index) = args
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, arg)| arg.to_string_lossy() == "logcat")
            .map(|(index, _)| index)
    {
        let next = args
            .get(logcat_index + 1)
            .map(|arg| arg.to_string_lossy().to_string());
        let is_subcommand = matches!(
            next.as_deref(),
            Some(
                "tail" | "peek" | "query" | "watch" | "start" | "stop" | "status" | "--help" | "-h"
            )
        );
        if !is_subcommand {
            args.insert(logcat_index + 1, OsString::from("tail"));
        }
    }
    args
}

fn top_level_command_index(args: &[OsString]) -> Option<usize> {
    let mut index = 1;
    while index < args.len() {
        let arg = args[index].to_string_lossy();
        if arg == "--json"
            || arg == "--jsonl"
            || arg == "--stats"
            || arg == "--trace"
            || arg == "--dry-run"
            || arg == "--yes"
            || arg == "--allow-physical-mutation"
            || arg == "--quiet"
            || arg == "--no-failure-capture"
            || arg == "--brief"
            || arg == "--compact"
            || arg.starts_with("--json=")
            || arg.starts_with("--jsonl=")
            || arg.starts_with("--stats=")
            || arg.starts_with("--trace=")
            || arg.starts_with("--dry-run=")
            || arg.starts_with("--yes=")
        {
            index += 1;
            continue;
        }
        if matches!(
            arg.as_ref(),
            "--session"
                | "--serial"
                | "--project"
                | "--run"
                | "--output-profile"
                | "--max-items"
                | "--max-text-chars"
                | "--fields"
        ) {
            index += 2;
            continue;
        }
        if arg.starts_with("--session=")
            || arg.starts_with("--serial=")
            || arg.starts_with("--project=")
            || arg.starts_with("--run=")
            || arg.starts_with("--output-profile=")
            || arg.starts_with("--max-items=")
            || arg.starts_with("--max-text-chars=")
            || arg.starts_with("--fields=")
        {
            index += 1;
            continue;
        }
        if arg.starts_with('-') {
            index += 1;
            continue;
        }
        return Some(index);
    }
    None
}
