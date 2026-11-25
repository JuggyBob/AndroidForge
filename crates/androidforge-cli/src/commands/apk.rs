//! Deterministic APK output resolution and APK comparison commands.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use androidforge_app::{
    ApkArtifact, ApkArtifactKind, ApkInfo, ApkInspector, SplitApk, compare, inspect_installed,
};
use androidforge_core::config::ForgeConfigFile;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use serde::{Deserialize, Serialize};

use crate::cli::{ApkDiffArgs, ApkResolveArgs};
use crate::commands::common::success;
use crate::context::CliContext;

const MAX_WALK_DEPTH: usize = 10;
const MAX_WALK_FILES: usize = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedApk {
    pub path: String,
    pub relative_path: String,
    pub variant: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    pub rule: String,
    pub size_bytes: u64,
    pub modified_at: String,
    pub sha256: String,
    #[serde(default)]
    pub artifact_kind: String,
    #[serde(default)]
    pub split_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub splits: Vec<SplitApk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectedCandidate {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApkResolution {
    pub schema_version: u32,
    pub variant: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    pub patterns: Vec<String>,
    pub selected: ResolvedApk,
    #[serde(default)]
    pub rejected: Vec<RejectedCandidate>,
    pub considered: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub async fn resolve_command(ctx: &CliContext, args: &ApkResolveArgs) -> Result<CommandSuccess> {
    let variant = args
        .variant
        .clone()
        .or_else(|| ctx.config.file.build.default_variant.clone())
        .ok_or_else(|| {
            ForgeError::invalid_argument("an APK variant is required")
                .with_suggestion("Pass --variant debug or set [build].default_variant.")
                .with_suggestion(
                    "Configure outputs under [build.variants] in .androidforge/project.toml.",
                )
        })?;
    let module = args
        .module
        .clone()
        .or_else(|| ctx.config.file.build.default_module.clone());
    let resolution = resolve_variant(ctx, &variant, module.as_deref()).await?;
    let human = format!(
        "Resolved {} {}\npath: {}\npackage: {}\nversion: {}\nSHA-256: {}\nartifact: {}{}\nrule: {}\n",
        resolution.variant,
        resolution
            .module
            .as_deref()
            .map(|module| format!("({module})"))
            .unwrap_or_default(),
        resolution.selected.path,
        resolution.selected.package.as_deref().unwrap_or("-"),
        resolution.selected.version_name.as_deref().unwrap_or("-"),
        resolution.selected.sha256,
        resolution.selected.artifact_kind,
        if resolution.selected.split_count > 0 {
            format!(" ({} split(s))", resolution.selected.split_count)
        } else {
            String::new()
        },
        resolution.selected.rule,
    );
    Ok(success(
        "apk.resolve",
        serde_json::to_value(&resolution)?,
        human,
    ))
}

/// Resolve the newest valid APK for a variant. Read-only: no Gradle or build
/// script is executed.
pub async fn resolve_variant(
    ctx: &CliContext,
    variant: &str,
    module: Option<&str>,
) -> Result<ApkResolution> {
    resolve_variant_with(
        &ctx.config.file,
        &ctx.paths.project_root,
        &ctx.inspector(),
        variant,
        module,
    )
    .await
}

/// CLI-independent core of [`resolve_variant`]. The host-side build engine
/// calls this directly so a built APK is resolved with exactly the same
/// deterministic patterns, rejection rules and inspector as `apk resolve`.
pub(crate) async fn resolve_variant_with(
    config: &ForgeConfigFile,
    root: &Path,
    inspector: &ApkInspector,
    variant: &str,
    module: Option<&str>,
) -> Result<ApkResolution> {
    let build = &config.build;
    let configured = find_variant_config(&build.variants, variant);
    let mut explicit_patterns = configured
        .map(|(_, patterns)| patterns.clone())
        .unwrap_or_default();
    explicit_patterns.retain(|pattern| !pattern.trim().is_empty());
    let mut warnings = Vec::new();
    if configured.is_none() {
        warnings.push(format!(
            "Variant '{variant}' is not configured in [build.variants]; using conventional Gradle output locations."
        ));
    }
    let conventional = conventional_patterns(variant);
    let mut patterns = explicit_patterns.clone();
    if patterns.is_empty() && build.conventional_fallback {
        patterns.extend(conventional.clone());
    } else if patterns.is_empty() {
        return Err(ForgeError::invalid_argument(format!(
            "no APK output patterns are configured for variant '{variant}' and conventional fallback is disabled"
        ))
        .with_suggestion("Add [build.variants] to .androidforge/project.toml.")
        .with_suggestion("Or set [build].conventional_fallback = true."));
    }
    if patterns.is_empty() {
        return Err(ForgeError::invalid_argument(format!(
            "no APK output patterns are configured for variant '{variant}'"
        ))
        .with_suggestion("Add [build.variants] to .androidforge/project.toml."));
    }

    let mut files = BTreeSet::new();
    collect_files(root, 0, &mut files)?;
    let mut matched: BTreeMap<PathBuf, String> = BTreeMap::new();
    for file in &files {
        let Ok(relative) = file.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if let Some(module) = module
            && !relative_in_module(&relative, module)
        {
            continue;
        }
        for pattern in &patterns {
            if pattern_matches(&relative, pattern) {
                matched
                    .entry(file.clone())
                    .or_insert_with(|| pattern.clone());
                break;
            }
        }
    }
    if matched.is_empty() {
        let mut error = ForgeError::new(
            ErrorCode::InvalidArgument,
            format!("no APK matched variant '{variant}' in {}", root.display()),
        )
        .with_context("patterns", serde_json::json!(patterns))
        .with_context("module", serde_json::json!(module))
        .with_suggestion("Build the application first, or correct [build.variants] patterns.")
        .with_suggestion("APK resolution never runs Gradle automatically.");
        if configured.is_none() {
            error = error.with_suggestion(
                "Add [build.variants] if this project does not use the conventional app module layout.",
            );
        }
        return Err(error);
    }

    let mut valid: Vec<(ResolvedApk, ApkInfo, SystemTime)> = Vec::new();
    let mut rejected = Vec::new();
    let mut inspected = 0usize;
    for (path, rule) in matched {
        if inspected >= build.max_candidates.max(1) {
            rejected.push(RejectedCandidate {
                path: path.to_string_lossy().into_owned(),
                rule: Some(rule),
                reason: format!(
                    "candidate limit of {} reached; later candidates were not inspected",
                    build.max_candidates
                ),
            });
            continue;
        }
        inspected += 1;
        let modified = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .map_err(|error| {
                ForgeError::new(
                    ErrorCode::ApkInvalid,
                    format!("could not read {}: {error}", path.display()),
                )
            })?;
        match ApkArtifact::load(inspector, &path).await {
            Ok(artifact) => {
                let info = artifact.base.clone();
                let relative = path
                    .strip_prefix(root)
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| path.to_string_lossy().into_owned());
                let selected = ResolvedApk {
                    path: path.to_string_lossy().into_owned(),
                    relative_path: relative,
                    variant: variant.to_string(),
                    module: module.map(str::to_string),
                    rule,
                    size_bytes: std::fs::metadata(&path)
                        .map(|metadata| metadata.len())
                        .unwrap_or(info.size_bytes),
                    modified_at: system_time_rfc3339(modified),
                    sha256: info.sha256.clone(),
                    artifact_kind: artifact.kind.as_str().to_string(),
                    split_count: artifact.split_count(),
                    splits: artifact.splits.clone(),
                    archive_sha256: artifact.archive_sha256.clone(),
                    package: info.package.clone(),
                    version_name: info.version_name.clone(),
                    version_code: info.version_code,
                    cert_sha256: info.signature.cert_sha256.clone(),
                };
                valid.push((selected, info, modified));
            }
            Err(error) => rejected.push(RejectedCandidate {
                path: path.to_string_lossy().into_owned(),
                rule: Some(rule),
                reason: error.message().to_string(),
            }),
        }
    }
    if valid.is_empty() {
        return Err(ForgeError::new(
            ErrorCode::ApkInvalid,
            format!(
                "no valid APK could be inspected for variant '{variant}' ({} candidate(s) rejected)",
                rejected.len()
            ),
        )
        .with_context(
            "rejected",
            serde_json::to_value(&rejected).unwrap_or(serde_json::Value::Null),
        )
        .with_suggestion("Inspect the rejected candidates and rebuild the APK."));
    }
    valid.sort_by(|left, right| {
        right
            .2
            .cmp(&left.2)
            .then_with(|| left.0.relative_path.cmp(&right.0.relative_path))
    });
    if valid.len() > 1 && valid[0].2 == valid[1].2 && valid[0].0.sha256 != valid[1].0.sha256 {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            format!(
                "two APK candidates for variant '{variant}' have the same timestamp but different content"
            ),
        )
        .with_context(
            "candidates",
            serde_json::json!([
                {
                    "path": valid[0].0.path,
                    "sha256": valid[0].0.sha256,
                    "modified_at": valid[0].0.modified_at,
                },
                {
                    "path": valid[1].0.path,
                    "sha256": valid[1].0.sha256,
                    "modified_at": valid[1].0.modified_at,
                }
            ]),
        )
        .with_suggestion("Configure one explicit output path for this variant.")
        .with_suggestion("Or remove the stale duplicate build output."));
    }
    if valid.len() > 1 && valid[0].0.sha256 == valid[1].0.sha256 {
        warnings.push(format!(
            "Multiple copies of the same APK hash were found; selected {} deterministically.",
            valid[0].0.relative_path
        ));
    }
    let mut selected = valid.remove(0).0;
    selected.variant = variant.to_string();
    selected.module = module.map(str::to_string);
    if selected.artifact_kind == "split_archive" && !selected.splits.is_empty() {
        warnings.push(
            "Resolved split paths are temporary extraction paths; pass the selected .apks path to install commands so the archive is re-opened."
                .to_string(),
        );
    }
    for (candidate, _, modified) in &valid {
        rejected.push(RejectedCandidate {
            path: candidate.path.clone(),
            rule: Some(candidate.rule.clone()),
            reason: if candidate.sha256 == selected.sha256 {
                format!(
                    "duplicate content; older or later tie-break (modified {})",
                    system_time_rfc3339(*modified)
                )
            } else {
                "a newer valid candidate exists".to_string()
            },
        });
    }
    let considered = inspected;
    Ok(ApkResolution {
        schema_version: 1,
        variant: variant.to_string(),
        module: module.map(str::to_string),
        patterns,
        selected,
        rejected,
        considered,
        warnings,
    })
}

