use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::util::{ensure_dir, is_valid_package_name, sha256_hex, write_atomic};
use androidforge_process::shell_quote;
use serde::{Deserialize, Serialize};

const MAX_STREAM_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppFileTransfer {
    pub package: String,
    pub direction: String,
    pub path: String,
    pub local_path: String,
    pub external: bool,
    pub size_bytes: u64,
    pub sha256: String,
    #[serde(default)]
    pub remote_size_verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_sha256: Option<String>,
    #[serde(default)]
    pub remote_hash_verified: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AppFiles {
    pub adb: AdbClient,
}

impl AppFiles {
    pub fn new(adb: AdbClient) -> Self {
        Self { adb }
    }

    pub async fn is_debuggable(&self, serial: &str, package: &str) -> Result<bool> {
        let output = self
            .adb
            .shell_args(
                serial,
                &["run-as".to_string(), package.to_string(), "id".to_string()],
                Some(Duration::from_secs(15)),
                Some(256 * 1024),
            )
            .await;
        match output {
            Ok(output) => {
                let text = format!("{}\n{}", output.stdout_text(), output.stderr_text());
                if text.to_ascii_lowercase().contains("not debuggable") {
                    return Ok(false);
                }
                Ok(output.success)
            }
            Err(error) => {
                let message = error.message().to_ascii_lowercase();
                if message.contains("not debuggable") {
                    Ok(false)
                } else if message.contains("unknown package") {
                    Err(ForgeError::new(
                        ErrorCode::PackageNotFound,
                        format!("package {package} is not installed"),
                    )
                    .device(serial.to_string()))
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn push(
        &self,
        serial: &str,
        package: &str,
        local: &Path,
        remote: &str,
        external: bool,
    ) -> Result<AppFileTransfer> {
        require_package_name(package, serial)?;
        if !local.is_file() {
            return Err(ForgeError::new(
                ErrorCode::FileNotFound,
                format!("local file {} was not found", local.display()),
            ));
        }
        let relative = validate_relative(remote)?;
        let bytes = std::fs::read(local).map_err(|error| {
            ForgeError::new(
                ErrorCode::FileNotFound,
                format!("could not read {}: {error}", local.display()),
            )
        })?;
        if bytes.len() > MAX_STREAM_BYTES {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                format!(
                    "file is {} bytes; app-file streaming is capped at {} bytes",
                    bytes.len(),
                    MAX_STREAM_BYTES
                ),
            )
            .with_suggestion(
                "Use raw ADB push for very large files, or a fixture-specific transfer.",
            )
            .device(serial.to_string()));
        }
        let sha256 = sha256_hex(&bytes);
        let mut notes = Vec::new();
        let mut remote_size_verified = false;
        if external {
            let full = external_path(package, &relative);
            let parent = parent_remote(&full);
            self.adb
                .shell_args(
                    serial,
                    &["mkdir".to_string(), "-p".to_string(), parent],
                    Some(Duration::from_secs(15)),
                    None,
                )
                .await?;
            self.adb.push(serial, local, &full).await?;
            if let Some(size) = self.remote_size(serial, package, &relative, true).await {
                remote_size_verified = size == bytes.len() as u64;
                if !remote_size_verified {
                    notes.push(format!(
                        "remote size {size} differs from local size {}",
                        bytes.len()
                    ));
                }
            }
            return Ok(AppFileTransfer {
                package: package.to_string(),
                direction: "push".to_string(),
                path: relative,
                local_path: local.to_string_lossy().into_owned(),
                external: true,
                size_bytes: bytes.len() as u64,
                sha256,
                remote_size_verified,
                remote_sha256: None,
                remote_hash_verified: false,
                notes,
            });
        }

        self.require_debuggable(serial, package).await?;
        let parent = parent_relative(&relative);
        if !parent.is_empty() {
            self.adb
                .shell_args(
                    serial,
                    &[
                        "run-as".to_string(),
                        package.to_string(),
                        "mkdir".to_string(),
                        "-p".to_string(),
                        parent,
                    ],
                    Some(Duration::from_secs(15)),
                    None,
                )
                .await?;
        }
        let script = format!("cat > {}", shell_quote(&relative));
        let output = self
            .adb
            .shell_args_with_stdin(
                serial,
                &[
                    "run-as".to_string(),
                    package.to_string(),
                    "sh".to_string(),
                    "-c".to_string(),
                    script,
                ],
                bytes.clone(),
                Some(Duration::from_secs(60)),
                Some(1024 * 1024),
            )
            .await?;
        if output.stdout_truncated || output.stderr_truncated {
            notes.push("transfer command output was truncated".to_string());
        }
        if let Some(size) = self.remote_size(serial, package, &relative, false).await {
            remote_size_verified = size == bytes.len() as u64;
            if !remote_size_verified {
                notes.push(format!(
                    "remote size {size} differs from local size {}",
                    bytes.len()
                ));
            }
        }
        let readback = self
            .adb
            .exec_out_args(
                serial,
                &[
                    "run-as".to_string(),
                    package.to_string(),
                    "cat".to_string(),
                    relative.clone(),
                ],
                Some(Duration::from_secs(120)),
                Some(MAX_STREAM_BYTES),
            )
            .await;
        let mut remote_sha256 = None;
        let mut remote_hash_verified = false;
        match readback {
            Ok(output) if !output.stdout_truncated => {
                let digest = sha256_hex(&output.stdout);
                remote_hash_verified = digest == sha256;
                remote_sha256 = Some(digest);
                if !remote_hash_verified {
                    notes.push("remote SHA-256 differs from the local file".to_string());
                }
            }
            Ok(_) => notes
                .push("remote SHA-256 read-back was truncated; checksum not verified".to_string()),
            Err(error) => notes.push(format!(
                "remote SHA-256 read-back failed: {}",
                error.message()
            )),
        }
        Ok(AppFileTransfer {
            package: package.to_string(),
            direction: "push".to_string(),
            path: relative,
            local_path: local.to_string_lossy().into_owned(),
            external: false,
            size_bytes: bytes.len() as u64,
            sha256,
            remote_size_verified,
            remote_sha256,
            remote_hash_verified,
            notes,
        })
    }

    pub async fn pull(
        &self,
        serial: &str,
        package: &str,
        remote: &str,
        local: &Path,
        external: bool,
    ) -> Result<AppFileTransfer> {
        require_package_name(package, serial)?;
        let relative = validate_relative(remote)?;
        if let Some(parent) = local.parent() {
            ensure_dir(parent)?;
        }
        let (bytes, external_used, notes) = if external {
            let full = external_path(package, &relative);
            self.adb.pull(serial, &full, local).await?;
            let bytes = std::fs::read(local).map_err(|error| {
                ForgeError::new(
                    ErrorCode::FileNotFound,
                    format!("could not read pulled file {}: {error}", local.display()),
                )
            })?;
            (bytes, true, Vec::new())
        } else {
            self.require_debuggable(serial, package).await?;
            let output = self
                .adb
                .exec_out_args(
                    serial,
                    &[
                        "run-as".to_string(),
                        package.to_string(),
                        "cat".to_string(),
                        relative.clone(),
                    ],
                    Some(Duration::from_secs(120)),
                    Some(MAX_STREAM_BYTES),
                )
                .await
                .map_err(|error| map_pull_error(error, package, &relative, serial))?;
            if output.stdout_truncated {
                return Err(ForgeError::new(
                    ErrorCode::CommandFailed,
                    format!(
                        "file {relative} exceeded the {} byte capture limit",
                        MAX_STREAM_BYTES
                    ),
                )
                .with_suggestion("Use raw ADB or a fixture-specific transfer for very large files.")
                .device(serial.to_string()));
            }
            (output.stdout, false, Vec::new())
        };
        write_atomic(local, &bytes)?;
        let sha256 = sha256_hex(&bytes);
        Ok(AppFileTransfer {
            package: package.to_string(),
            direction: "pull".to_string(),
            path: relative,
            local_path: local.to_string_lossy().into_owned(),
            external: external_used,
            size_bytes: bytes.len() as u64,
            sha256,
            remote_size_verified: false,
            remote_sha256: None,
            remote_hash_verified: false,
            notes,
        })
    }

    pub async fn backup_data(
        &self,
        serial: &str,
        package: &str,
        local: &Path,
    ) -> Result<AppFileTransfer> {
        require_package_name(package, serial)?;
        self.require_debuggable(serial, package).await?;
        let data_dir = format!("/data/data/{package}");
        let output = self
            .adb
            .exec_out_args(
                serial,
                &[
                    "run-as".to_string(),
                    package.to_string(),
                    "tar".to_string(),
                    "-C".to_string(),
                    data_dir.clone(),
                    "-cf".to_string(),
                    "-".to_string(),
                    ".".to_string(),
                ],
                Some(Duration::from_secs(300)),
                Some(MAX_STREAM_BYTES),
            )
            .await
            .map_err(|error| map_tar_error(error, package, serial))?;
        if output.stdout_truncated {
            return Err(ForgeError::new(
                ErrorCode::CommandFailed,
                format!("app data backup exceeded the {} byte capture limit", MAX_STREAM_BYTES),
            )
            .with_suggestion("Use a fixture-specific backup method or increase the limit in a future AndroidForge build.")
            .device(serial.to_string()));
        }
        if let Some(parent) = local.parent() {
            ensure_dir(parent)?;
        }
        write_atomic(local, &output.stdout)?;
        let sha256 = sha256_hex(&output.stdout);
        Ok(AppFileTransfer {
            package: package.to_string(),
            direction: "backup".to_string(),
            path: data_dir,
            local_path: local.to_string_lossy().into_owned(),
            external: false,
            size_bytes: output.stdout.len() as u64,
            sha256,
            remote_size_verified: false,
            remote_sha256: None,
            remote_hash_verified: false,
            notes: vec![
                "backup uses `run-as` and `tar` and requires a debuggable application".to_string(),
            ],
        })
    }

    pub async fn restore_data(
        &self,
        serial: &str,
        package: &str,
        local: &Path,
    ) -> Result<AppFileTransfer> {
        require_package_name(package, serial)?;
        self.require_debuggable(serial, package).await?;
        let bytes = std::fs::read(local).map_err(|error| {
            ForgeError::new(
                ErrorCode::FileNotFound,
                format!("could not read backup {}: {error}", local.display()),
            )
        })?;
        if bytes.len() > MAX_STREAM_BYTES {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                "backup exceeds the stream limit".to_string(),
            ));
        }
        validate_backup_tar(&bytes, package, serial)?;
        let data_dir = format!("/data/data/{package}");
        self.adb
            .shell_args_with_stdin(
                serial,
                &[
                    "run-as".to_string(),
                    package.to_string(),
                    "tar".to_string(),
                    "-xf".to_string(),
                    "-".to_string(),
                    "-C".to_string(),
                    data_dir.clone(),
                ],
                bytes.clone(),
                Some(Duration::from_secs(300)),
                Some(4 * 1024 * 1024),
            )
            .await
            .map_err(|error| map_tar_error(error, package, serial))?;
        let sha256 = sha256_hex(&bytes);
        Ok(AppFileTransfer {
            package: package.to_string(),
            direction: "restore".to_string(),
            path: data_dir,
            local_path: local.to_string_lossy().into_owned(),
            external: false,
            size_bytes: bytes.len() as u64,
            sha256,
            remote_size_verified: false,
            remote_sha256: None,
            remote_hash_verified: false,
            notes: vec![
                "restore overlays files from the backup; it does not clear existing data first"
                    .to_string(),
            ],
        })
    }

    async fn require_debuggable(&self, serial: &str, package: &str) -> Result<()> {
        if self.is_debuggable(serial, package).await? {
            Ok(())
        } else {
            Err(ForgeError::new(
                ErrorCode::AppNotDebuggable,
                format!("package {package} is not debuggable; app-private files are inaccessible"),
            )
            .with_suggestion("Build and install a debuggable APK for app-private file access.")
            .with_suggestion("External app files can be used with --external.")
            .with_suggestion("AndroidForge never roots a device or modifies system partitions.")
            .device(serial.to_string()))
        }
    }

    async fn remote_size(
        &self,
        serial: &str,
        package: &str,
        relative: &str,
        external: bool,
    ) -> Option<u64> {
        let args = if external {
            vec![
                "stat".to_string(),
                "-c".to_string(),
                "%s".to_string(),
                external_path(package, relative),
            ]
        } else {
            vec![
                "run-as".to_string(),
                package.to_string(),
                "stat".to_string(),
                "-c".to_string(),
                "%s".to_string(),
                relative.to_string(),
            ]
        };
        self.adb
            .shell_args(serial, &args, Some(Duration::from_secs(10)), None)
            .await
            .ok()
            .and_then(|output| output.stdout_text().trim().parse::<u64>().ok())
    }
}

fn require_package_name(package: &str, serial: &str) -> Result<()> {
    if is_valid_package_name(package) {
        return Ok(());
    }
    Err(ForgeError::new(
        ErrorCode::InvalidArgument,
        format!("'{package}' is not a valid Android package name"),
    )
    .with_suggestion("Use a name like com.example.app; separators and '..' are not allowed.")
    .device(serial.to_string()))
}

fn validate_backup_tar(bytes: &[u8], package: &str, serial: &str) -> Result<()> {
    let mut archive = tar::Archive::new(std::io::Cursor::new(bytes));
    let entries = archive.entries().map_err(|error| {
        ForgeError::new(
            ErrorCode::InvalidArgument,
            format!("backup is not a valid tar archive: {error}"),
        )
        .with_suggestion("Create the archive with `androidforge app data export`.")
        .device(serial.to_string())
    })?;
    let mut count = 0u32;
    for entry in entries {
        count += 1;
        if count > 100_000 {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                "backup tar has an unexpected number of entries",
            )
            .device(serial.to_string()));
        }
        let entry = entry.map_err(|error| {
            ForgeError::new(
                ErrorCode::InvalidArgument,
                format!("could not read backup tar entry: {error}"),
            )
            .device(serial.to_string())
        })?;
        let kind = entry.header().entry_type();
        let link = kind.is_symlink() || kind.is_hard_link();
        if !(kind.is_file() || kind.is_dir() || link) {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                format!("backup tar for {package} contains a non-regular entry"),
            )
            .with_suggestion(
                "Only files, directories and safe links are allowed; devices are rejected.",
            )
            .device(serial.to_string()));
        }
        if link {
            let target = entry.link_name().ok().flatten();
            let unsafe_target = target
                .as_deref()
                .map(|target| {
                    target.as_os_str().is_empty()
                        || target.components().any(|component| {
                            matches!(
                                component,
                                Component::ParentDir | Component::RootDir | Component::Prefix(_)
                            )
                        })
                })
                .unwrap_or(true);
            if unsafe_target {
                return Err(ForgeError::new(
                    ErrorCode::InvalidArgument,
                    "backup tar contains an unsafe link target",
                )
                .with_suggestion("Use an archive produced by `androidforge app data export`.")
                .device(serial.to_string()));
            }
        }
        let path = entry.path().map_err(|error| {
            ForgeError::new(
                ErrorCode::InvalidArgument,
                format!("could not read backup tar path: {error}"),
            )
            .device(serial.to_string())
        })?;
        if path.as_os_str().is_empty()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                "backup tar contains an absolute or parent path",
            )
            .with_suggestion("Use an archive produced by `androidforge app data export`.")
            .device(serial.to_string()));
        }
    }
    Ok(())
}

