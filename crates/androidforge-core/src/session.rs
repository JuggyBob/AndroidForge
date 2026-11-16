use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::model::BuildIdentity;

use crate::error::{ErrorCode, ForgeError, Result};
use crate::paths::ForgePaths;
use crate::util::{
    ensure_dir, now_rfc3339, read_json, sanitize_serial, short_id, write_atomic, write_json_atomic,
};

pub const SESSION_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_STALE_AFTER_SECS: i64 = 2 * 60 * 60;
pub const DEAD_OPERATION_GRACE_SECS: i64 = 10 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Emulator,
    Attached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Starting,
    Ready,
    Stopping,
    Released,
    Failed,
    Abandoned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationLock {
    pub pid: u32,
    pub operation: String,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRecord {
    pub action: String,
    pub name: String,
    pub at: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub schema_version: u32,
    pub id: String,
    pub owner: String,
    pub kind: SessionKind,
    pub state: SessionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default)]
    pub headless: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emulator_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emulator_args: Vec<String>,
    pub created_at: String,
    pub heartbeat_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<OperationLock>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub device_settings: BTreeMap<String, Option<String>>,
    /// Last build identity observed in this session. Installs update it; runs
    /// use it so a report always names the build under test.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildIdentity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snapshots: Vec<SnapshotRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Project root recorded when the session is acquired by `session ensure`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    /// Advisory lease expiry. An expired lease marks the session as requiring
    /// reconciliation; it never stops the emulator or interrupts work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_ttl_seconds: Option<u64>,
}

impl Session {
    pub fn new(
        owner: impl Into<String>,
        kind: SessionKind,
        serial: Option<String>,
        avd: Option<String>,
        port: Option<u16>,
        headless: bool,
    ) -> Self {
        let now = now_rfc3339();
        Self {
            schema_version: SESSION_SCHEMA_VERSION,
            id: format!("ses-{}", short_id()),
            owner: owner.into(),
            kind,
            state: SessionState::Starting,
            serial,
            avd,
            port,
            headless,
            profile: None,
            emulator_pid: None,
            emulator_args: Vec::new(),
            created_at: now.clone(),
            heartbeat_at: now.clone(),
            updated_at: now,
            created_by_pid: Some(std::process::id()),
            operation: None,
            device_settings: BTreeMap::new(),
            build: None,
            snapshots: Vec::new(),
            notes: Vec::new(),
            last_error: None,
            project_root: None,
            lease_expires_at: None,
            lease_ttl_seconds: None,
        }
    }

    pub fn create(&self, paths: &ForgePaths) -> Result<()> {
        let dir = session_dir(paths, &self.id)?;
        ensure_dir(&dir)?;
        self.save(paths)
    }

    pub fn save(&self, paths: &ForgePaths) -> Result<()> {
        let dir = session_dir(paths, &self.id)?;
        ensure_dir(&dir)?;
        write_json_atomic(&dir.join("session.json"), self)
    }

    pub fn load(paths: &ForgePaths, id: &str) -> Result<Self> {
        let dir = session_dir(paths, id)?;
        let path = dir.join("session.json");
        if !path.is_file() {
            return Err(ForgeError::new(
                ErrorCode::SessionNotFound,
                format!("session '{id}' was not found"),
            )
            .with_suggestion("Run `androidforge session list` to see available sessions.")
            .with_suggestion("Start one with `androidforge session start --avd <AVD>`."));
        }
        let session: Session = read_json(&path).map_err(|error| {
            ForgeError::new(
                ErrorCode::SessionExpired,
                format!("session '{id}' metadata could not be read: {error}"),
            )
        })?;
        if session.schema_version != SESSION_SCHEMA_VERSION {
            return Err(ForgeError::new(
                ErrorCode::SessionExpired,
                format!(
                    "session '{id}' uses schema {}; this build supports {}",
                    session.schema_version, SESSION_SCHEMA_VERSION
                ),
            )
            .with_suggestion("Release the old session and create a new one."));
        }
        Ok(session)
    }