fn find_variant_config<'a>(
    variants: &'a BTreeMap<String, Vec<String>>,
    variant: &str,
) -> Option<(&'a String, &'a Vec<String>)> {
    if let Some(found) = variants.get_key_value(variant) {
        return Some(found);
    }
    let mut matches = variants
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case(variant));
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first)
}

fn conventional_patterns(variant: &str) -> Vec<String> {
    let mut patterns = vec![
        format!("app/build/outputs/apk/{variant}/*.apk"),
        format!("*/build/outputs/apk/{variant}/*.apk"),
    ];
    if let Some((flavor, build_type)) = split_variant(variant) {
        patterns.push(format!("app/build/outputs/apk/{flavor}/{build_type}/*.apk"));
        patterns.push(format!("*/build/outputs/apk/{flavor}/{build_type}/*.apk"));
    }
    patterns.sort();
    patterns.dedup();
    patterns
}

fn split_variant(variant: &str) -> Option<(String, String)> {
    let index = variant.find(|ch: char| ch.is_ascii_uppercase())?;
    if index == 0 || index >= variant.len() - 1 {
        return None;
    }
    Some((
        variant[..index].to_ascii_lowercase(),
        variant[index..].to_ascii_lowercase(),
    ))
}

fn relative_in_module(relative: &str, module: &str) -> bool {
    let module = module.trim_matches(['/', '\\']);
    if module.is_empty() {
        return true;
    }
    relative == module
        || relative.starts_with(&format!("{module}/"))
        || relative.contains(&format!("/{module}/"))
}

