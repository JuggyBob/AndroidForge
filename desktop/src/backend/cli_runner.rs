use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::json_parser::{error_from_envelope, parse_envelope};
use super::process_manager::kill_process_tree;
use super::{DesktopError, Result};
use crate::models::Envelope;

#[derive(Debug, Clone)]
pub struct Invocation {
    pub args: Vec<String>,
    pub project: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub timeout: Duration,
}

impl Invocation {
    pub fn new(args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            args: args.into_iter().map(Into::into).collect(),
            project: None,
            cwd: None,
            timeout: Duration::from_secs(60),
        }
    }

    pub fn project(mut self, project: Option<PathBuf>) -> Self {
        self.project = project;
        self
    }

    pub fn cwd(mut self, cwd: Option<PathBuf>) -> Self {
        self.cwd = cwd;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[derive(Debug, Clone)]
pub struct CliOutput {
    pub envelope: Envelope,
    pub raw_stdout: String,
    pub stderr: String,
    pub duration: Duration,
}

impl CliOutput {
    pub fn require_success(self) -> Result<Envelope> {
        if self.envelope.status == "success" {
            Ok(self.envelope)
        } else {
            Err(error_from_envelope(&self.raw_stdout, &self.envelope))
        }
    }
}

#[derive(Debug, Clone)]
pub struct CliRunner {
    pub cli_path: PathBuf,
}

impl CliRunner {
    pub fn new(cli_path: impl Into<PathBuf>) -> Self {
        Self {
            cli_path: cli_path.into(),
        }
    }

    pub fn command(&self, invocation: &Invocation) -> Command {
        let mut command = Command::new(&self.cli_path);
        command.arg("--json");
        if let Some(project) = &invocation.project {
            command.arg("--project").arg(project);
        }
        if let Some(cwd) = &invocation.cwd {
            command.current_dir(cwd);
        }
        command.args(&invocation.args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        hide_console(&mut command);
        command
    }

    pub fn run(&self, invocation: &Invocation) -> Result<CliOutput> {
        let started = Instant::now();
        let mut command = self.command(invocation);
        let mut child = command.spawn().map_err(|source| DesktopError::CliSpawn {
            path: self.cli_path.clone(),
            source,
        })?;
        let pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let stdout_reader = std::thread::spawn(move || read_pipe(stdout));
        let stderr_reader = std::thread::spawn(move || read_pipe(stderr));

        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() >= invocation.timeout {
                kill_process_tree(pid);
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(DesktopError::CliTimeout {
                    seconds: invocation.timeout.as_secs(),
                });
            }
            std::thread::sleep(Duration::from_millis(20));
        };

        let raw_stdout = stdout_reader.join().unwrap_or_default();
        let stderr = stderr_reader.join().unwrap_or_default();
        let duration = started.elapsed();

        let envelope = match parse_envelope(&raw_stdout, &stderr) {
            Ok(envelope) => envelope,
            Err(DesktopError::InvalidJson { .. }) if !status.success() => {
                return Err(DesktopError::CliExit {
                    code: status.code().unwrap_or(-1),
                    stderr: if stderr.trim().is_empty() {
                        raw_stdout.clone()
                    } else {
                        stderr.clone()
                    },
                });
            }
            Err(error) => return Err(error),
        };

        Ok(CliOutput {
            envelope,
            raw_stdout,
            stderr,
            duration,
        })
    }

    pub fn run_success(&self, invocation: &Invocation) -> Result<CliOutput> {
        let output = self.run(invocation)?;
        if output.envelope.status != "success" {
            return Err(error_from_envelope(&output.raw_stdout, &output.envelope));
        }
        Ok(output)
    }

    pub fn version(&self) -> Result<String> {
        let mut command = Command::new(&self.cli_path);
        command.arg("--version");
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        hide_console(&mut command);
        let output = command.output().map_err(|source| DesktopError::CliSpawn {
            path: self.cli_path.clone(),
            source,
        })?;
        if !output.status.success() {
            return Err(DesktopError::CliExit {
                code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let text = text.trim();
        Ok(text
            .strip_prefix("androidforge ")
            .unwrap_or(text)
            .trim()
            .to_string())
    }
}

fn read_pipe<R: Read>(pipe: Option<R>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut bytes = Vec::new();
    let _ = pipe.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

pub fn project_args(project: Option<&Path>) -> Vec<String> {
    project
        .map(|path| vec!["--project".to_string(), path.display().to_string()])
        .unwrap_or_default()
}
