//! APK identity comparison.
//!
//! Comparison is deliberately explicit about `same`, `different` and
//! `unknown`. Version strings alone never prove byte equality.

use std::path::Path;
use std::time::Duration;

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::util::{normalize_fingerprint, sha256_file};
use serde::{Deserialize, Serialize};

use crate::apk::{ApkInfo, ApkInspector};
use crate::artifact::SplitApk;
use crate::install::installed_package;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldStatus {
    Same,
    Different,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldComparison {
    pub field: String,
    pub status: FieldStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Equivalence {
    Same,
    Different,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSplit {
    pub remote_path: String,
    pub name: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledApkIdentity {
    pub package: String,
    /// Remote APK paths reported by `pm path`; the first entry is the base APK.
    pub remote_paths: Vec<String>,
    /// Remote path of the base APK, also used as `base_apk.path`.
    pub base_remote_path: String,
    pub base_apk: ApkInfo,
    pub split_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub splits: Vec<InstalledSplit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApkDiff {
    pub schema_version: u32,
    pub candidate: ApkInfo,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_splits: Vec<SplitApk>,
    pub installed: InstalledApkIdentity,
    pub fields: Vec<FieldComparison>,
    pub overall: Equivalence,
    pub summary: String,
    /// True only when package, version, base APK bytes and split shape are all
    /// proven equivalent.
    pub safe_to_skip_install: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub const APK_DIFF_SCHEMA_VERSION: u32 = 1;

/// Pull the installed base APK and inspect it. Split APKs are listed but not
/// pulled; a split installation makes full byte equivalence unknown.
pub async fn inspect_installed(
    adb: &AdbClient,
    inspector: &ApkInspector,
    serial: &str,
    package: &str,
) -> Result<Option<InstalledApkIdentity>> {
    // Some vendor builds (observed on MIUI) exit non-zero for
    // `pm path <missing-package>`. Treat any failed lookup as "not installed"
    // so callers can report a clean PackageNotFound instead of an opaque ADB
    // failure. `installed_package` already uses the same policy.
    let Ok(paths) = adb
        .shell_args(
            serial,
            &["pm".to_string(), "path".to_string(), package.to_string()],
            Some(Duration::from_secs(15)),
            None,
        )
        .await
    else {
        return Ok(None);
    };
    let remote_paths = paths
        .stdout_text()
        .lines()
        .filter_map(|line| line.trim().strip_prefix("package:"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let Some(base_remote_path) = remote_paths.first().cloned() else {
        return Ok(None);
    };
    let temp = tempfile::tempdir().map_err(|error| {
        ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            format!("could not create a temporary directory for the installed APK: {error}"),
        )
    })?;
    let local = temp.path().join("installed-base.apk");
    adb.pull(serial, &base_remote_path, &local).await?;
    let sha256 = sha256_file(&local)?;
    let mut base_apk = inspector.inspect(&local, false).await?;
    // The temporary file is about to disappear; expose the device path instead.
    base_apk.path = base_remote_path.clone();
    base_apk.sha256 = sha256;

    let mut splits = Vec::new();
    for (index, remote_path) in remote_paths.iter().skip(1).enumerate() {
        let local = temp.path().join(format!("installed-split-{index:02}.apk"));
        adb.pull(serial, remote_path, &local).await?;
        let size_bytes = std::fs::metadata(&local)
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        splits.push(InstalledSplit {
            remote_path: remote_path.clone(),
            name: Path::new(remote_path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("split-{index}.apk")),
            size_bytes,
            sha256: sha256_file(&local)?,
        });
    }

    let package_info = installed_package(adb, serial, package).await?;
    let mut warnings = Vec::new();
    if !splits.is_empty() {
        warnings.push(format!(
            "The installed package uses {} split APK(s); split hashes are included when comparing a split set.",
            splits.len()
        ));
    }
    Ok(Some(InstalledApkIdentity {
        package: package.to_string(),
        split_count: remote_paths.len().saturating_sub(1),
        remote_paths,
        base_remote_path,
        version_name: package_info
            .as_ref()
            .and_then(|info| info.version_name.clone())
            .or_else(|| base_apk.version_name.clone()),
        version_code: package_info
            .as_ref()
            .and_then(|info| info.version_code)
            .or(base_apk.version_code),
        data_dir: package_info.as_ref().and_then(|info| info.data_dir.clone()),
        base_apk,
        splits,
        warnings,
    }))
}

pub fn compare(
    candidate: &ApkInfo,
    candidate_splits: &[SplitApk],
    installed: &InstalledApkIdentity,
) -> ApkDiff {
    let mut fields = vec![
        compare_field(
            "package",
            candidate.package.clone(),
            Some(installed.package.clone()),
        ),
        compare_field(
            "version_name",
            candidate.version_name.clone(),
            installed
                .version_name
                .clone()
                .or_else(|| installed.base_apk.version_name.clone()),
        ),
        compare_field(
            "version_code",
            candidate.version_code.map(|value| value.to_string()),
            installed
                .version_code
                .or(installed.base_apk.version_code)
                .map(|value| value.to_string()),
        ),
        compare_field(
            "apk_sha256",
            Some(candidate.sha256.clone()),
            Some(installed.base_apk.sha256.clone()),
        ),
        compare_field(
            "cert_sha256",
            candidate
                .signature
                .cert_sha256
                .as_deref()
                .and_then(normalize_fingerprint),
            installed
                .base_apk
                .signature
                .cert_sha256
                .as_deref()
                .and_then(normalize_fingerprint),
        ),
        compare_field(
            "min_sdk",
            candidate.min_sdk.map(|value| value.to_string()),
            installed.base_apk.min_sdk.map(|value| value.to_string()),
        ),
        compare_field(
            "target_sdk",
            candidate.target_sdk.map(|value| value.to_string()),
            installed.base_apk.target_sdk.map(|value| value.to_string()),
        ),
        compare_field(
            "launchable_activity",
            candidate.launchable_activity.clone(),
            installed.base_apk.launchable_activity.clone(),
        ),
    ];
    let split_status = compare_split_sets(candidate_splits, &installed.splits);
    fields.push(FieldComparison {
        field: "split_apks".to_string(),
        status: split_status,
        candidate: Some(format!(
            "{} split(s): {}",
            candidate_splits.len(),
            candidate_splits
                .iter()
                .map(|split| split.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        installed: Some(format!(
            "{} split(s): {}",
            installed.splits.len(),
            installed
                .splits
                .iter()
                .map(|split| split.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        note: Some(
            "Split sets are compared by their sorted SHA-256 hashes; names are not identities."
                .to_string(),
        ),
    });

    let any_different = fields
        .iter()
        .any(|field| field.status == FieldStatus::Different);
    let required_unknown = fields.iter().any(|field| {
        matches!(
            field.field.as_str(),
            "apk_sha256" | "version_code" | "split_apks"
        ) && field.status == FieldStatus::Unknown
    });
    let overall = if any_different {
        Equivalence::Different
    } else if required_unknown {
        Equivalence::Unknown
    } else {
        Equivalence::Same
    };
    let safe_to_skip_install = overall == Equivalence::Same
        && field_is(&fields, "package", FieldStatus::Same)
        && field_is(&fields, "version_code", FieldStatus::Same)
        && field_is(&fields, "apk_sha256", FieldStatus::Same)
        && field_is(&fields, "split_apks", FieldStatus::Same);
    let mut warnings = installed.warnings.clone();
    if split_status != FieldStatus::Same {
        warnings.push(
            "The candidate and installed split sets differ or cannot be proven identical; AndroidForge will not skip installation."
                .to_string(),
        );
    }
    if candidate_splits.is_empty() && !installed.splits.is_empty() {
        warnings.push(
            "The installed package has splits but the candidate is a single APK; installing it may replace the split installation."
                .to_string(),
        );
    }
    let differing = fields
        .iter()
        .filter(|field| field.status == FieldStatus::Different)
        .map(|field| field.field.as_str())
        .collect::<Vec<_>>();
    let unknown = fields
        .iter()
        .filter(|field| field.status == FieldStatus::Unknown)
        .map(|field| field.field.as_str())
        .collect::<Vec<_>>();
    let summary = match overall {
        Equivalence::Same => format!(
            "Candidate APK is equivalent to installed {} ({}).",
            installed.package,
            installed
                .version_name
                .as_deref()
                .unwrap_or("version unknown")
        ),
        Equivalence::Different => format!(
            "Candidate APK differs from installed {}: {}.",
            installed.package,
            differing.join(", ")
        ),
        Equivalence::Unknown => format!(
            "Candidate APK cannot be proven equivalent to installed {}; unknown: {}.",
            installed.package,
            if unknown.is_empty() {
                "split identity".to_string()
            } else {
                unknown.join(", ")
            }
        ),
    };
    ApkDiff {
        schema_version: APK_DIFF_SCHEMA_VERSION,
        candidate: candidate.clone(),
        candidate_splits: candidate_splits.to_vec(),
        installed: installed.clone(),
        fields,
        overall,
        summary,
        safe_to_skip_install,
        warnings,
    }
}

fn compare_split_sets(candidate: &[SplitApk], installed: &[InstalledSplit]) -> FieldStatus {
    if candidate.is_empty() && installed.is_empty() {
        return FieldStatus::Same;
    }
    if candidate.len() != installed.len() {
        return FieldStatus::Different;
    }
    let mut candidate_hashes = candidate
        .iter()
        .map(|split| split.sha256.as_str())
        .collect::<Vec<_>>();
    let mut installed_hashes = installed
        .iter()
        .map(|split| split.sha256.as_str())
        .collect::<Vec<_>>();
    candidate_hashes.sort_unstable();
    installed_hashes.sort_unstable();
    if candidate_hashes == installed_hashes {
        FieldStatus::Same
    } else {
        FieldStatus::Different
    }
}

fn field_is(fields: &[FieldComparison], name: &str, status: FieldStatus) -> bool {
    fields
        .iter()
        .find(|field| field.field == name)
        .is_some_and(|field| field.status == status)
}

fn compare_field(
    name: &str,
    candidate: Option<String>,
    installed: Option<String>,
) -> FieldComparison {
    let candidate = candidate.filter(|value| !value.trim().is_empty());
    let installed = installed.filter(|value| !value.trim().is_empty());
    let status = match (&candidate, &installed) {
        (Some(candidate), Some(installed)) if candidate == installed => FieldStatus::Same,
        (Some(_), Some(_)) => FieldStatus::Different,
        _ => FieldStatus::Unknown,
    };
    FieldComparison {
        field: name.to_string(),
        status,
        candidate,
        installed,
        note: match status {
            FieldStatus::Unknown => Some("the value is unavailable on one side".to_string()),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apk::ApkSignatureStatus;

    fn apk(version_code: i64, sha: &str, cert: Option<&str>) -> ApkInfo {
        ApkInfo {
            path: "C:/build/app-debug.apk".to_string(),
            size_bytes: 100,
            sha256: sha.to_string(),
            package: Some("com.example".to_string()),
            version_name: Some("1.0".to_string()),
            version_code: Some(version_code),
            min_sdk: Some(24),
            target_sdk: Some(35),
            label: None,
            launchable_activity: Some("com.example.Main".to_string()),
            architectures: Vec::new(),
            signature: ApkSignatureStatus {
                checked: true,
                signed: Some(true),
                verified: Some(true),
                cert_sha256: cert.map(str::to_string),
                ..ApkSignatureStatus::default()
            },
            notes: Vec::new(),
        }
    }

    fn installed(candidate: &ApkInfo, split_hashes: &[&str]) -> InstalledApkIdentity {
        InstalledApkIdentity {
            package: "com.example".to_string(),
            remote_paths: std::iter::once("/data/app/base.apk".to_string())
                .chain((0..split_hashes.len()).map(|index| format!("/data/app/split-{index}.apk")))
                .collect(),
            base_remote_path: "/data/app/base.apk".to_string(),
            base_apk: candidate.clone(),
            split_count: split_hashes.len(),
            splits: split_hashes
                .iter()
                .enumerate()
                .map(|(index, hash)| InstalledSplit {
                    remote_path: format!("/data/app/split-{index}.apk"),
                    name: format!("split-{index}.apk"),
                    size_bytes: 10,
                    sha256: (*hash).to_string(),
                })
                .collect(),
            version_name: candidate.version_name.clone(),
            version_code: candidate.version_code,
            data_dir: None,
            warnings: Vec::new(),
        }
    }

    fn candidate_splits(hashes: &[&str]) -> Vec<SplitApk> {
        hashes
            .iter()
            .enumerate()
            .map(|(index, hash)| SplitApk {
                path: format!("C:/build/split-{index}.apk"),
                name: format!("split-{index}.apk"),
                size_bytes: 10,
                sha256: (*hash).to_string(),
                package: Some("com.example".to_string()),
                version_code: Some(42),
            })
            .collect()
    }

    #[test]
    fn identical_apks_are_safe_to_skip() {
        let candidate = apk(42, "abc", Some("AA:BB"));
        let diff = compare(&candidate, &[], &installed(&candidate, &[]));
        assert_eq!(diff.overall, Equivalence::Same);
        assert!(diff.safe_to_skip_install);
    }

    #[test]
    fn same_version_different_hash_is_different() {
        let candidate = apk(42, "abc", Some("AA:BB"));
        let mut installed_apk = candidate.clone();
        installed_apk.sha256 = "def".to_string();
        let diff = compare(&candidate, &[], &installed(&installed_apk, &[]));
        assert_eq!(diff.overall, Equivalence::Different);
        assert!(!diff.safe_to_skip_install);
    }

    #[test]
    fn missing_hash_is_unknown_and_never_safe_to_skip() {
        let candidate = apk(42, "", Some("AA:BB"));
        let diff = compare(&candidate, &[], &installed(&candidate, &[]));
        assert_eq!(diff.overall, Equivalence::Unknown);
        assert!(!diff.safe_to_skip_install);
    }

    #[test]
    fn split_installations_are_different_when_candidate_is_single() {
        let candidate = apk(42, "abc", Some("AA:BB"));
        let diff = compare(&candidate, &[], &installed(&candidate, &["s1", "s2"]));
        assert_eq!(diff.overall, Equivalence::Different);
        assert!(!diff.safe_to_skip_install);
        assert!(!diff.warnings.is_empty());
    }

    #[test]
    fn matching_split_sets_are_safe_to_skip() {
        let candidate = apk(42, "abc", Some("AA:BB"));
        let splits = candidate_splits(&["s1", "s2"]);
        let diff = compare(&candidate, &splits, &installed(&candidate, &["s2", "s1"]));
        assert_eq!(diff.overall, Equivalence::Same);
        assert!(diff.safe_to_skip_install);
    }

    #[test]
    fn changed_split_hashes_are_different() {
        let candidate = apk(42, "abc", Some("AA:BB"));
        let splits = candidate_splits(&["s1", "s2"]);
        let diff = compare(
            &candidate,
            &splits,
            &installed(&candidate, &["s1", "changed"]),
        );
        assert_eq!(diff.overall, Equivalence::Different);
        assert!(!diff.safe_to_skip_install);
    }

    #[test]
    fn missing_certificate_is_unknown_but_hash_can_still_prove_same() {
        let candidate = apk(42, "abc", None);
        let diff = compare(&candidate, &[], &installed(&candidate, &[]));
        assert_eq!(diff.overall, Equivalence::Same);
        assert!(diff.safe_to_skip_install);
        assert!(
            diff.fields
                .iter()
                .any(|field| field.field == "cert_sha256" && field.status == FieldStatus::Unknown)
        );
    }
}
