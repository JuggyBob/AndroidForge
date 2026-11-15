use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, ForgeError, Result};
use crate::model::ArtifactRef;
use crate::util::{now_rfc3339, relative_to, sha256_file, write_atomic};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub run_id: String,
    pub generated_at: String,
    pub files: Vec<ManifestFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestFile {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

pub fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute() {
        return Err(ForgeError::invalid_argument(format!(
            "artifact path must be relative to the run directory: {relative}"
        )));
    }
    let mut clean = PathBuf::new();
    for component in relative_path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ForgeError::invalid_argument(format!(
                    "artifact path escapes the run directory: {relative}"
                )));
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(ForgeError::invalid_argument("artifact path is empty"));
    }
    Ok(root.join(clean))
}

pub fn write(
    root: &Path,
    relative: &str,
    kind: &str,
    description: Option<&str>,
    bytes: &[u8],
) -> Result<ArtifactRef> {
    let path = safe_join(root, relative)?;
    write_atomic(&path, bytes)?;
    let sha256 = crate::util::sha256_hex(bytes);
    let mut artifact = ArtifactRef::new(kind, relative.replace('\\', "/"))
        .with_size(bytes.len() as u64)
        .with_sha256(sha256);
    if let Some(description) = description {
        artifact = artifact.with_description(description);
    }
    Ok(artifact)
}

pub fn register(
    root: &Path,
    path: &Path,
    kind: &str,
    description: Option<&str>,
) -> Result<ArtifactRef> {
    let relative = relative_to(path, root);
    let size = fs::metadata(path)
        .map_err(|error| {
            ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!("could not stat {}: {error}", path.display()),
            )
        })?
        .len();
    let sha256 = sha256_file(path)?;
    let mut artifact = ArtifactRef::new(kind, relative)
        .with_size(size)
        .with_sha256(sha256);
    if let Some(description) = description {
        artifact = artifact.with_description(description);
    }
    Ok(artifact)
}

pub fn copy_into(
    root: &Path,
    source: &Path,
    relative: &str,
    kind: &str,
    description: Option<&str>,
) -> Result<ArtifactRef> {
    let bytes = fs::read(source).map_err(|error| {
        ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            format!("could not read {}: {error}", source.display()),
        )
    })?;
    write(root, relative, kind, description, &bytes)
}

pub fn build_manifest(root: &Path, run_id: &str) -> Result<Manifest> {
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(Manifest {
        schema_version: 1,
        run_id: run_id.to_string(),
        generated_at: now_rfc3339(),
        files,
    })
}

fn walk(root: &Path, directory: &Path, files: &mut Vec<ManifestFile>) -> Result<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            return Err(ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!("could not read {}: {error}", directory.display()),
            ));
        }
    };
    let mut entries = entries.filter_map(|entry| entry.ok()).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.is_dir() {
            walk(root, &path, files)?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".tmp") || name == "manifest.json" {
            continue;
        }
        let relative = relative_to(&path, root);
        let sha256 = sha256_file(&path)?;
        files.push(ManifestFile {
            path: relative,
            size_bytes: metadata.len(),
            sha256,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_join_rejects_escape() {
        let root = Path::new("run");
        assert!(safe_join(root, "../secret").is_err());
        assert!(safe_join(root, "C:/secret").is_err());
        assert_eq!(
            safe_join(root, "logs/a.txt").unwrap(),
            root.join("logs/a.txt")
        );
    }

    #[test]
    fn manifest_lists_files_with_checksums() {
        let temp = tempfile::tempdir().unwrap();
        write(temp.path(), "logs/a.txt", "logcat", None, b"hello").unwrap();
        write(temp.path(), "screenshots/a.png", "screenshot", None, b"png").unwrap();
        let manifest = build_manifest(temp.path(), "run-1").unwrap();
        assert_eq!(manifest.files.len(), 2);
        assert!(manifest.files.iter().all(|file| file.sha256.len() == 64));
    }
}
