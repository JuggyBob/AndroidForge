use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use super::cli_runner::{CliRunner, Invocation};
use super::json_parser::parse_envelope;
use crate::models::Envelope;

const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub enum OperationEvent {
    Stdout(String),
    Stderr(String),
    Finished(OperationOutcome),
}

#[derive(Debug, Clone)]
pub struct OperationOutcome {
    pub title: String,
    pub command: String,
    pub success: bool,
    pub cancelled: bool,
    pub status_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
    pub envelope: Option<Envelope>,
    pub parse_error: Option<String>,
    pub truncated: bool,
}

pub struct OperationHandle {
    pub id: u64,
    pub title: String,
    pub command: String,
    pub pid: u32,
    pub started_at: Instant,
    pub cancel: Arc<AtomicBool>,
    pub receiver: mpsc::Receiver<OperationEvent>,
}

impl OperationHandle {
    pub fn cancel(&self) -> bool {
        self.cancel.store(true, Ordering::SeqCst);
        kill_process_tree(self.pid)
    }

    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }
}

pub fn spawn_operation(
    runner: &CliRunner,
    invocation: &Invocation,
    id: u64,
    title: impl Into<String>,
) -> std::io::Result<OperationHandle> {
    let title = title.into();
    let command = invocation
        .args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    let mut command_builder = runner.command(invocation);
    let mut child = command_builder.spawn()?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_thread = Arc::clone(&cancel);
    let started = Instant::now();

    let out_sender = sender.clone();
    let out_thread = std::thread::spawn(move || read_stream(stdout, true, out_sender));
    let err_sender = sender.clone();
    let err_thread = std::thread::spawn(move || read_stream(stderr, false, err_sender));

    let title_for_thread = title.clone();
    let command_for_thread = command.clone();
    std::thread::spawn(move || {
        let status = wait_child(&mut child);
        let (stdout, stdout_truncated) = out_thread.join().unwrap_or_default();
        let (stderr, stderr_truncated) = err_thread.join().unwrap_or_default();
        let duration_ms = started.elapsed().as_millis() as u64;
        let cancelled = cancel_thread.load(Ordering::SeqCst);
        let envelope = parse_envelope(&stdout, &stderr).ok();
        let parse_error = if envelope.is_none() {
            Some(if stdout.trim().is_empty() {
                "CLI produced no JSON output".to_string()
            } else {
                "CLI output was not a valid AndroidForge JSON envelope".to_string()
            })
        } else {
            None
        };
        let success = status.is_some_and(|status| status.success())
            && envelope
                .as_ref()
                .is_some_and(|envelope| envelope.status == "success")
            && !cancelled;
        let _ = sender.send(OperationEvent::Finished(OperationOutcome {
            title: title_for_thread,
            command: command_for_thread,
            success,
            cancelled,
            status_code: status.and_then(|status| status.code()),
            duration_ms,
            stdout,
            stderr,
            envelope,
            parse_error,
            truncated: stdout_truncated || stderr_truncated,
        }));
    });

    Ok(OperationHandle {
        id,
        title,
        command,
        pid,
        started_at: started,
        cancel,
        receiver,
    })
}

fn wait_child(child: &mut Child) -> Option<std::process::ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return None,
        }
    }
}

fn read_stream<R: Read>(
    stream: Option<R>,
    is_stdout: bool,
    sender: mpsc::Sender<OperationEvent>,
) -> (String, bool) {
    let Some(stream) = stream else {
        return (String::new(), false);
    };
    let mut reader = BufReader::new(stream);
    let mut captured = String::new();
    let mut truncated = false;
    let mut buffer = String::new();
    loop {
        buffer.clear();
        match reader.read_line(&mut buffer) {
            Ok(0) => break,
            Ok(_) => {
                if captured.len() < MAX_CAPTURE_BYTES {
                    let remaining = MAX_CAPTURE_BYTES - captured.len();
                    if buffer.len() <= remaining {
                        captured.push_str(&buffer);
                    } else {
                        captured.push_str(&buffer[..remaining]);
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
                let line = buffer.trim_end_matches(['\r', '\n']).to_string();
                let event = if is_stdout {
                    OperationEvent::Stdout(line)
                } else {
                    OperationEvent::Stderr(line)
                };
                if sender.send(event).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    (captured, truncated)
}

#[cfg(windows)]
pub fn kill_process_tree(pid: u32) -> bool {
    if terminate_native_tree(pid) {
        return true;
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn terminate_native_tree(root: u32) -> bool {
    use std::collections::{HashMap, HashSet};
    use std::mem::size_of;

    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
        return false;
    }
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut found_root = false;
    if unsafe { Process32FirstW(snapshot, &mut entry) } != 0 {
        loop {
            if entry.th32ProcessID == root {
                found_root = true;
            }
            if entry.th32ProcessID != 0 {
                children
                    .entry(entry.th32ParentProcessID)
                    .or_default()
                    .push(entry.th32ProcessID);
            }
            if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
                break;
            }
        }
    }
    unsafe {
        CloseHandle(snapshot);
    }
    if !found_root {
        return false;
    }

    fn collect(
        parent: u32,
        children: &std::collections::HashMap<u32, Vec<u32>>,
        seen: &mut HashSet<u32>,
        order: &mut Vec<u32>,
    ) {
        if !seen.insert(parent) {
            return;
        }
        order.push(parent);
        if let Some(ids) = children.get(&parent) {
            for id in ids {
                collect(*id, children, seen, order);
            }
        }
    }

    let mut order = Vec::new();
    collect(root, &children, &mut HashSet::new(), &mut order);
    let mut terminated = false;
    // Deepest children first so a shell cannot respawn its work while we walk upward.
    for id in order.iter().rev() {
        let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, *id) };
        if process.is_null() {
            continue;
        }
        if unsafe { TerminateProcess(process, 1) } != 0 {
            terminated = true;
        }
        unsafe {
            CloseHandle(process);
        }
    }
    terminated
}

#[cfg(not(windows))]
pub fn kill_process_tree(pid: u32) -> bool {
    Command::new("kill")
        .args(["-TERM", &format!("-{pid}")])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn taskkill_on_invalid_pid_returns_false() {
        assert!(!kill_process_tree(999_999));
    }

    #[cfg(windows)]
    #[test]
    fn kill_process_tree_terminates_a_child_process_tree() {
        let mut child = Command::new("cmd")
            .args(["/C", "ping -n 30 127.0.0.1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(400));
        assert!(
            kill_process_tree(child.id()),
            "process-tree termination should report success"
        );
        let status = child.wait().unwrap();
        assert!(!status.success());
    }
}