    pub fn touch(&mut self, paths: &ForgePaths) -> Result<()> {
        let now = now_rfc3339();
        self.heartbeat_at = now.clone();
        self.updated_at = now;
        self.save(paths)
    }

    pub fn serial(&self) -> Result<&str> {
        self.serial.as_deref().ok_or_else(|| {
            ForgeError::new(
                ErrorCode::SessionExpired,
                format!("session {} has no device serial yet", self.id),
            )
            .with_suggestion("The session may still be starting; retry after it is ready.")
        })
    }

    pub fn mark_ready(&mut self, paths: &ForgePaths) -> Result<()> {
        self.state = SessionState::Ready;
        self.last_error = None;
        self.touch(paths)
    }

    pub fn mark_failed(&mut self, paths: &ForgePaths, error: impl Into<String>) -> Result<()> {
        self.state = SessionState::Failed;
        self.last_error = Some(error.into());
        self.operation = None;
        self.touch(paths)?;
        release_device_lock(paths, &self.id, self.serial.as_deref())
    }

    pub fn mark_stopping(&mut self, paths: &ForgePaths) -> Result<()> {
        self.state = SessionState::Stopping;
        self.touch(paths)
    }

    pub fn begin_operation(&mut self, paths: &ForgePaths, operation: &str) -> Result<()> {
        let operation = operation.to_string();
        let id = self.id.clone();
        with_global_lock(paths, || {
            let mut disk = Session::load(paths, &id)?;
            if let Some(existing) = &disk.operation {
                let age = age_secs(&existing.started_at);
                let alive = androidforge_process::process_exists(existing.pid);
                if alive && age < 60 * 60 {
                    return Err(ForgeError::new(
                        ErrorCode::DeviceBusy,
                        format!(
                            "session {} is already running '{}' (pid {}, started {})",
                            disk.id, existing.operation, existing.pid, existing.started_at
                        ),
                    )
                    .with_suggestion(
                        "Wait for the other operation to complete, or run `androidforge session gc --reclaim` if that process is gone.",
                    )
                    .session(disk.id.clone()));
                }
            }
            disk.operation = Some(OperationLock {
                pid: std::process::id(),
                operation,
                started_at: now_rfc3339(),
            });
            disk.touch(paths)?;
            *self = disk;
            Ok(())
        })
    }

    pub fn finish_operation(&mut self, paths: &ForgePaths) -> Result<()> {
        let id = self.id.clone();
        with_global_lock(paths, || {
            let mut disk = Session::load(paths, &id)?;
            if disk
                .operation
                .as_ref()
                .map(|operation| operation.pid == std::process::id())
                .unwrap_or(true)
            {
                disk.operation = None;
            }
            disk.touch(paths)?;
            *self = disk;
            Ok(())
        })
    }

    pub fn record_device_setting(
        &mut self,
        paths: &ForgePaths,
        key: impl Into<String>,
        previous: Option<String>,
    ) -> Result<()> {
        self.device_settings.entry(key.into()).or_insert(previous);
        self.touch(paths)
    }

    pub fn record_snapshot(
        &mut self,
        paths: &ForgePaths,
        action: &str,
        name: &str,
        success: bool,
        message: Option<String>,
    ) -> Result<()> {
        self.snapshots.push(SnapshotRecord {
            action: action.to_string(),
            name: name.to_string(),
            at: now_rfc3339(),
            success,
            message,
        });
        self.touch(paths)
    }

    pub fn add_note(&mut self, paths: &ForgePaths, note: impl Into<String>) -> Result<()> {
        let note = note.into();
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
        self.touch(paths)
    }

