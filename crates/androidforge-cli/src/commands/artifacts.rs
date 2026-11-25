//! Read-only, host-only inspection of AndroidForge-owned artifact text.
//!
//! This command never resolves a session, contacts ADB or mutates a project.
//! Findings carry location metadata and category-derived previews only; the
//! matched value is never read into the result or printed.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::paths::ForgePaths;
use androidforge_diagnostics::{SECRET_SCAN_MAX_BYTES, SECRET_SCAN_MAX_FINDINGS, scan_secrets};
use serde_json::json;

use crate::cli::{ArtifactsScanSecretsArgs, Cli};
use crate::commands::common::success;

/// Hard bounds for one scan. They are deliberately independent of configuration
/// so an untrusted artifact tree cannot make reporting unbounded.
const MAX_FILES: usize = 4_000;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DEPTH: usize = 12;

pub fn scan_secrets_command(cli: &Cli, args: &ArtifactsScanSecretsArgs) -> Result<CommandSuccess> {
    let root = scan_root(cli, args)?;
    let exists = root.is_dir();
    let mut files = Vec::new();
    if exists {
        collect_files(&root, &root, 0, &mut files)?;
        files.sort();
    }

    let mut files_scanned = 0usize;
    let mut files_skipped_binary = 0usize;
    let mut files_skipped_oversized = 0usize;
    let mut findings = Vec::new();
    let mut files_visited = 0usize;
    let mut total_bytes = 0u64;
    let mut truncated = !exists;

    for path in files {
        if files_visited >= MAX_FILES || total_bytes >= MAX_TOTAL_BYTES {
            truncated = true;
            break;
        }
        files_visited += 1;
        let relative = relative_path(&root, &path);
        if is_binary_name(&path) {
            files_skipped_binary += 1;
            continue;
        }
        let Ok(mut file) = File::open(&path) else {
            // Unreadable files are skipped without exposing their contents.
            continue;
        };
        let mut bytes = Vec::new();
        let Ok(read) = file
            .by_ref()
            .take(SECRET_SCAN_MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
        else {
            continue;
        };
        let _ = read;
        if bytes.len() > SECRET_SCAN_MAX_BYTES {
            files_skipped_oversized += 1;
            truncated = true;
            continue;
        }
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        let Ok(text) = std::str::from_utf8(&bytes) else {
            files_skipped_binary += 1;
            continue;
        };
        if text.as_bytes().contains(&0) {
            files_skipped_binary += 1;
            continue;
        }
        files_scanned += 1;
        for finding in scan_secrets(text) {
            if findings.len() >= SECRET_SCAN_MAX_FINDINGS {
                truncated = true;
                break;
            }
            findings.push(json!({
                "path": relative,
                "line": finding.line,
                "kind": finding.label(),
                "preview": finding.preview,
                "start": finding.start,
                "end": finding.end,
            }));
        }
        if findings.len() >= SECRET_SCAN_MAX_FINDINGS {
            truncated = true;
            break;
        }
    }

    let findings_total = findings.len();
    let result = json!({
        "root": root.to_string_lossy(),
        "exists": exists,
        "files_scanned": files_scanned,
        "files_skipped_binary": files_skipped_binary,
        "files_skipped_oversized": files_skipped_oversized,
        "findings_total": findings_total,
        "truncated": truncated,
        "findings": findings,
        "files_visited": files_visited,
        "max_bytes_per_file": SECRET_SCAN_MAX_BYTES,
        "max_findings": SECRET_SCAN_MAX_FINDINGS,
        "max_files": MAX_FILES,
        "max_total_bytes": MAX_TOTAL_BYTES,
        "binary_media_not_scanned": true,
        "note": "Binary screenshots, recordings and other media are disclosed but not scanned; finding previews are category-derived and never contain matched values.",
    });
    let human = format!(
        "Scanned {} artifact text file(s) under {}; {} binary file(s) not scanned, {} oversized file(s) skipped, {} suspected secret(s). Truncated: {}.\n",
        files_scanned,
        root.display(),
        files_skipped_binary,
        files_skipped_oversized,
        findings_total,
        truncated
    );
    Ok(success("artifacts.scan-secrets", result, human))
}

fn scan_root(cli: &Cli, args: &ArtifactsScanSecretsArgs) -> Result<PathBuf> {
    if let Some(path) = &args.path {
        let root = androidforge_core::util::canonicalize_or_self(path);
        if !root.is_dir() {
            return Err(ForgeError::invalid_argument(format!(
                "scan path {} is not a directory",
                root.display()
            )));
        }
        return Ok(root);
    }
    let paths = ForgePaths::discover(None, cli.project.clone())?;
    Ok(paths.artifacts_dir)
}

fn collect_files(root: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|error| {
        ForgeError::new(
            ErrorCode::InvalidArgument,
            format!("could not read {}: {error}", dir.display()),
        )
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_file() {
            out.push(path);
        } else if metadata.is_dir() {
            collect_files(root, &path, depth + 1, out)?;
        }
    }
    let _ = root;
    Ok(())
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn is_binary_name(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "bmp"
            | "ico"
            | "mp4"
            | "webm"
            | "mov"
            | "avi"
            | "mkv"
            | "apk"
            | "zip"
            | "gz"
            | "tar"
            | "bin"
            | "dex"
            | "so"
            | "pdf"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_names_are_disclosed_as_not_scanned() {
        assert!(is_binary_name(Path::new("capture/screen.png")));
        assert!(is_binary_name(Path::new("recordings/demo.mp4")));
        assert!(!is_binary_name(Path::new("logs/logcat.txt")));
    }

    #[test]
    fn relative_paths_use_forward_slashes() {
        let root = Path::new("A:/project/.artifacts");
        assert_eq!(
            relative_path(root, &root.join("runs").join("one").join("log.txt")),
            "runs/one/log.txt"
        );
    }
}
