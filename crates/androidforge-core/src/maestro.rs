//! Pinned, locally installed optional tools.
//!
//! AndroidForge does not require Maestro, but when a user or agent asks it to
//! install the optional integration we must download a specific, verified
//! build. A floating "latest" would make the CLI contract depend on a third
//! party release that changed after this binary was tested.

use std::path::{Path, PathBuf};

pub const MAESTRO_PINNED_VERSION: &str = "2.12.1";
pub const MAESTRO_PINNED_ZIP_URL: &str =
    "https://github.com/mobile-dev-inc/maestro/releases/download/cli-2.12.1/maestro.zip";
pub const MAESTRO_PINNED_ZIP_SHA256: &str =
    "54dbb0716425626604973cbb374c585ca30272375d9ce15177a1152765ddd9d7";

#[derive(Debug, Clone)]
pub struct ManagedMaestro {
    pub version: String,
    pub root: PathBuf,
    pub launcher: PathBuf,
}

/// The launcher name inside the Maestro distribution for the current OS.
pub fn maestro_launcher_name() -> &'static str {
    if cfg!(windows) {
        "maestro.bat"
    } else {
        "maestro"
    }
}

/// Root directory for AndroidForge-managed optional tools.
pub fn managed_tools_dir() -> PathBuf {
    crate::paths::managed_tools_dir()
}

/// Scan installed Maestro versions and return the highest version first.
pub fn installed_maestros() -> Vec<ManagedMaestro> {
    let root = managed_tools_dir().join("maestro");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let launcher = maestro_launcher_name();
    let mut installs = entries
        .flatten()
        .filter_map(|entry| {
            let version = entry.file_name().to_string_lossy().into_owned();
            let root = entry.path().join("maestro");
            let launcher = root.join("bin").join(launcher);
            launcher.is_file().then_some(ManagedMaestro {
                version,
                root,
                launcher,
            })
        })
        .collect::<Vec<_>>();
    installs.sort_by_key(|install| std::cmp::Reverse(version_key(&install.version)));
    installs
}

pub fn latest_installed() -> Option<ManagedMaestro> {
    installed_maestros().into_iter().next()
}

/// Return the managed install for a specific version, when present.
pub fn installed_version(version: &str) -> Option<ManagedMaestro> {
    installed_maestros()
        .into_iter()
        .find(|install| install.version == version)
}

/// True when `path` is inside the AndroidForge-managed Maestro directory.
pub fn is_managed_launcher(path: &Path) -> bool {
    let root = managed_tools_dir().join("maestro");
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    path.starts_with(root)
}

fn version_key(version: &str) -> Vec<u64> {
    version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_key_orders_semantic_versions() {
        assert!(version_key("2.12.1") > version_key("2.9.0"));
        assert!(version_key("2.12.1") < version_key("2.13.0"));
    }

    #[test]
    fn pinned_metadata_is_internally_consistent() {
        assert_eq!(MAESTRO_PINNED_VERSION, "2.12.1");
        assert!(MAESTRO_PINNED_ZIP_URL.contains(MAESTRO_PINNED_VERSION));
        assert_eq!(MAESTRO_PINNED_ZIP_SHA256.len(), 64);
    }
}
