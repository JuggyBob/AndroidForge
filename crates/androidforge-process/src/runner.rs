use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::encoding::decode_lossless;
use crate::process_util::kill_tree;
use crate::shell::shell_quote_join;

const DEFAULT_MAX_CAPTURE: usize = 8 * 1024 * 1024;
const READER_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, Option<String>)>,
    pub timeout: Duration,
    pub max_stdout: usize,
    pub max_stderr: usize,
    pub stdin: Option<Vec<u8>>,
    pub trace: bool,
    pub hide_window: bool,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            timeout: Duration::from_secs(30),
            max_stdout: DEFAULT_MAX_CAPTURE,
            max_stderr: DEFAULT_MAX_CAPTURE,
            stdin: None,
            trace: false,
            hide_window: true,
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), Some(value.into())));
        self
    }

    pub fn env_remove(mut self, key: impl Into<String>) -> Self {
        self.env.push((key.into(), None));
        self
    }

    pub fn stdin(mut self, stdin: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(stdin.into());
        self
    }

    pub fn trace(mut self, trace: bool) -> Self {
        self.trace = trace;
        self
    }

    pub fn hide_window(mut self, hide: bool) -> Self {
        self.hide_window = hide;
        self
    }

    pub fn max_stdout(mut self, bytes: usize) -> Self {
        self.max_stdout = bytes;
        self
    }

    pub fn max_stderr(mut self, bytes: usize) -> Self {
        self.max_stderr = bytes;
        self
    }
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub program: String,
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub duration_ms: u64,
    pub timed_out: bool,
}

impl CommandOutput {
    pub fn stdout_text(&self) -> String {
        decode_lossless(&self.stdout)
    }

    pub fn stderr_text(&self) -> String {
        decode_lossless(&self.stderr)
    }

    pub fn combined_text(&self) -> String {
        let mut out = self.stdout_text();
        let err = self.stderr_text();
        if !err.is_empty() {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&err);
        }
        out
    }

    pub fn command_line(&self) -> String {
        let mut parts = Vec::with_capacity(self.args.len() + 1);
        parts.push(crate::shell::shell_quote(&self.program));
        parts.push(shell_quote_join(&self.args));
        parts
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn require_success(self) -> ProcessResult<Self> {
        if self.timed_out {
            return Err(ProcessError::Timeout {
                program: self.program,
                args: self.args,
                timeout_ms: self.duration_ms,
                stdout: self.stdout,
                stderr: self.stderr,
            });
        }
        if !self.success {
            return Err(ProcessError::Failed {
                program: self.program,
                args: self.args,
                exit_code: self.exit_code,
                stdout: self.stdout,
                stderr: self.stderr,
            });
        }
        Ok(self)
    }
}

pub type ProcessResult<T> = Result<T, ProcessError>;

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("failed to start {program}: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("{program} timed out after {timeout_ms} ms")]
    Timeout {
        program: String,
        args: Vec<String>,
        timeout_ms: u64,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },

    #[error("{program} exited with {code}", code = exit_code.map(|c| format!("code {c}")).unwrap_or_else(|| "no exit code".to_string()))]
    Failed {
        program: String,
        args: Vec<String>,
        exit_code: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },

    #[error("I/O error while running {program}: {source}")]
    Io {
        program: String,
        #[source]
        source: std::io::Error,
    },
}

impl ProcessError {
    pub fn stdout_text(&self) -> String {
        match self {
            Self::Timeout { stdout, .. } | Self::Failed { stdout, .. } => decode_lossless(stdout),
            Self::Spawn { .. } | Self::Io { .. } => String::new(),
        }
    }

    pub fn stderr_text(&self) -> String {
        match self {
            Self::Timeout { stderr, .. } | Self::Failed { stderr, .. } => decode_lossless(stderr),
            Self::Spawn { .. } | Self::Io { .. } => String::new(),
        }
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self {
            Self::Failed { exit_code, .. } => *exit_code,
            _ => None,
        }
    }

    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout { .. })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Runner {
    trace: bool,
}

impl Runner {
    pub fn new(trace: bool) -> Self {
        Self { trace }
    }

    pub fn with_trace(&self, trace: bool) -> Self {
        Self { trace }
    }