fn collect_files(root: &Path, depth: usize, files: &mut BTreeSet<PathBuf>) -> Result<()> {
    if depth > MAX_WALK_DEPTH || files.len() >= MAX_WALK_FILES {
        return Ok(());
    }
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        if files.len() >= MAX_WALK_FILES {
            break;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        if file_type.is_dir() {
            if skip_directory(&name) {
                continue;
            }
            collect_files(&path, depth + 1, files)?;
        } else if file_type.is_file() {
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".apk") || lower.ends_with(".apks") {
                files.insert(path);
            }
        }
    }
    Ok(())
}

fn skip_directory(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".gradle"
            | ".idea"
            | ".androidforge"
            | ".artifacts"
            | "target"
            | "node_modules"
            | "dist"
            | "state"
            | "intermediates"
            | "tmp"
            | "caches"
    )
}

fn pattern_matches(relative: &str, pattern: &str) -> bool {
    let path = relative
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let pattern = pattern
        .replace('\\', "/")
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    segments_match(&path, &pattern)
}

fn segments_match(path: &[&str], pattern: &[String]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((segment, rest)) if segment == "**" => {
            (0..=path.len()).any(|index| segments_match(&path[index..], rest))
        }
        Some((segment, rest)) => {
            let Some((candidate, remaining)) = path.split_first() else {
                return false;
            };
            wildcard_match(candidate, segment) && segments_match(remaining, rest)
        }
    }
}

