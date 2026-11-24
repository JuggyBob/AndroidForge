//! APK artifacts: single APKs, explicit split sets, split directories and
//! `.apks` archives.
//!
//! AndroidForge does not run bundletool or Gradle. An `.apks` archive is
//! extracted as provided; device-specific split selection remains the caller's
//! responsibility. A directory is treated as a split set when it contains more
//! than one APK.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::util::sha256_file;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::apk::{ApkInfo, ApkInspector};

const MAX_ARCHIVE_APKS: usize = 200;
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Reject an entry based on its declared size before reading any payload.
fn declared_size_exceeds_limit(declared: u64, total: u64) -> bool {
    declared > MAX_ARCHIVE_BYTES.saturating_sub(total)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApkArtifactKind {
    Single,
    SplitDirectory,
    SplitArchive,
}

impl ApkArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::SplitDirectory => "split_directory",
            Self::SplitArchive => "split_archive",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitApk {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApkArtifactSummary {
    pub source: String,
    pub kind: ApkArtifactKind,
    pub base: ApkInfo,
    pub splits: Vec<SplitApk>,
    pub split_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ApkArtifact {
    pub source: PathBuf,
    pub kind: ApkArtifactKind,
    pub base: ApkInfo,
    pub splits: Vec<SplitApk>,
    pub archive_sha256: Option<String>,
    pub warnings: Vec<String>,
    _keep: Option<Arc<TempDir>>,
}

impl ApkArtifact {
    pub async fn load(inspector: &ApkInspector, path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(ForgeError::new(
                ErrorCode::FileNotFound,
                format!("APK path {} was not found", path.display()),
            )
            .with_suggestion("Build the application or pass the correct path."));
        }
        if path.is_dir() {
            return Self::load_directory(inspector, path, None).await;
        }
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("apks"))
        {
            return Self::load_archive(inspector, path).await;
        }
        let base = inspector.inspect(path, false).await?;
        Ok(Self {
            source: path.to_path_buf(),
            kind: ApkArtifactKind::Single,
            base,
            splits: Vec::new(),
            archive_sha256: None,
            warnings: Vec::new(),
            _keep: None,
        })
    }

    async fn load_directory(
        inspector: &ApkInspector,
        directory: &Path,
        keep: Option<Arc<TempDir>>,
    ) -> Result<Self> {
        let mut warnings = Vec::new();
        let mut files = Vec::new();
        let mut layout_selected = false;
        let split_dir = directory.join("splits");
        let standalone_dir = directory.join("standalones");
        if split_dir.is_dir() {
            layout_selected = true;
            collect_apk_files(&split_dir, 0, &mut files)?;
            if !files.is_empty()
                && directory.join("standalones").is_dir()
                && dir_contains_apks(&standalone_dir)
            {
                warnings.push(
                    "Ignored standalones/ because this directory contains a splits/ set; standalone APKs are not part of split installs."
                        .to_string(),
                );
            }
        } else if standalone_dir.is_dir() {
            layout_selected = true;
            collect_apk_files(&standalone_dir, 0, &mut files)?;
            if files.len() > 1 {
                return Err(ForgeError::new(
                    ErrorCode::ApkInvalid,
                    format!(
                        "{} contains {} standalone APKs; AndroidForge will not guess which one targets this device",
                        standalone_dir.display(),
                        files.len()
                    ),
                )
                .with_suggestion(
                    "Pass the device-specific standalone APK explicitly instead of the directory.",
                ));
            }
        }
        if files.is_empty() && !layout_selected {
            collect_apk_files(directory, 0, &mut files)?;
        }
        files.sort();
        files.dedup();
        if files.is_empty() {
            return Err(ForgeError::new(
                ErrorCode::ApkInvalid,
                format!("no APK files were found in {}", directory.display()),
            )
            .with_suggestion(
                "Pass a base APK, an .apks archive, or a directory containing APKs.",
            ));
        }
        let mut inspected = Vec::new();
        for file in &files {
            inspected.push((file.clone(), inspector.inspect(file, false).await?));
        }
        let base_index = choose_base(&inspected);
        for (index, (path, info)) in inspected.iter().enumerate() {
            if index != base_index && info.launchable_activity.is_some() {
                return Err(ForgeError::new(
                    ErrorCode::ApkInvalid,
                    format!(
                        "{} contains multiple launchable APKs; {} is not a split",
                        directory.display(),
                        path.display()
                    ),
                )
                .with_suggestion(
                    "Pass one base APK with --split for each split, or use an .apks archive with a splits/ directory.",
                ));
            }
        }
        let package = inspected[base_index].1.package.clone().ok_or_else(|| {
            ForgeError::new(
                ErrorCode::ApkInvalid,
                format!(
                    "base APK {} does not declare a package",
                    files[base_index].display()
                ),
            )
        })?;
        for (path, info) in &inspected {
            if info.package.as_deref() != Some(package.as_str()) {
                return Err(ForgeError::new(
                    ErrorCode::ApkInvalid,
                    format!(
                        "APK {} declares package {:?}, but the base APK declares {package}",
                        path.display(),
                        info.package
                    ),
                )
                .with_suggestion(
                    "Install a split set whose APKs all belong to the same package.",
                ));
            }
        }
        let base_version_code = inspected[base_index].1.version_code;
        for (path, info) in &inspected {
            if let (Some(base), Some(other)) = (base_version_code, info.version_code)
                && base != other
            {
                return Err(ForgeError::new(
                    ErrorCode::ApkInvalid,
                    format!(
                        "APK {} declares versionCode {other}, but the base APK declares {base}",
                        path.display()
                    ),
                )
                .with_suggestion(
                    "Build the base APK and all splits in the same build before installing them together.",
                ));
            }
        }
        let mut splits = Vec::new();
        for (index, (path, info)) in inspected.iter().enumerate() {
            if index == base_index {
                continue;
            }
            splits.push(SplitApk {
                path: path.to_string_lossy().into_owned(),
                name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| format!("split-{index}.apk")),
                size_bytes: info.size_bytes,
                sha256: info.sha256.clone(),
                package: info.package.clone(),
                version_code: info.version_code,
            });
        }
        splits.sort_by(|left, right| left.name.cmp(&right.name));
        let kind = if splits.is_empty() {
            ApkArtifactKind::Single
        } else {
            ApkArtifactKind::SplitDirectory
        };
        if !splits.is_empty() {
            warnings.push(
                "Split APKs are installed exactly as provided; AndroidForge does not run bundletool or select device-specific splits."
                    .to_string(),
            );
        }
        Ok(Self {
            source: directory.to_path_buf(),
            kind,
            base: inspected[base_index].1.clone(),
            splits,
            archive_sha256: None,
            warnings,
            _keep: keep,
        })
    }

    async fn load_archive(inspector: &ApkInspector, archive: &Path) -> Result<Self> {
        let archive_sha256 = sha256_file(archive)?;
        let temp = tempfile::tempdir().map_err(|error| {
            ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!("could not create a temporary directory for the .apks archive: {error}"),
            )
        })?;
        let file = std::fs::File::open(archive).map_err(|error| {
            ForgeError::new(
                ErrorCode::ApkInvalid,
                format!("could not open {}: {error}", archive.display()),
            )
        })?;
        let mut zip = zip::ZipArchive::new(file).map_err(|error| {
            ForgeError::new(
                ErrorCode::ApkInvalid,
                format!(
                    "{} is not a readable .apks ZIP archive: {error}",
                    archive.display()
                ),
            )
        })?;
        if zip.len() > MAX_ARCHIVE_APKS * 4 {
            return Err(ForgeError::new(
                ErrorCode::ApkInvalid,
                ".apks archive contains too many entries",
            ));
        }
        let mut entry_names = Vec::new();
        for index in 0..zip.len() {
            let entry = zip.by_index(index).map_err(|error| {
                ForgeError::new(
                    ErrorCode::ApkInvalid,
                    format!("could not read .apks entry {index}: {error}"),
                )
            })?;
            if entry.is_dir() {
                continue;
            }
            let entry_name = entry
                .name()
                .map_err(|error| {
                    ForgeError::new(
                        ErrorCode::ApkInvalid,
                        format!("could not read .apks entry name: {error}"),
                    )
                })?
                .to_string();
            if entry_name.to_ascii_lowercase().ends_with(".apk") {
                entry_names.push(entry_name);
            }
        }
        let selection = select_archive_entries(&entry_names)?;
        let mut extracted = 0usize;
        let mut total = 0u64;
        let mut extracted_entries = Vec::new();
        for index in 0..zip.len() {
            let entry = zip.by_index(index).map_err(|error| {
                ForgeError::new(
                    ErrorCode::ApkInvalid,
                    format!("could not read .apks entry {index}: {error}"),
                )
            })?;
            let entry_name = entry
                .name()
                .map_err(|error| {
                    ForgeError::new(
                        ErrorCode::ApkInvalid,
                        format!("could not read .apks entry name: {error}"),
                    )
                })?
                .to_string();
            if !selection.entries.iter().any(|name| name == &entry_name) {
                continue;
            }
            let safe_name = sanitize_entry_name(&entry_name)?;
            extracted += 1;
            let target = temp.path().join(format!("{:03}-{safe_name}", extracted));
            extracted_entries.push((target.clone(), file_name_of_entry(&entry_name).to_string()));
            let declared = entry.size();
            let remaining = MAX_ARCHIVE_BYTES.saturating_sub(total);
            if declared_size_exceeds_limit(declared, total) {
                return Err(ForgeError::new(
                    ErrorCode::ApkInvalid,
                    ".apks archive exceeds the 2 GiB size limit",
                ));
            }
            let mut bytes = Vec::new();
            let mut limited = entry.take(remaining.saturating_add(1));
            limited.read_to_end(&mut bytes).map_err(|error| {
                ForgeError::new(
                    ErrorCode::ApkInvalid,
                    format!("could not extract {entry_name}: {error}"),
                )
            })?;
            if bytes.len() as u64 > remaining {
                return Err(ForgeError::new(
                    ErrorCode::ApkInvalid,
                    ".apks archive exceeds the 2 GiB size limit",
                ));
            }
            total = total.saturating_add(bytes.len() as u64);
            std::fs::write(&target, &bytes).map_err(|error| {
                ForgeError::new(
                    ErrorCode::ArtifactWriteFailed,
                    format!(
                        "could not write extracted APK {}: {error}",
                        target.display()
                    ),
                )
            })?;
        }
        if extracted == 0 {
            return Err(ForgeError::new(
                ErrorCode::ApkInvalid,
                format!("{} contains no usable APK entries", archive.display()),
            ));
        }
        let keep = Arc::new(temp);
        let mut artifact = Self::load_directory(inspector, keep.path(), Some(keep.clone())).await?;
        for split in &mut artifact.splits {
            if let Some((_, name)) = extracted_entries
                .iter()
                .find(|(target, _)| target.as_path() == Path::new(&split.path))
            {
                split.name = name.clone();
            }
        }
        artifact
            .splits
            .sort_by(|left, right| left.name.cmp(&right.name));
        artifact.source = archive.to_path_buf();
        artifact.kind = ApkArtifactKind::SplitArchive;
        artifact.archive_sha256 = Some(archive_sha256);
        if let Some(warning) = selection.warning {
            artifact.warnings.push(warning);
        }
        artifact.warnings.push(
            "The .apks archive was extracted as provided. AndroidForge does not perform bundletool device targeting."
                .to_string(),
        );
        Ok(artifact)
    }
    pub fn base_path(&self) -> PathBuf {
        PathBuf::from(&self.base.path)
    }

    pub fn split_paths(&self) -> Vec<PathBuf> {
        self.splits
            .iter()
            .map(|split| PathBuf::from(&split.path))
            .collect()
    }

    pub fn install_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.base_path()];
        paths.extend(self.split_paths());
        paths
    }

    pub fn split_count(&self) -> usize {
        self.splits.len()
    }

    pub fn summary(&self) -> ApkArtifactSummary {
        ApkArtifactSummary {
            source: self.source.to_string_lossy().into_owned(),
            kind: self.kind,
            base: self.base.clone(),
            splits: self.splits.clone(),
            split_count: self.splits.len(),
            archive_sha256: self.archive_sha256.clone(),
            warnings: self.warnings.clone(),
        }
    }
}