    pub fn release(&mut self, paths: &ForgePaths) -> Result<()> {
        let id = self.id.clone();
        let serial = self.serial.clone();
        with_global_lock(paths, || {
            release_device_lock_unlocked(paths, &id, serial.as_deref())?;
            let mut disk = Session::load(paths, &id)?;
            disk.state = SessionState::Released;
            disk.operation = None;
            disk.touch(paths)?;
            *self = disk;
            Ok(())
        })
    }

    pub fn acquire_device(&mut self, paths: &ForgePaths, serial: &str) -> Result<()> {
        let id = self.id.clone();
        let serial = serial.to_string();
        let owner = self.owner.clone();
        with_global_lock(paths, || {
            let lock_path = device_lock_path(paths, &serial)?;
            if lock_path.is_file() {
                if let Ok(existing) = read_json::<DeviceLock>(&lock_path) {
                    if existing.session_id != id {
                        let existing_session = Session::load(paths, &existing.session_id).ok();
                        if let Some(existing_session) = &existing_session {
                            let stale_operation = existing_session
                                .operation
                                .as_ref()
                                .map(|operation| {
                                    !androidforge_process::process_exists(operation.pid)
                                        && age_secs(&operation.started_at)
                                            > DEAD_OPERATION_GRACE_SECS
                                })
                                .unwrap_or(false);
                            let unusable = matches!(
                                existing_session.state,
                                SessionState::Released
                                    | SessionState::Failed
                                    | SessionState::Abandoned
                            ) || (matches!(
                                existing_session.state,
                                SessionState::Starting | SessionState::Stopping
                            ) && stale_operation);
                            if unusable {
                                let _ = fs::remove_file(&lock_path);
                            } else {
                                return Err(ForgeError::new(
                                    ErrorCode::DeviceBusy,
                                    format!(
                                        "device {serial} is owned by session {} ({}, state {:?})",
                                        existing_session.id,
                                        existing_session.owner,
                                        existing_session.state
                                    ),
                                )
                                .with_suggestion(format!(
                                    "Use `--session {}` if you are the owner.",
                                    existing_session.id
                                ))
                                .with_suggestion("Run `androidforge session list` to inspect ownership.")
                                .with_suggestion(
                                    "If that session is abandoned, run `androidforge session gc --reclaim`.",
                                )
                                .device(serial.clone())
                                .session(existing_session.id.clone()));
                            }
                        } else {
                            let _ = fs::remove_file(&lock_path);
                        }
                    }
                } else {
                    let _ = fs::remove_file(&lock_path);
                }
            }

            let lock = DeviceLock {
                schema_version: 1,
                session_id: id.clone(),
                serial: serial.clone(),
                owner,
                acquired_at: now_rfc3339(),
            };
            write_json_atomic(&lock_path, &lock)?;
            let mut disk = Session::load(paths, &id)?;
            disk.serial = Some(serial);
            disk.heartbeat_at = now_rfc3339();
            disk.updated_at = now_rfc3339();
            disk.save(paths)?;
            *self = disk;
            Ok(())
        })
    }

    pub fn age_seconds(&self) -> i64 {
        age_secs(&self.heartbeat_at)
    }

    pub fn is_stale(&self) -> bool {
        self.age_seconds() > DEFAULT_STALE_AFTER_SECS
    }

    /// An expired lease is advisory: it marks the session as needing
    /// reconciliation. It never stops the emulator or interrupts a run.
    pub fn lease_expired(&self) -> bool {
        self.lease_expired_at(Utc::now())
    }

