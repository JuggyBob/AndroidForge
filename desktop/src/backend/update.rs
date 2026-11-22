use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::cli_runner::CliRunner;
use super::{DesktopError, Result};

pub const REPOSITORY: &str = "JuggyBob/AndroidForge";
pub const RELEASES_URL: &str = "https://github.com/JuggyBob/AndroidForge/releases";
pub const CLI_ASSET: &str = "androidforge-cli-windows-x86_64.zip";
pub const DESKTOP_ASSET: &str = "androidforge-desktop-windows-x86_64.zip";
pub const CHECKSUMS_ASSET: &str = "SHA256SUMS.txt";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReleaseAsset {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReleaseInfo {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub published_at: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheck {
    pub schema_version: u32,
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub checked_at: String,
    pub release: ReleaseInfo,
    pub cli_asset: Option<ReleaseAsset>,
    pub desktop_asset: Option<ReleaseAsset>,
    pub notes: Vec<String>,
}

pub fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

pub fn check_latest() -> Result<UpdateCheck> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let mut response = agent
        .get(&url)
        .header("User-Agent", format!("AndroidForge-Desktop/{}", current_version()))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| match error {
            ureq::Error::StatusCode(403) => DesktopError::Network(
                "GitHub API rate limit reached. Try again later, or open the releases page directly."
                    .to_string(),
            ),
            ureq::Error::StatusCode(code) => {
                DesktopError::Network(format!("GitHub returned HTTP {code}"))
            }
            other => DesktopError::Network(other.to_string()),
        })?;
    let body = response
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_to_string()
        .map_err(|error| {
            DesktopError::Network(format!("invalid GitHub release response: {error}"))
        })?;
    let release: ReleaseInfo = serde_json::from_str(&body)
        .map_err(|error| DesktopError::Network(format!("invalid GitHub release JSON: {error}")))?;
    let latest = release.tag_name.trim_start_matches('v').to_string();
    let current = current_version();
    let mut notes = Vec::new();
    if release.prerelease {
        notes.push("The latest GitHub release is marked prerelease.".to_string());
    }
    if release.assets.is_empty() {
        notes.push("The release has no downloadable assets.".to_string());
    }
    Ok(UpdateCheck {
        schema_version: 1,
        current_version: current.clone(),
        latest_version: latest.clone(),
        update_available: version_is_newer(&latest, &current),
        checked_at: chrono::Utc::now().to_rfc3339(),
        cli_asset: release
            .assets
            .iter()
            .find(|asset| asset.name == CLI_ASSET)
            .cloned(),
        desktop_asset: release
            .assets
            .iter()
            .find(|asset| asset.name == DESKTOP_ASSET)
            .cloned(),
        release,
        notes,
    })
}

pub fn version_is_newer(candidate: &str, current: &str) -> bool {
    let parse = |value: &str| -> Vec<u64> {
        value
            .trim_start_matches('v')
            .split(['.', '-', '+'])
            .take(3)
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let mut left = parse(candidate);
    let mut right = parse(current);
    left.resize(3, 0);
    right.resize(3, 0);
    left > right
}

pub fn parse_checksums(text: &str, asset: &str) -> Result<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            continue;
        };
        let name = name.trim_start_matches('*');
        if name == asset {
            if hash.len() != 64 || !hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(DesktopError::Verification(format!(
                    "SHA256SUMS.txt has an invalid hash for {asset}"
                )));
            }
            return Ok(hash.to_ascii_lowercase());
        }
    }
    Err(DesktopError::Verification(format!(
        "SHA256SUMS.txt does not list {asset}"
    )))
}

fn safe_release_url(url: &str) -> Result<&str> {
    if url.starts_with("https://github.com/JuggyBob/AndroidForge/releases/download/") {
        Ok(url)
    } else {
        Err(DesktopError::Verification(format!(
            "release asset URL is not on the expected GitHub release host: {url}"
        )))
    }
}