fn choose_base(inspected: &[(PathBuf, ApkInfo)]) -> usize {
    inspected
        .iter()
        .enumerate()
        .min_by(
            |(left_index, (left_path, left)), (right_index, (right_path, right))| {
                let score = |path: &Path, info: &ApkInfo| {
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_ascii_lowercase())
                        .unwrap_or_default();
                    let rank = if info.launchable_activity.is_some() {
                        0
                    } else if name.contains("base") && !name.contains("config") {
                        1
                    } else if name.contains("standalone") {
                        2
                    } else {
                        3
                    };
                    (rank, std::cmp::Reverse(info.size_bytes))
                };
                score(left_path, left)
                    .cmp(&score(right_path, right))
                    .then_with(|| left_index.cmp(right_index))
            },
        )
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn collect_apk_files(directory: &Path, depth: usize, files: &mut Vec<PathBuf>) -> Result<()> {
    if depth > 2 {
        return Ok(());
    }
    let entries = std::fs::read_dir(directory).map_err(|error| {
        ForgeError::new(
            ErrorCode::ApkInvalid,
            format!("could not read {}: {error}", directory.display()),
        )
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_apk_files(&path, depth + 1, files)?;
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("apk"))
        {
            files.push(path);
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ArchiveSelection {
    entries: Vec<String>,
    warning: Option<String>,
}

fn select_archive_entries(names: &[String]) -> Result<ArchiveSelection> {
    if names.is_empty() {
        return Err(ForgeError::new(
            ErrorCode::ApkInvalid,
            ".apks archive contains no APK entries",
        ));
    }
    let split_entries = names
        .iter()
        .filter(|name| has_path_component(name, "splits"))
        .cloned()
        .collect::<Vec<_>>();
    if !split_entries.is_empty() {
        let ignored = names.len().saturating_sub(split_entries.len());
        return Ok(ArchiveSelection {
            entries: split_entries,
            warning: (ignored > 0).then(|| {
                format!(
                    "Ignored {ignored} non-split APK entry(ies) such as standalones or universal APKs; only splits/ entries are installed."
                )
            }),
        });
    }
    if let Some(universal) = names
        .iter()
        .find(|name| file_name_of_entry(name).eq_ignore_ascii_case("universal.apk"))
    {
        let ignored = names.len().saturating_sub(1);
        return Ok(ArchiveSelection {
            entries: vec![universal.clone()],
            warning: (ignored > 0).then(|| {
                format!("Ignored {ignored} other APK entry(ies); the universal APK was selected.")
            }),
        });
    }
    let standalone_entries = names
        .iter()
        .filter(|name| has_path_component(name, "standalones"))
        .cloned()
        .collect::<Vec<_>>();
    if standalone_entries.len() > 1 {
        return Err(ForgeError::new(
            ErrorCode::ApkInvalid,
            format!(
                ".apks archive contains {} standalone APKs; AndroidForge will not guess which one targets this device",
                standalone_entries.len()
            ),
        )
        .with_suggestion(
            "Extract the archive and pass the device-specific standalone APK explicitly.",
        ));
    }
    if standalone_entries.len() == 1 {
        let ignored = names.len().saturating_sub(1);
        return Ok(ArchiveSelection {
            entries: standalone_entries,
            warning: (ignored > 0).then(|| {
                format!(
                    "Ignored {ignored} other APK entry(ies); the single standalone APK was selected."
                )
            }),
        });
    }
    if names.len() == 1 {
        return Ok(ArchiveSelection {
            entries: vec![names[0].clone()],
            warning: None,
        });
    }
    Err(ForgeError::new(
        ErrorCode::ApkInvalid,
        format!(
            ".apks archive contains {} APKs but no splits/ directory; AndroidForge will not guess which APKs form one installable set",
            names.len()
        ),
    )
    .with_suggestion(
        "Use an .apks archive produced from one device, or pass a split directory or base APK plus --split flags.",
    ))
}

fn has_path_component(name: &str, component: &str) -> bool {
    name.split(['/', '\\'])
        .any(|part| part.eq_ignore_ascii_case(component))
}

fn file_name_of_entry(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

fn dir_contains_apks(dir: &Path) -> bool {
    let mut files = Vec::new();
    collect_apk_files(dir, 0, &mut files).is_ok() && !files.is_empty()
}

fn sanitize_entry_name(name: &str) -> Result<String> {
    if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
        return Err(ForgeError::new(
            ErrorCode::ApkInvalid,
            format!(".apks entry '{name}' has an unsafe path"),
        ));
    }
    let name = name
        .replace(['/', '\\'], "_")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if name.to_ascii_lowercase().ends_with(".apk") && name.len() > 4 {
        Ok(name)
    } else {
        Ok(format!("{name}.apk"))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;

    /// Build a tiny .apks whose central directory declares an oversized entry.
    /// Only the 4-byte uncompressed-size field is patched, so the archive never
    /// contains the claimed payload.
    fn apks_with_declared_size(declared: u32) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer
            .start_file("splits/base-master.apk", options)
            .unwrap();
        writer.write_all(b"tiny payload").unwrap();
        let cursor = writer.finish().unwrap();
        let mut bytes = cursor.into_inner();

        let header = [0x50, 0x4b, 0x01, 0x02]; // PK\x01\x02 central directory
        let position = bytes
            .windows(4)
            .position(|window| window == header)
            .expect("central directory header");
        let size_at = position + 24;
        bytes[size_at..size_at + 4].copy_from_slice(&declared.to_le_bytes());
        bytes
    }

    #[test]
    fn archive_declared_size_guard_enforces_remaining_budget() {
        assert!(declared_size_exceeds_limit(MAX_ARCHIVE_BYTES + 1, 0));
        assert!(!declared_size_exceeds_limit(MAX_ARCHIVE_BYTES, 0));
        assert!(declared_size_exceeds_limit(2, MAX_ARCHIVE_BYTES - 1));
        assert!(!declared_size_exceeds_limit(1, MAX_ARCHIVE_BYTES - 1));
    }

    #[tokio::test]
    async fn apks_rejects_declared_size_above_limit_without_writing_payload() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("oversized.apks");
        std::fs::write(
            &archive,
            apks_with_declared_size((MAX_ARCHIVE_BYTES + 1) as u32),
        )
        .unwrap();
        let inspector = ApkInspector::new(None, None, None, false);

        let error = ApkArtifact::load_archive(&inspector, &archive)
            .await
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ApkInvalid);
        assert!(
            error.message().contains("2 GiB"),
            "unexpected error: {}",
            error.message()
        );
    }

    #[test]
    fn archive_names_cannot_escape_the_temp_directory() {
        assert!(sanitize_entry_name("../evil.apk").is_err());
        assert!(sanitize_entry_name("/absolute.apk").is_err());
        assert_eq!(
            sanitize_entry_name("splits/base-master.apk").unwrap(),
            "splits_base-master.apk"
        );
    }

    #[test]
    fn archive_selection_prefers_split_entries() {
        let names = vec![
            "splits/base-master.apk".to_string(),
            "splits/base-arm64_v8a.apk".to_string(),
            "standalones/standalone-arm64_v8a.apk".to_string(),
        ];
        let selection = select_archive_entries(&names).unwrap();
        assert_eq!(
            selection.entries,
            vec!["splits/base-master.apk", "splits/base-arm64_v8a.apk"]
        );
        assert!(
            selection
                .warning
                .expect("selection warning")
                .contains("Ignored 1")
        );
    }

    #[test]
    fn archive_selection_accepts_a_universal_apk() {
        let names = vec!["universal.apk".to_string()];
        let selection = select_archive_entries(&names).unwrap();
        assert_eq!(selection.entries, vec!["universal.apk"]);
        assert!(selection.warning.is_none());
    }

    #[test]
    fn archive_selection_rejects_ambiguous_standalones() {
        let names = vec![
            "standalones/standalone-arm64_v8a.apk".to_string(),
            "standalones/standalone-x86_64.apk".to_string(),
        ];
        let error = select_archive_entries(&names).unwrap_err();
        assert!(error.message().contains("will not guess"));
    }

    #[test]
    fn archive_selection_rejects_unknown_multi_apk_layouts() {
        let names = vec!["first.apk".to_string(), "second.apk".to_string()];
        let error = select_archive_entries(&names).unwrap_err();
        assert!(error.message().contains("no splits/ directory"));
    }

    #[test]
    fn base_selection_prefers_launchable_apk() {
        let base = ApkInfo {
            path: "base.apk".to_string(),
            size_bytes: 10,
            sha256: "a".to_string(),
            package: Some("com.example".to_string()),
            version_name: None,
            version_code: None,
            min_sdk: None,
            target_sdk: None,
            label: None,
            launchable_activity: Some("com.example.Main".to_string()),
            architectures: Vec::new(),
            signature: Default::default(),
            notes: Vec::new(),
        };
        let mut split = base.clone();
        split.launchable_activity = None;
        split.size_bytes = 100;
        let index = choose_base(&[
            (PathBuf::from("split.apk"), split),
            (PathBuf::from("base.apk"), base),
        ]);
        assert_eq!(index, 1);
    }
}