    pub async fn run(&self, spec: CommandSpec) -> ProcessResult<CommandOutput> {
        let trace = self.trace || spec.trace;
        let started = Instant::now();
        let command_line = display_command(&spec.program, &spec.args);

        if trace {
            eprintln!(
                "[trace] {} (timeout={}ms{})",
                command_line,
                spec.timeout.as_millis(),
                spec.cwd
                    .as_ref()
                    .map(|p| format!(", cwd={}", p.display()))
                    .unwrap_or_default()
            );
        }

        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .stdin(if spec.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &spec.env {
            match value {
                Some(value) => {
                    command.env(key, value);
                }
                None => {
                    command.env_remove(key);
                }
            }
        }
        apply_creation_flags(&mut command, spec.hide_window);

        let mut child = command.spawn().map_err(|source| ProcessError::Spawn {
            program: spec.program.clone(),
            source,
        })?;
        let pid = child.id().unwrap_or(0);

        let stdout_task = child
            .stdout
            .take()
            .map(|stdout| tokio::spawn(drain_stream(stdout, spec.max_stdout)));
        let stderr_task = child
            .stderr
            .take()
            .map(|stderr| tokio::spawn(drain_stream(stderr, spec.max_stderr)));
        let stdin_task = if let Some(input) = spec.stdin.clone() {
            let mut stdin = child.stdin.take();
            Some(tokio::spawn(async move {
                if let Some(stdin) = stdin.as_mut() {
                    // A broken pipe simply means the child exited without
                    // consuming all of stdin; the exit status is authoritative.
                    let _ = stdin.write_all(&input).await;
                    let _ = stdin.shutdown().await;
                }
            }))
        } else {
            None
        };

        let wait_result = tokio::time::timeout(spec.timeout, child.wait()).await;
        let mut timed_out = false;
        let status = match wait_result {
            Ok(result) => match result {
                Ok(status) => Some(status),
                Err(source) => {
                    if trace {
                        eprintln!("[trace] {} wait error: {}", command_line, source);
                    }
                    None
                }
            },
            Err(_) => {
                timed_out = true;
                if pid != 0 {
                    kill_tree(pid);
                }
                let _ = child.start_kill();
                match tokio::time::timeout(READER_GRACE, child.wait()).await {
                    Ok(_) => {}
                    Err(_) => {
                        if trace {
                            eprintln!("[trace] {} did not exit after timeout kill", command_line);
                        }
                    }
                }
                None
            }
        };

        if let Some(task) = stdin_task {
            let _ = tokio::time::timeout(Duration::from_secs(1), task).await;
        }
        let (stdout, stdout_truncated) = join_reader(stdout_task).await;
        let (stderr, stderr_truncated) = join_reader(stderr_task).await;

        let duration_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let exit_code = status.and_then(|status| status.code());
        let success = status.map(|status| status.success()).unwrap_or(false) && !timed_out;

        if trace {
            eprintln!(
                "[trace] {} -> exit={} duration={}ms stdout={}B{} stderr={}B{}",
                command_line,
                exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "timeout".to_string()),
                duration_ms,
                stdout.len(),
                if stdout_truncated { " (truncated)" } else { "" },
                stderr.len(),
                if stderr_truncated { " (truncated)" } else { "" }
            );
        }

        Ok(CommandOutput {
            program: spec.program,
            args: spec.args,
            exit_code,
            success,
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
            duration_ms,
            timed_out,
        })
    }
}

async fn drain_stream<R: AsyncRead + Unpin>(mut stream: R, max: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::with_capacity(max.min(64 * 1024));
    let mut total = 0usize;
    let mut buf = [0u8; 16 * 1024];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => break,
            Ok(read) => {
                total = total.saturating_add(read);
                if kept.len() < max {
                    let take = read.min(max - kept.len());
                    kept.extend_from_slice(&buf[..take]);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    (kept, total > max)
}

async fn join_reader(task: Option<tokio::task::JoinHandle<(Vec<u8>, bool)>>) -> (Vec<u8>, bool) {
    match task {
        Some(task) => match tokio::time::timeout(READER_GRACE, task).await {
            Ok(Ok(result)) => result,
            _ => (Vec::new(), false),
        },
        None => (Vec::new(), false),
    }
}

fn display_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        crate::shell::shell_quote(program)
    } else {
        format!(
            "{} {}",
            crate::shell::shell_quote(program),
            shell_quote_join(args)
        )
    }
}

#[cfg(windows)]
fn apply_creation_flags(command: &mut Command, hide_window: bool) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut flags = CREATE_NEW_PROCESS_GROUP;
    if hide_window {
        flags |= CREATE_NO_WINDOW;
    }
    command.as_std_mut().creation_flags(flags);
}

#[cfg(not(windows))]
fn apply_creation_flags(_command: &mut Command, _hide_window: bool) {}