    pub fn lease_expired_at(&self, now: DateTime<Utc>) -> bool {
        self.lease_expires_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|expiry| now >= expiry.with_timezone(&Utc))
            .unwrap_or(false)
    }

    pub fn lease_remaining_seconds(&self) -> Option<i64> {
        self.lease_expires_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|expiry| {
                expiry
                    .with_timezone(&Utc)
                    .signed_duration_since(Utc::now())
                    .num_seconds()
                    .max(0)
            })
    }

    /// Record an advisory lease and persist it. `None` clears the lease.
    pub fn set_lease(&mut self, paths: &ForgePaths, ttl: Option<Duration>) -> Result<()> {
        match ttl {
            Some(ttl) => {
                if ttl.is_zero() {
                    return Err(ForgeError::invalid_argument(
                        "lease duration must be greater than zero",
                    ));
                }
                let seconds = i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX);
                let expiry = Utc::now()
                    .checked_add_signed(chrono::Duration::seconds(seconds))
                    .ok_or_else(|| ForgeError::invalid_argument("lease duration is too large"))?;
                self.lease_ttl_seconds = Some(ttl.as_secs());
                self.lease_expires_at =
                    Some(expiry.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
            }
            None => {
                self.lease_ttl_seconds = None;
                self.lease_expires_at = None;
            }
        }
        self.touch(paths)
    }

    pub fn bind_project(&mut self, paths: &ForgePaths, project_root: &str) -> Result<()> {
        self.project_root = Some(project_root.to_string());
        self.touch(paths)
    }

    pub fn operation_active(&self) -> bool {
        self.operation
            .as_ref()
            .map(|operation| androidforge_process::process_exists(operation.pid))
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceLock {
    pub schema_version: u32,
    pub session_id: String,
    pub serial: String,
    pub owner: String,
    pub acquired_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListEntry {
    pub id: String,
    pub owner: String,
    pub kind: SessionKind,
    pub state: SessionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub headless: bool,
    pub created_at: String,
    pub heartbeat_at: String,
    pub age_seconds: i64,
    pub stale: bool,
    pub operation_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emulator_pid: Option<u32>,
    pub emulator_process_alive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_ttl_seconds: Option<u64>,
    pub lease_expired: bool,
}

impl From<&Session> for SessionListEntry {
    fn from(session: &Session) -> Self {
        Self {
            id: session.id.clone(),
            owner: session.owner.clone(),
            kind: session.kind,
            state: session.state,
            serial: session.serial.clone(),
            avd: session.avd.clone(),
            port: session.port,
            headless: session.headless,
            created_at: session.created_at.clone(),
            heartbeat_at: session.heartbeat_at.clone(),
            age_seconds: session.age_seconds(),
            stale: session.is_stale(),
            operation_active: session.operation_active(),
            operation: session
                .operation
                .as_ref()
                .map(|operation| operation.operation.clone()),
            emulator_pid: session.emulator_pid,
            emulator_process_alive: session
                .emulator_pid
                .map(androidforge_process::process_exists)
                .unwrap_or(false),
            last_error: session.last_error.clone(),
            project_root: session.project_root.clone(),
            lease_expires_at: session.lease_expires_at.clone(),
            lease_ttl_seconds: session.lease_ttl_seconds,
            lease_expired: session.lease_expired(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGcReport {
    pub sessions_scanned: usize,
    pub stale_sessions: Vec<String>,
    pub reclaimed_sessions: Vec<String>,
    pub cleared_operations: Vec<String>,
    pub dry_run: bool,
}

pub fn list_sessions(paths: &ForgePaths) -> Result<Vec<SessionListEntry>> {
    let dir = paths.sessions_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut sessions = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|error| {
        ForgeError::new(
            ErrorCode::Internal,
            format!("could not read {}: {error}", dir.display()),
        )
    })? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry.path().is_dir() {
            continue;
        }
        let session_path = entry.path().join("session.json");
        if !session_path.is_file() {
            continue;
        }
        if let Ok(session) = read_json::<Session>(&session_path) {
            sessions.push(SessionListEntry::from(&session));
        }
    }
    sessions.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(sessions)
}

/// Persist the build observed by an install so later runs in this session carry
/// the same identity without re-pulling and rehashing the APK.
pub fn record_build(paths: &ForgePaths, id: &str, build: BuildIdentity) -> Result<()> {
    with_global_lock(paths, || {
        let mut session = Session::load(paths, id)?;
        session.build = Some(build);
        session.touch(paths)
    })
}

/// Clear the cached build when its package is uninstalled.
pub fn clear_build(paths: &ForgePaths, id: &str, package: &str) -> Result<()> {
    with_global_lock(paths, || {
        let mut session = Session::load(paths, id)?;
        let matches = session
            .build
            .as_ref()
            .and_then(|build| build.package.as_deref())
            == Some(package);
        if matches {
            session.build = None;
            session.touch(paths)?;
        }
        Ok(())
    })
}

pub fn gc_sessions(paths: &ForgePaths, reclaim: bool, dry_run: bool) -> Result<SessionGcReport> {
    let sessions = list_sessions(paths)?;
    let mut report = SessionGcReport {
        sessions_scanned: sessions.len(),
        stale_sessions: Vec::new(),
        reclaimed_sessions: Vec::new(),
        cleared_operations: Vec::new(),
        dry_run,
    };

    for entry in sessions {
        let mut session = match Session::load(paths, &entry.id) {
            Ok(session) => session,
            Err(_) => continue,
        };
        let mut changed = false;

        if let Some(operation) = session.operation.clone()
            && !androidforge_process::process_exists(operation.pid)
            && age_secs(&operation.started_at) > DEAD_OPERATION_GRACE_SECS
        {
            report
                .cleared_operations
                .push(format!("{}:{}", session.id, operation.operation));
            if !dry_run {
                session.operation = None;
                changed = true;
            }
        }

        if session.is_stale()
            && !matches!(
                session.state,
                SessionState::Released | SessionState::Abandoned
            )
        {
            report.stale_sessions.push(session.id.clone());
            if reclaim {
                report.reclaimed_sessions.push(session.id.clone());
                if !dry_run {
                    if let Some(serial) = &session.serial {
                        release_device_lock_unlocked(paths, &session.id, Some(serial))?;
                    }
                    session.state = SessionState::Abandoned;
                    session.operation = None;
                    changed = true;
                }
            }
        }

        if changed {
            session.touch(paths)?;
        }
    }
    Ok(report)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPruneEntry {
    pub id: String,
    pub state: SessionState,
    pub serial: Option<String>,
    pub created_at: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPruneReport {
    pub sessions_scanned: usize,
    pub terminal_sessions: usize,
    pub pruned: Vec<SessionPruneEntry>,
    pub kept: usize,
    pub dry_run: bool,
}

/// Remove old terminal session tombstones. Active sessions, sessions with a
/// live operation or emulator, and the newest `keep_last` tombstones are kept.
/// The device lock is released only when it still belongs to the pruned session.
pub fn prune_sessions(
    paths: &ForgePaths,
    older_than: std::time::Duration,
    keep_last: usize,
    dry_run: bool,
) -> Result<SessionPruneReport> {
    let sessions = list_sessions(paths)?;
    let mut terminal: Vec<SessionListEntry> = sessions
        .iter()
        .filter(|entry| {
            matches!(
                entry.state,
                SessionState::Released | SessionState::Failed | SessionState::Abandoned
            )
        })
        .cloned()
        .collect();
    terminal.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    let terminal_sessions = terminal.len();
    let mut report = SessionPruneReport {
        sessions_scanned: sessions.len(),
        terminal_sessions,
        pruned: Vec::new(),
        kept: sessions.len(),
        dry_run,
    };
    for (index, entry) in terminal.iter().enumerate() {
        if index < keep_last {
            continue;
        }
        if entry.operation_active || entry.emulator_process_alive {
            continue;
        }
        let age = age_secs(&entry.created_at);
        if age < older_than.as_secs() as i64 {
            continue;
        }
        let reason = format!("terminal session older than {}s", older_than.as_secs());
        report.pruned.push(SessionPruneEntry {
            id: entry.id.clone(),
            state: entry.state,
            serial: entry.serial.clone(),
            created_at: entry.created_at.clone(),
            reason,
        });
        if !dry_run {
            release_device_lock_unlocked(paths, &entry.id, entry.serial.as_deref())?;
            let dir = paths.sessions_dir().join(&entry.id);
            if dir.is_dir() {
                fs::remove_dir_all(&dir).map_err(|error| {
                    ForgeError::new(
                        ErrorCode::Internal,
                        format!("could not remove session {}: {error}", dir.display()),
                    )
                })?;
            }
        }
    }
    report.kept = sessions.len().saturating_sub(report.pruned.len());
    Ok(report)
}

pub fn resolve_session_id(
    paths: &ForgePaths,
    cli_value: Option<&str>,
    env_value: Option<&str>,
) -> Result<Option<String>> {
    if let Some(value) = cli_value {
        let value = value.trim();
        if value.is_empty() {
            return Err(ForgeError::invalid_argument("--session cannot be empty"));
        }
        return Ok(Some(value.to_string()));
    }
    if let Some(value) = env_value {
        let value = value.trim();
        if !value.is_empty() {
            return Ok(Some(value.to_string()));
        }
    }
    let pointer = paths.session_pointer_path();
    if pointer.is_file() {
        let text = fs::read_to_string(&pointer).map_err(|error| {
            ForgeError::new(
                ErrorCode::SessionExpired,
                format!("could not read {}: {error}", pointer.display()),
            )
        })?;
        let value = text.trim();
        if !value.is_empty() {
            return Ok(Some(value.to_string()));
        }
    }
    Ok(None)
}

pub fn set_current_session(paths: &ForgePaths, session_id: &str) -> Result<()> {
    let path = paths.session_pointer_path();
    ensure_dir(path.parent().unwrap_or(&paths.project_root))?;
    let mut bytes = session_id.trim().as_bytes().to_vec();
    bytes.push(b'\n');
    write_atomic(&path, &bytes)
}

pub fn current_session(paths: &ForgePaths) -> Result<Option<String>> {
    let pointer = paths.session_pointer_path();
    if !pointer.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&pointer).map_err(|error| {
        ForgeError::new(
            ErrorCode::SessionExpired,
            format!("could not read {}: {error}", pointer.display()),
        )
    })?;
    let value = text.trim().to_string();
    Ok(if value.is_empty() { None } else { Some(value) })
}

pub fn owner_of_serial(paths: &ForgePaths, serial: &str) -> Result<Option<Session>> {
    let lock_path = match device_lock_path(paths, serial) {
        Ok(path) => path,
        Err(_) => return Ok(None),
    };
    if !lock_path.is_file() {
        return Ok(None);
    }
    let lock: DeviceLock = match read_json(&lock_path) {
        Ok(lock) => lock,
        Err(_) => {
            let _ = fs::remove_file(lock_path);
            return Ok(None);
        }
    };
    match Session::load(paths, &lock.session_id) {
        Ok(session) => Ok(Some(session)),
        Err(_) => {
            let _ = fs::remove_file(lock_path);
            Ok(None)
        }
    }
}

pub fn check_serial_ownership(
    paths: &ForgePaths,
    serial: &str,
    session_id: Option<&str>,
) -> Result<()> {
    if let Some(owner) = owner_of_serial(paths, serial)? {
        if matches!(
            owner.state,
            SessionState::Released | SessionState::Failed | SessionState::Abandoned
        ) {
            return Ok(());
        }
        if Some(owner.id.as_str()) != session_id {
            return Err(ForgeError::new(
                ErrorCode::DeviceBusy,
                format!(
                    "device {serial} is owned by session {} ({})",
                    owner.id, owner.owner
                ),
            )
            .with_suggestion(format!(
                "Use `--session {}` if you are the owner.",
                owner.id
            ))
            .with_suggestion("Run `androidforge session list` to inspect ownership.")
            .device(serial.to_string())
            .session(owner.id));
        }
    }
    Ok(())
}

pub fn release_device_lock(
    paths: &ForgePaths,
    session_id: &str,
    serial: Option<&str>,
) -> Result<()> {
    with_global_lock(paths, || {
        release_device_lock_unlocked(paths, session_id, serial)
    })
}

fn release_device_lock_unlocked(
    paths: &ForgePaths,
    session_id: &str,
    serial: Option<&str>,
) -> Result<()> {
    let Some(serial) = serial else {
        return Ok(());
    };
    let lock_path = device_lock_path(paths, serial)?;
    if !lock_path.is_file() {
        return Ok(());
    }
    if let Ok(lock) = read_json::<DeviceLock>(&lock_path)
        && lock.session_id == session_id
    {
        fs::remove_file(&lock_path).map_err(|error| {
            ForgeError::new(
                ErrorCode::DeviceBusy,
                format!(
                    "could not release device lock {}: {error}",
                    lock_path.display()
                ),
            )
        })?;
    }
    Ok(())
}

fn session_dir(paths: &ForgePaths, session_id: &str) -> Result<PathBuf> {
    if session_id.is_empty()
        || !session_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err(ForgeError::invalid_argument(format!(
            "invalid session id '{session_id}'"
        )));
    }
    Ok(paths.sessions_dir().join(session_id))
}

fn device_lock_path(paths: &ForgePaths, serial: &str) -> Result<PathBuf> {
    if serial.trim().is_empty() {
        return Err(ForgeError::invalid_argument(
            "device serial cannot be empty",
        ));
    }
    Ok(paths
        .locks_dir()
        .join("devices")
        .join(format!("{}.json", sanitize_serial(serial))))
}

fn with_global_lock<T>(paths: &ForgePaths, action: impl FnOnce() -> Result<T>) -> Result<T> {
    let locks = paths.locks_dir();
    ensure_dir(&locks)?;
    let path = locks.join(".global.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| {
            ForgeError::new(
                ErrorCode::Internal,
                format!("could not open lock {}: {error}", path.display()),
            )
        })?;
    let started = std::time::Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => break,
            Err(_) if started.elapsed() < Duration::from_secs(10) => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                return Err(ForgeError::new(
                    ErrorCode::DeviceBusy,
                    format!("could not acquire AndroidForge state lock: {error}"),
                )
                .with_suggestion("Retry after other AndroidForge commands finish."));
            }
        }
    }
    let result = action();
    let _ = fs2::FileExt::unlock(&file);
    if let Ok(mut file) = OpenOptions::new().write(true).open(&path) {
        let _ = writeln!(file, "{}", now_rfc3339());
    }
    result
}