fn download(url: &str, limit: u64) -> Result<Vec<u8>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header(
            "User-Agent",
            format!("AndroidForge-Desktop/{}", current_version()),
        )
        .call()
        .map_err(|error| DesktopError::Network(format!("download failed: {error}")))?;
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|error| DesktopError::Network(format!("download failed: {error}")))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Download the CLI archive, verify its published SHA-256, extract the
/// executable to a staging directory and validate its `--version` output.
pub fn stage_cli_update(check: &UpdateCheck) -> Result<PathBuf> {
    let cli_asset = check.cli_asset.as_ref().ok_or_else(|| {
        DesktopError::Update(format!(
            "release {} has no {CLI_ASSET}",
            check.latest_version
        ))
    })?;
    let sums_asset = check
        .release
        .assets
        .iter()
        .find(|asset| asset.name == CHECKSUMS_ASSET)
        .ok_or_else(|| {
            DesktopError::Update(format!(
                "release {} has no {CHECKSUMS_ASSET}",
                check.latest_version
            ))
        })?;
    let sums_text = String::from_utf8_lossy(&download(
        safe_release_url(&sums_asset.browser_download_url)?,
        1024 * 1024,
    )?)
    .into_owned();
    let expected = parse_checksums(&sums_text, CLI_ASSET)?;
    let archive = download(
        safe_release_url(&cli_asset.browser_download_url)?,
        256 * 1024 * 1024,
    )?;
    let actual = sha256_hex(&archive);
    if actual != expected {
        return Err(DesktopError::Verification(format!(
            "downloaded CLI archive hash {actual} does not match published {expected}"
        )));
    }
    let staging = tempfile::Builder::new()
        .prefix("androidforge-update-")
        .tempdir()
        .map_err(|error| {
            DesktopError::Update(format!("could not create staging directory: {error}"))
        })?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
        .map_err(|error| DesktopError::Update(format!("invalid CLI archive: {error}")))?;
    let mut entry = zip.by_name("androidforge.exe").map_err(|error| {
        DesktopError::Update(format!("CLI archive has no androidforge.exe: {error}"))
    })?;
    if entry.size() > 100 * 1024 * 1024 {
        return Err(DesktopError::Update(
            "CLI archive executable exceeds the 100 MB safety limit".to_string(),
        ));
    }
    let path = staging.path().join("androidforge.exe");
    let mut file = std::fs::File::create(&path)?;
    std::io::copy(&mut entry, &mut file)?;
    drop(file);
    drop(entry);
    let version = CliRunner::new(path.clone()).version()?;
    if version != check.latest_version {
        return Err(DesktopError::Verification(format!(
            "downloaded CLI reports {version}, expected {}",
            check.latest_version
        )));
    }
    let staging = staging.keep();
    Ok(staging.join("androidforge.exe"))
}

/// Replace a CLI executable with a staged executable. The previous binary is
/// kept as a `.bak-<timestamp>` file and restored if validation fails.
pub fn install_staged_cli(staged: &Path, target: &Path) -> Result<PathBuf> {
    if !staged.is_file() {
        return Err(DesktopError::Update(format!(
            "staged executable does not exist: {}",
            staged.display()
        )));
    }
    let parent = target
        .parent()
        .ok_or_else(|| DesktopError::Update("target CLI path has no parent".to_string()))?;
    if !parent.is_dir() {
        return Err(DesktopError::Update(format!(
            "target directory does not exist: {}",
            parent.display()
        )));
    }
    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let backup = target.with_extension(format!("exe.bak-{stamp}"));
    let had_target = target.is_file();
    if had_target {
        std::fs::rename(target, &backup).map_err(|error| {
            DesktopError::Update(format!(
                "could not move the existing CLI to {}: {error}. Close any running AndroidForge operation and try again, or run the installer with sufficient permissions.",
                backup.display()
            ))
        })?;
    }
    std::fs::copy(staged, target).map_err(|error| {
        if had_target {
            let _ = std::fs::rename(&backup, target);
        }
        DesktopError::Update(format!("could not install the validated CLI: {error}"))
    })?;
    let version = match CliRunner::new(target.to_path_buf()).version() {
        Ok(version) => version,
        Err(error) => {
            let _ = std::fs::remove_file(target);
            if had_target {
                let _ = std::fs::rename(&backup, target);
            }
            return Err(DesktopError::Update(format!(
                "installed CLI failed validation and was rolled back: {error}"
            )));
        }
    };
    let _ = version;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semantic_versions() {
        assert!(version_is_newer("0.3.0", "0.2.0"));
        assert!(version_is_newer("v0.3.1", "0.3.0"));
        assert!(!version_is_newer("0.3.0", "0.3.0"));
        assert!(!version_is_newer("0.2.9", "0.3.0"));
        assert!(version_is_newer("1.0.0", "0.9.9"));
    }

    #[test]
    fn parses_published_checksums() {
        let text = "# release\n\
                    aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  androidforge-desktop-windows-x86_64.zip\n\
                    bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  *androidforge-cli-windows-x86_64.zip\n";
        assert_eq!(
            parse_checksums(text, CLI_ASSET).unwrap(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
    }

    #[test]
    fn rejects_missing_and_invalid_checksums() {
        assert!(parse_checksums("not-a-hash  other.zip", CLI_ASSET).is_err());
        assert!(parse_checksums("", CLI_ASSET).is_err());
        let invalid = format!("{}  {CLI_ASSET}\n", "z".repeat(64));
        assert!(parse_checksums(&invalid, CLI_ASSET).is_err());
    }

    #[test]
    fn failed_staged_update_restores_the_previous_cli() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("androidforge.exe");
        let staged = temp.path().join("staged.exe");
        std::fs::write(&target, b"previous-cli").unwrap();
        std::fs::write(&staged, b"not-a-valid-executable").unwrap();

        let result = install_staged_cli(&staged, &target);
        assert!(
            result.is_err(),
            "invalid staged binaries must not be installed"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"previous-cli");
    }

    #[test]
    fn expects_the_documented_release_asset_names() {
        assert_eq!(CLI_ASSET, "androidforge-cli-windows-x86_64.zip");
        assert_eq!(DESKTOP_ASSET, "androidforge-desktop-windows-x86_64.zip");
        assert_eq!(CHECKSUMS_ASSET, "SHA256SUMS.txt");
        assert_eq!(REPOSITORY, "JuggyBob/AndroidForge");
    }
}