fn wildcard_match(value: &str, pattern: &str) -> bool {
    let value = value.chars().collect::<Vec<_>>();
    let pattern = pattern.chars().collect::<Vec<_>>();
    let (mut value_index, mut pattern_index) = (0usize, 0usize);
    let (mut star, mut star_value) = (None, 0usize);
    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == '?' || pattern[pattern_index] == value[value_index])
        {
            value_index += 1;
            pattern_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == '*' {
            star = Some(pattern_index);
            star_value = value_index;
            pattern_index += 1;
        } else if let Some(star_index) = star {
            pattern_index = star_index + 1;
            star_value += 1;
            value_index = star_value;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

fn system_time_rfc3339(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339()
}

pub(crate) async fn load_artifact_with_splits(
    ctx: &CliContext,
    path: &Path,
    explicit_splits: &[PathBuf],
) -> Result<ApkArtifact> {
    let inspector = ctx.inspector();
    let mut artifact = ApkArtifact::load(&inspector, path).await?;
    if explicit_splits.is_empty() {
        return Ok(artifact);
    }
    let package = artifact.base.package.clone().ok_or_else(|| {
        ForgeError::new(
            ErrorCode::ApkInvalid,
            format!("base APK {} does not declare a package", path.display()),
        )
    })?;
    for split in explicit_splits {
        let split = if split.is_absolute() {
            split.clone()
        } else {
            ctx.paths.cwd.join(split)
        };
        let info = inspector.inspect(&split, false).await?;
        if info.package.as_deref() != Some(package.as_str()) {
            return Err(ForgeError::new(
                ErrorCode::ApkInvalid,
                format!(
                    "split APK {} declares package {:?}, but the base APK declares {package}",
                    split.display(),
                    info.package
                ),
            ));
        }
        if let (Some(base), Some(other)) = (artifact.base.version_code, info.version_code)
            && base != other
        {
            return Err(ForgeError::new(
                ErrorCode::ApkInvalid,
                format!(
                    "split APK {} declares versionCode {other}, but the base APK declares {base}",
                    split.display()
                ),
            )
            .with_suggestion(
                "Build the base APK and all splits in the same build before installing them together.",
            ));
        }
        artifact.splits.push(SplitApk {
            path: split.to_string_lossy().into_owned(),
            name: split
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "split.apk".to_string()),
            size_bytes: info.size_bytes,
            sha256: info.sha256,
            package: info.package,
            version_code: info.version_code,
        });
    }
    artifact
        .splits
        .sort_by(|left, right| left.name.cmp(&right.name));
    artifact.kind = ApkArtifactKind::SplitDirectory;
    artifact
        .warnings
        .push("Explicit --split APKs were added to the candidate set.".to_string());
    Ok(artifact)
}

pub async fn diff_command(ctx: &CliContext, args: &ApkDiffArgs) -> Result<CommandSuccess> {
    let (installed_spec, apk_spec) = if let Some(package) = args.target.strip_prefix("installed:") {
        (package.to_string(), args.apk.clone())
    } else if let Some(package) = args.apk.strip_prefix("installed:") {
        (package.to_string(), args.target.clone())
    } else {
        return Err(ForgeError::invalid_argument(
            "one side of `apk diff` must be installed:<package>",
        )
        .with_suggestion(
            "Example: androidforge apk diff installed:com.example.app .\\app-debug.apk --session <ID> --json",
        ));
    };
    if installed_spec.trim().is_empty() {
        return Err(ForgeError::invalid_argument(
            "installed:<package> requires a package name",
        ));
    }
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let apk_path = if Path::new(&apk_spec).is_absolute() {
        PathBuf::from(&apk_spec)
    } else {
        ctx.paths.cwd.join(&apk_spec)
    };
    let inspector = ctx.inspector();
    let artifact = load_artifact_with_splits(ctx, &apk_path, &args.split).await?;
    let candidate = artifact.base.clone();
    let adb = ctx.adb()?;
    let installed = inspect_installed(&adb, &inspector, &device.serial, &installed_spec)
        .await?
        .ok_or_else(|| {
            ForgeError::new(
                ErrorCode::PackageNotFound,
                format!(
                    "package {installed_spec} is not installed on {}",
                    device.serial
                ),
            )
            .with_suggestion("Install the application or pass the correct package name.")
            .device(device.serial.clone())
        })?;
    let diff = compare(&candidate, &artifact.splits, &installed);
    let mut value = serde_json::to_value(&diff)?;
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "candidate_artifact".to_string(),
            serde_json::to_value(artifact.summary())?,
        );
    }
    let human = format!(
        "{}\noverall: {:?}\nsafe to skip install: {}\nartifact: {}{}\n",
        diff.summary,
        diff.overall,
        diff.safe_to_skip_install,
        artifact.kind.as_str(),
        if artifact.split_count() > 0 {
            format!(" ({} split(s))", artifact.split_count())
        } else {
            String::new()
        },
    );
    Ok(success("apk.diff", value, human)
        .with_optional_session(session.map(|session| session.id))
        .with_device(device.serial))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_variant_patterns_without_crossing_segments() {
        assert!(pattern_matches(
            "app/build/outputs/apk/debug/app-debug.apk",
            "app/build/outputs/apk/debug/*.apk"
        ));
        assert!(!pattern_matches(
            "app/build/outputs/apk/release/app-release.apk",
            "app/build/outputs/apk/debug/*.apk"
        ));
        assert!(pattern_matches(
            "feature/build/outputs/apk/debug/app-debug.apk",
            "*/build/outputs/apk/debug/*.apk"
        ));
        assert!(pattern_matches(
            "app/build/outputs/apk/free/debug/app-free-debug.apk",
            "app/build/outputs/apk/**/*.apk"
        ));
        assert!(!pattern_matches(
            "app/build/outputs/apk/debug/nested/app-debug.apk",
            "app/build/outputs/apk/debug/*.apk"
        ));
    }

    #[test]
    fn splits_gradle_flavor_variants() {
        assert_eq!(
            split_variant("freeDebug"),
            Some(("free".to_string(), "debug".to_string()))
        );
        assert_eq!(split_variant("debug"), None);
    }

    #[test]
    fn module_filter_matches_a_module_directory_only() {
        assert!(relative_in_module(
            "app/build/outputs/apk/debug/app.apk",
            "app"
        ));
        assert!(!relative_in_module(
            "application/build/outputs/apk/debug/app.apk",
            "app"
        ));
        assert!(relative_in_module(
            "build/app/outputs/apk/debug/app.apk",
            "app"
        ));
    }

    #[test]
    fn conventional_patterns_cover_flavors() {
        let patterns = conventional_patterns("freeDebug");
        assert!(patterns.contains(&"app/build/outputs/apk/free/debug/*.apk".to_string()));
        assert!(patterns.contains(&"app/build/outputs/apk/freeDebug/*.apk".to_string()));
    }
}
