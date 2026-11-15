use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::runner::ProcessError;
use crate::shell::shell_quote_join;

/// Specification for a managed-but-detached child process.
///
/// AndroidForge is not a daemon. Some operations (emulator, logcat capture,
/// screenrecord) must outlive one CLI invocation; those are persistent managed
/// subprocesses with metadata and explicit stop commands.
#[derive(Debug, Clone)]
pub struct DetachedSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, Option<String>)>,
    pub stdout_path: Option<PathBuf>,
    pub stderr_path: Option<PathBuf>,
    pub hide_window: bool,
    pub trace: bool,
}

impl DetachedSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            stdout_path: None,
            stderr_path: None,
            hide_window: true,
            trace: false,
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

    pub fn stdout_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.stdout_path = Some(path.into());
        self
    }

    pub fn stderr_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.stderr_path = Some(path.into());
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

    pub fn hide_window(mut self, hide: bool) -> Self {
        self.hide_window = hide;
        self
    }

    pub fn trace(mut self, trace: bool) -> Self {
        self.trace = trace;
        self
    }
}

#[derive(Debug, Clone)]
pub struct DetachedChild {
    pub pid: u32,
    pub program: String,
    pub args: Vec<String>,
}

pub fn spawn_detached(spec: &DetachedSpec) -> Result<DetachedChild, ProcessError> {
    if spec.trace {
        eprintln!(
            "[trace] spawn detached: {} {}",
            spec.program,
            shell_quote_join(&spec.args)
        );
    }

    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .stdin(Stdio::null())
        .stdout(open_stdio(spec.stdout_path.as_deref()))
        .stderr(open_stdio(spec.stderr_path.as_deref()));
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

    // Even with DETACHED_PROCESS, bInheritHandles=TRUE can leak the caller's
    // stdout/stderr pipe into the detached process tree. Clear the inherit bit
    // around CreateProcess so a captured `session start` can return while the
    // emulator keeps running.
    let saved_handles = clear_inheritable_std_handles();
    let spawn_result = command.spawn();
    restore_std_handles(saved_handles);
    let child = spawn_result.map_err(|source| ProcessError::Spawn {
        program: spec.program.clone(),
        source,
    })?;
    let pid = child.id();
    // Dropping a std::process::Child deliberately does not wait or terminate it.
    drop(child);
    Ok(DetachedChild {
        pid,
        program: spec.program.clone(),
        args: spec.args.clone(),
    })
}

fn open_stdio(path: Option<&Path>) -> Stdio {
    match path {
        Some(path) => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match OpenOptions::new().create(true).append(true).open(path) {
                Ok(file) => Stdio::from(file),
                Err(_) => Stdio::null(),
            }
        }
        None => Stdio::null(),
    }
}

#[allow(dead_code)]
fn _assert_file_send(_: File) {}

#[cfg(windows)]
fn apply_creation_flags(command: &mut Command, hide_window: bool) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // DETACHED_PROCESS is essential for the emulator: its qemu child otherwise
    // inherits the caller's stdout/stderr handles, so a captured `session start`
    // would not return until the emulator exits. The explicit log files are the
    // only output we want from these managed children.
    let mut flags = CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS;
    if hide_window {
        flags |= CREATE_NO_WINDOW;
    }
    command.creation_flags(flags);
}

#[cfg(not(windows))]
fn apply_creation_flags(_command: &mut Command, _hide_window: bool) {}

#[cfg(windows)]
fn clear_inheritable_std_handles() -> Vec<(isize, u32)> {
    // STD_INPUT_HANDLE/-10, STD_OUTPUT_HANDLE/-11, STD_ERROR_HANDLE/-12.
    let mut saved = Vec::new();
    for std_handle in [(-10i32) as u32, (-11i32) as u32, (-12i32) as u32] {
        unsafe {
            let handle = GetStdHandle(std_handle);
            if handle == 0 || handle == -1 {
                continue;
            }
            let mut flags = 0u32;
            if GetHandleInformation(handle, &mut flags) != 0
                && flags & HANDLE_FLAG_INHERIT != 0
                && SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) != 0
            {
                saved.push((handle, flags));
            }
        }
    }
    saved
}

#[cfg(not(windows))]
fn clear_inheritable_std_handles() -> Vec<(isize, u32)> {
    Vec::new()
}

#[cfg(windows)]
fn restore_std_handles(saved: Vec<(isize, u32)>) {
    for (handle, flags) in saved {
        unsafe {
            let _ = SetHandleInformation(handle, HANDLE_FLAG_INHERIT, flags & HANDLE_FLAG_INHERIT);
        }
    }
}

#[cfg(not(windows))]
fn restore_std_handles(_saved: Vec<(isize, u32)>) {}

#[cfg(windows)]
const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;

#[cfg(windows)]
unsafe extern "system" {
    fn GetStdHandle(n_std_handle: u32) -> isize;
    fn GetHandleInformation(object: isize, flags: *mut u32) -> i32;
    fn SetHandleInformation(object: isize, mask: u32, flags: u32) -> i32;
}
