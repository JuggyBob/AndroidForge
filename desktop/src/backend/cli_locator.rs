use std::path::{Path, PathBuf};

use super::{DesktopError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliResolution {
    pub path: PathBuf,
    pub source: CliSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliSource {
    Configured,
    Adjacent,
    Development,
    Path,
    InstallDirectory,
}

impl CliSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Configured => "configured path",
            Self::Adjacent => "bundled executable",
            Self::Development => "development build",
            Self::Path => "PATH",
            Self::InstallDirectory => "installation directory",
        }
    }
}

pub fn resolve(explicit: Option<&Path>) -> Result<CliResolution> {
    if let Some(path) = explicit {
        if !path.is_file() {
            return Err(DesktopError::CliNotFound {
                searched: vec![path.to_path_buf()],
            });
        }
        return Ok(CliResolution {
            path: path.to_path_buf(),
            source: CliSource::Configured,
        });
    }

    let mut searched = Vec::new();
    let adjacent = current_exe_dir().map(|dir| dir.join(cli_name()));
    if let Some(path) = adjacent {
        searched.push(path.clone());
        if path.is_file() {
            return Ok(CliResolution {
                path,
                source: CliSource::Adjacent,
            });
        }
    }

    for path in development_candidates() {
        if !searched.contains(&path) {
            searched.push(path.clone());
        }
        if path.is_file() {
            return Ok(CliResolution {
                path,
                source: CliSource::Development,
            });
        }
    }

    for path in install_candidates() {
        if !searched.contains(&path) {
            searched.push(path.clone());
        }
        if path.is_file() {
            return Ok(CliResolution {
                path,
                source: CliSource::InstallDirectory,
            });
        }
    }

    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let path = dir.join(cli_name());
            if !searched.contains(&path) {
                searched.push(path.clone());
            }
            if path.is_file() {
                return Ok(CliResolution {
                    path,
                    source: CliSource::Path,
                });
            }
        }
    }

    Err(DesktopError::CliNotFound { searched })
}

pub fn current_exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

fn install_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local)
                .join("Programs")
                .join("AndroidForge")
                .join(cli_name()),
        );
    }
    if let Ok(program_files) = std::env::var("ProgramFiles") {
        candidates.push(
            PathBuf::from(program_files)
                .join("AndroidForge")
                .join(cli_name()),
        );
    }
    if let Ok(program_files) = std::env::var("ProgramFiles(x86)") {
        candidates.push(
            PathBuf::from(program_files)
                .join("AndroidForge")
                .join(cli_name()),
        );
    }
    candidates
}

fn development_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = current_exe_dir() {
        // target/debug/androidforge-desktop.exe -> target/release/androidforge.exe
        if let Some(target) = dir.parent() {
            candidates.push(target.join("release").join(cli_name()));
            candidates.push(target.join("debug").join(cli_name()));
            if let Some(root) = target.parent() {
                candidates.push(root.join("target").join("release").join(cli_name()));
            }
        }
    }
    if let Some(root) = workspace_root() {
        candidates.push(root.join("target").join("release").join(cli_name()));
        candidates.push(root.join("target").join("debug").join(cli_name()));
    }
    candidates
}

pub fn workspace_root() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("ANDROIDFORGE_WORKSPACE") {
        let path = PathBuf::from(value);
        if path.is_dir() {
            return Some(path);
        }
    }
    // `desktop/` is the manifest dir in development.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().map(Path::to_path_buf)
}

pub fn cli_name() -> &'static str {
    if cfg!(windows) {
        "androidforge.exe"
    } else {
        "androidforge"
    }
}

pub fn desktop_name() -> &'static str {
    if cfg!(windows) {
        "androidforge-desktop.exe"
    } else {
        "androidforge-desktop"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_missing_path_is_an_error() {
        let error = resolve(Some(Path::new("definitely-not-a-real-androidforge.exe"))).unwrap_err();
        assert!(matches!(error, DesktopError::CliNotFound { .. }));
    }

    #[test]
    fn explicit_existing_path_wins() {
        let path = std::env::current_exe().unwrap();
        let resolution = resolve(Some(&path)).unwrap();
        assert_eq!(resolution.source, CliSource::Configured);
        assert_eq!(resolution.path, path);
    }

    #[test]
    fn development_candidates_include_target_release() {
        let candidates = development_candidates();
        assert!(
            candidates
                .iter()
                .any(|path| path.ends_with("release/androidforge.exe")
                    || path.ends_with("release\\androidforge.exe")
                    || path.ends_with("release/androidforge")
                    || path.ends_with("release\\androidforge"))
        );
    }
}