fn age_secs(timestamp: &str) -> i64 {
    let Ok(parsed) = DateTime::parse_from_rfc3339(timestamp) else {
        return i64::MAX;
    };
    Utc::now()
        .signed_duration_since(parsed.with_timezone(&Utc))
        .num_seconds()
        .max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths(temp: &tempfile::TempDir) -> ForgePaths {
        let mut paths = ForgePaths::discover(
            Some(temp.path().to_path_buf()),
            Some(temp.path().to_path_buf()),
        )
        .unwrap();
        paths.state_dir = temp.path().join("state");
        paths
    }

    #[test]
    fn session_lifecycle_and_device_lock() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(&temp);
        let mut session = Session::new(
            "TestAgent",
            SessionKind::Attached,
            Some("emulator-5554".to_string()),
            None,
            None,
            true,
        );
        session.create(&paths).unwrap();
        session.acquire_device(&paths, "emulator-5554").unwrap();
        session.mark_ready(&paths).unwrap();
        assert_eq!(
            owner_of_serial(&paths, "emulator-5554")
                .unwrap()
                .unwrap()
                .id,
            session.id
        );
        assert!(check_serial_ownership(&paths, "emulator-5554", Some("ses-other")).is_err());
        session.release(&paths).unwrap();
        assert!(owner_of_serial(&paths, "emulator-5554").unwrap().is_none());
    }

    #[test]
    fn different_session_cannot_steal_device() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(&temp);
        let mut first = Session::new(
            "First",
            SessionKind::Attached,
            Some("emulator-5554".to_string()),
            None,
            None,
            true,
        );
        first.create(&paths).unwrap();
        first.acquire_device(&paths, "emulator-5554").unwrap();
        first.mark_ready(&paths).unwrap();

        let mut second = Session::new(
            "Second",
            SessionKind::Attached,
            Some("emulator-5554".to_string()),
            None,
            None,
            true,
        );
        second.create(&paths).unwrap();
        let error = second.acquire_device(&paths, "emulator-5554").unwrap_err();
        assert_eq!(error.code(), ErrorCode::DeviceBusy);
        assert!(
            error
                .body()
                .suggestions
                .iter()
                .any(|s| s.contains(&first.id))
        );
    }

    #[test]
    fn dead_operation_is_recovered() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(&temp);
        let mut session = Session::new(
            "TestAgent",
            SessionKind::Attached,
            Some("emulator-5554".to_string()),
            None,
            None,
            true,
        );
        session.create(&paths).unwrap();
        session.mark_ready(&paths).unwrap();
        let mut disk = Session::load(&paths, &session.id).unwrap();
        disk.operation = Some(OperationLock {
            pid: 999_999,
            operation: "app.install".to_string(),
            started_at: now_rfc3339(),
        });
        disk.save(&paths).unwrap();
        session.begin_operation(&paths, "ui.tap").unwrap();
        assert_eq!(
            Session::load(&paths, &session.id)
                .unwrap()
                .operation
                .unwrap()
                .operation,
            "ui.tap"
        );
    }

    #[test]
    fn ownership_check_handles_released_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(&temp);
        let mut session = Session::new(
            "TestAgent",
            SessionKind::Attached,
            Some("serial".to_string()),
            None,
            None,
            true,
        );
        session.create(&paths).unwrap();
        session.acquire_device(&paths, "serial").unwrap();
        session.release(&paths).unwrap();
        assert!(check_serial_ownership(&paths, "serial", None).is_ok());
    }

    #[test]
    fn lease_is_advisory_and_persisted() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(&temp);
        let mut session = Session::new(
            "TestAgent",
            SessionKind::Emulator,
            Some("emulator-5556".to_string()),
            Some("test_avd".to_string()),
            Some(5556),
            true,
        );
        session.create(&paths).unwrap();
        session.mark_ready(&paths).unwrap();
        session
            .set_lease(&paths, Some(Duration::from_secs(60)))
            .unwrap();
        assert!(!session.lease_expired());
        assert!(session.lease_remaining_seconds().unwrap() > 0);
        let loaded = Session::load(&paths, &session.id).unwrap();
        assert_eq!(loaded.lease_ttl_seconds, Some(60));
        assert!(loaded.lease_expires_at.is_some());

        let mut expired = loaded;
        expired.lease_expires_at = Some(
            (Utc::now() - chrono::Duration::seconds(5))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        );
        assert!(expired.lease_expired());
        expired
            .set_lease(&paths, Some(Duration::from_secs(30)))
            .unwrap();
        assert!(!expired.lease_expired());
        expired.set_lease(&paths, None).unwrap();
        assert!(expired.lease_expires_at.is_none());
        assert!(!expired.lease_expired());
        // Expiry is metadata only: the emulator PID is never touched by lease logic.
        assert_eq!(expired.state, SessionState::Ready);
    }

    #[test]
    fn project_binding_persists_without_changing_owner() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(&temp);
        let mut session = Session::new("AgentA", SessionKind::Attached, None, None, None, true);
        session.create(&paths).unwrap();
        session.bind_project(&paths, "C:\\work\\app").unwrap();
        let loaded = Session::load(&paths, &session.id).unwrap();
        assert_eq!(loaded.project_root.as_deref(), Some("C:\\work\\app"));
        assert_eq!(loaded.owner, "AgentA");
    }
}