fn validate_relative(remote: &str) -> Result<String> {
    let relative = remote.replace('\\', "/");
    if relative.chars().any(char::is_control) {
        return Err(ForgeError::invalid_argument(
            "app file paths may not contain control characters",
        ));
    }
    let path = Path::new(&relative);
    if path.is_absolute() {
        return Err(ForgeError::invalid_argument(
            "app file paths must be relative to the app's data directory (for example files/state.json)",
        ));
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ForgeError::invalid_argument(
                    "app file paths may not contain '..' or absolute prefixes",
                ));
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(ForgeError::invalid_argument(
            "app file path cannot be empty",
        ));
    }
    Ok(clean.to_string_lossy().replace('\\', "/"))
}

fn parent_relative(relative: &str) -> String {
    Path::new(relative)
        .parent()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

fn parent_remote(full: &str) -> String {
    Path::new(full)
        .parent()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "/sdcard".to_string())
}

fn external_path(package: &str, relative: &str) -> String {
    format!("/sdcard/Android/data/{package}/files/{relative}")
}

fn map_pull_error(error: ForgeError, package: &str, path: &str, serial: &str) -> ForgeError {
    let text = error
        .body()
        .context
        .values()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    if text.contains("no such file") || text.contains("not found") {
        ForgeError::new(
            ErrorCode::FileNotFound,
            format!("app-private file {path} was not found in {package}"),
        )
        .with_suggestion("Inspect the app's data directory or run a fixture that creates the file.")
        .device(serial.to_string())
    } else if text.contains("not debuggable") {
        ForgeError::new(
            ErrorCode::AppNotDebuggable,
            format!("package {package} is not debuggable"),
        )
        .with_suggestion("Use a debuggable build or --external for app external files.")
        .device(serial.to_string())
    } else if text.contains("permission denied") {
        ForgeError::new(
            ErrorCode::PermissionDenied,
            format!("permission denied while reading {path} from {package}"),
        )
        .with_suggestion("Verify that the path is inside the app's private data directory.")
        .device(serial.to_string())
    } else {
        error
    }
}

fn map_tar_error(error: ForgeError, package: &str, serial: &str) -> ForgeError {
    let text = error
        .body()
        .context
        .values()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    if text.contains("tar: not found") || text.contains("inaccessible or not found") {
        ForgeError::new(
            ErrorCode::UnsupportedOperation,
            format!("the device does not provide tar for app data access in {package}"),
        )
        .with_suggestion("Use a project fixture with a supported backup mechanism instead.")
        .device(serial.to_string())
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_relative_app_paths() {
        assert_eq!(
            validate_relative("files/state.json").unwrap(),
            "files/state.json"
        );
        assert_eq!(
            validate_relative("./files/state.json").unwrap(),
            "files/state.json"
        );
        assert!(validate_relative("../secret").is_err());
        assert!(validate_relative("/data/data/com.example").is_err());
        assert!(validate_relative("files/line\nbreak").is_err());
    }
}
