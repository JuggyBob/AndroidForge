use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{SecondsFormat, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{ForgeError, Result};

pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

pub fn elapsed_ms(started: std::time::Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

pub fn new_run_token() -> String {
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
    let uuid = Uuid::new_v4().simple().to_string();
    format!("{timestamp}-{}", &uuid[..8])
}

pub fn short_id() -> String {
    Uuid::new_v4().simple().to_string()[..12].to_string()
}

pub fn parse_duration(value: &str) -> std::result::Result<Duration, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("duration cannot be empty".to_string());
    }
    let (number, multiplier) = if let Some(number) = value.strip_suffix("ms") {
        (number, 0.001)
    } else if let Some(number) = value.strip_suffix('s') {
        (number, 1.0)
    } else if let Some(number) = value.strip_suffix('m') {
        (number, 60.0)
    } else if let Some(number) = value.strip_suffix('h') {
        (number, 3600.0)
    } else if let Some(number) = value.strip_suffix('d') {
        (number, 86_400.0)
    } else if let Some(number) = value.strip_suffix('w') {
        (number, 604_800.0)
    } else {
        (value, 1.0)
    };
    let amount: f64 = number.trim().parse().map_err(|_| {
        format!("invalid duration '{value}'; use forms such as 30s, 5m, 1h, 14d or 500ms")
    })?;
    if !amount.is_finite() || amount <= 0.0 {
        return Err(format!("duration '{value}' must be greater than zero"));
    }
    let seconds = amount * multiplier;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(format!("duration '{value}' must be greater than zero"));
    }
    Duration::try_from_secs_f64(seconds)
        .map_err(|_| format!("duration '{value}' is outside the supported range"))
}

pub fn slugify(value: &str, max_len: usize) -> String {
    let mut out = String::with_capacity(max_len);
    let mut last_dash = false;
    for ch in value.chars() {
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= max_len {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "run".to_string()
    } else {
        out
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|error| {
        ForgeError::new(
            crate::error::ErrorCode::ArtifactWriteFailed,
            format!("could not open {} for hashing: {error}", path.display()),
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            ForgeError::new(
                crate::error::ErrorCode::ArtifactWriteFailed,
                format!("could not read {} for hashing: {error}", path.display()),
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Write a file atomically enough for evidence: a unique sibling is written and
/// fsynced first, then renamed into place.
fn path_write_lock(path: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static LOCKS: OnceLock<Mutex<HashMap<std::path::PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks
        .entry(path.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    // Windows can transiently reject a replace when another thread still has
    // the target open. Serialize writers to the same path in-process; the
    // replace loop below handles cross-process sharing violations.
    let write_lock = path_write_lock(path);
    let _write_guard = write_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            ForgeError::new(
                crate::error::ErrorCode::ArtifactWriteFailed,
                format!("could not create {}: {error}", parent.display()),
            )
        })?;
    }
    let temp = temp_path_for(path);
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(|error| {
        ForgeError::new(
            crate::error::ErrorCode::ArtifactWriteFailed,
            format!("could not create {}: {error}", temp.display()),
        )
    })?;
    if let Err(error) = file.write_all(bytes) {
        let _ = fs::remove_file(&temp);
        return Err(ForgeError::new(
            crate::error::ErrorCode::ArtifactWriteFailed,
            format!("could not write {}: {error}", temp.display()),
        ));
    }
    if let Err(error) = file.sync_all() {
        let _ = fs::remove_file(&temp);
        return Err(ForgeError::new(
            crate::error::ErrorCode::ArtifactWriteFailed,
            format!("could not flush {}: {error}", temp.display()),
        ));
    }
    drop(file);

    // Windows refuses to rename over an existing file. MoveFileExW replaces it
    // atomically and writes through, so a crash cannot leave the target missing.
    for attempt in 0..40 {
        let result = replace_file(&temp, path);
        match result {
            Ok(()) => return Ok(()),
            Err(error) if attempt < 39 => {
                std::thread::sleep(Duration::from_millis(25));
                let _ = error;
            }
            Err(error) => {
                let _ = fs::remove_file(&temp);
                return Err(ForgeError::new(
                    crate::error::ErrorCode::ArtifactWriteFailed,
                    format!("could not replace {}: {error}", path.display()),
                ));
            }
        }
    }
    unreachable!("replace loop always returns")
}

#[cfg(windows)]
fn replace_file(temp: &Path, path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let wide = |value: &Path| {
        value
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let from = wide(temp);
    let to = wide(path);
    let result = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(temp: &Path, path: &Path) -> std::io::Result<()> {
    fs::rename(temp, path)
}

fn temp_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".to_string());
    let unique = Uuid::new_v4().simple().to_string();
    path.with_file_name(format!(".{file_name}.{unique}.tmp"))
}

pub fn write_json_atomic<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).map_err(|error| {
        ForgeError::new(
            crate::error::ErrorCode::ConfigInvalid,
            format!("could not read {}: {error}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        ForgeError::new(
            crate::error::ErrorCode::ConfigInvalid,
            format!("could not parse {}: {error}", path.display()),
        )
    })
}

pub fn canonicalize_or_self(path: &Path) -> PathBuf {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    // Windows canonicalization may add the extended-length prefix. Keep paths
    // readable in JSON and logs while retaining the canonical location.
    if let Some(value) = canonical.to_str()
        && let Some(stripped) = value.strip_prefix(r"\\?\")
    {
        return PathBuf::from(stripped);
    }
    canonical
}

pub fn relative_to(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
}

pub fn available_space(path: &Path) -> std::io::Result<u64> {
    fs2::available_space(path)
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn tail_lines(text: &str, lines: usize) -> String {
    let mut selected: Vec<&str> = text.lines().rev().take(lines).collect();
    selected.reverse();
    if selected.is_empty() {
        String::new()
    } else {
        format!("{}\n", selected.join("\n"))
    }
}

pub fn normalize_fingerprint(value: &str) -> Option<String> {
    let normalized: String = value
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_uppercase();
    if normalized.len() == 64 {
        Some(normalized)
    } else {
        None
    }
}

pub fn version_at_least(current: &str, required: &str) -> bool {
    fn parts(value: &str) -> Vec<u64> {
        value
            .split(['.', '-', '+'])
            .take(3)
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    }
    let current = parts(current);
    let required = parts(required);
    for index in 0..3 {
        let left = *current.get(index).unwrap_or(&0);
        let right = *required.get(index).unwrap_or(&0);
        if left > right {
            return true;
        }
        if left < right {
            return false;
        }
    }
    true
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|error| {
        ForgeError::new(
            crate::error::ErrorCode::ArtifactWriteFailed,
            format!("could not create {}: {error}", path.display()),
        )
    })
}

pub fn is_dir_writable(path: &Path) -> bool {
    if fs::create_dir_all(path).is_err() {
        return false;
    }
    let probe = path.join(format!(".androidforge-write-{}.tmp", short_id()));
    match OpenOptions::new().create_new(true).write(true).open(&probe) {
        Ok(mut file) => {
            let _ = file.write_all(b"ok");
            drop(file);
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

/// Validate an Android application package name before it is interpolated into
/// a remote path or passed to `run-as`/`pm`.
pub fn is_valid_package_name(package: &str) -> bool {
    if package.is_empty() || package.len() > 255 {
        return false;
    }
    let mut segments = package.split('.');
    let Some(first) = segments.next() else {
        return false;
    };
    if !valid_package_segment(first) {
        return false;
    }
    let mut count = 1;
    for segment in segments {
        if !valid_package_segment(segment) {
            return false;
        }
        count += 1;
    }
    count >= 2
}

fn valid_package_segment(segment: &str) -> bool {
    let mut characters = segment.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

pub fn sanitize_serial(serial: &str) -> String {
    let mut output = String::with_capacity(serial.len());
    for byte in serial.bytes() {
        let character = byte as char;
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
            output.push(character);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

pub fn shell_safe_hint(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':'))
    {
        value.to_string()
    } else {
        format!("{value:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_serial_is_injective_and_stable() {
        assert_eq!(sanitize_serial("emulator-5554"), "emulator-5554");
        assert_eq!(sanitize_serial("192.168.1.2:5555"), "192.168.1.2%3A5555");
        assert_ne!(sanitize_serial("a:b"), sanitize_serial("a_b"));
        assert_ne!(sanitize_serial("a:b"), sanitize_serial("a%3Ab"));
    }

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("30").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert!(parse_duration("0").is_err());
        assert!(parse_duration("nonsense").is_err());
    }

    #[test]
    fn normalizes_certificate_fingerprints() {
        let raw = "61:ED:37:7E:85:D3:86:A8:DF:EE:6B:86:4B:D8:5B:0B:FA:A5:AF:81:9B:3F:6B:5E:6B:6B:FF:3A:8E:0E:3F:4F";
        let normalized = normalize_fingerprint(raw).unwrap();
        assert_eq!(normalized.len(), 64);
        assert!(!normalized.contains(':'));
        assert!(normalize_fingerprint("abc").is_none());
    }

    #[test]
    fn version_comparison_handles_short_versions() {
        assert!(version_at_least("0.1.0", "0.1"));
        assert!(version_at_least("1.0.0", "0.9.9"));
        assert!(!version_at_least("0.1.0", "1.0"));
    }

    #[test]
    fn slugification_is_bounded_and_safe() {
        assert_eq!(
            slugify("Example App Smoke / 001", 40),
            "example-app-smoke-001"
        );
        assert_eq!(slugify("///", 10), "run");
    }

    #[test]
    fn tail_keeps_last_lines() {
        assert_eq!(tail_lines("a\nb\nc\n", 2), "b\nc\n");
    }

    #[test]
    fn atomic_write_replaces_an_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.json");
        write_json_atomic(&path, &serde_json::json!({"version": 1, "value": "old"})).unwrap();
        write_json_atomic(&path, &serde_json::json!({"version": 2, "value": "new"})).unwrap();
        let value: serde_json::Value = read_json(&path).unwrap();
        assert_eq!(value["version"], 2);
        assert_eq!(value["value"], "new");
    }

    #[test]
    fn concurrent_atomic_writes_leave_one_complete_json_value() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let mut handles = Vec::new();
        for index in 0..8 {
            let path = path.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for round in 0..20 {
                    write_json_atomic(
                        &path,
                        &serde_json::json!({"writer": index, "round": round, "payload": "x".repeat(4096)}),
                    )
                    .unwrap();
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        let value: serde_json::Value = read_json(&path).unwrap();
        assert!(value["writer"].as_u64().unwrap() < 8);
        assert!(value["round"].as_u64().unwrap() < 20);
        assert_eq!(value["payload"].as_str().unwrap().len(), 4096);
    }

    #[test]
    fn truncated_json_is_rejected_with_a_structured_error() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("broken.json");
        std::fs::write(&path, b"{\"partial\":").unwrap();
        let error = read_json::<serde_json::Value>(&path).unwrap_err();
        assert_eq!(error.code(), crate::error::ErrorCode::ConfigInvalid);
    }
}
