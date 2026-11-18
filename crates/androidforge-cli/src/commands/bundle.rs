//! Portable diagnostic bundles.
//!
//! Bundles are built from a stored run without modifying it. Every payload is
//! redacted, size-bounded, checksummed and named by AndroidForge; source paths
//! are never used directly as ZIP entry names.

use std::collections::BTreeSet;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use androidforge_core::diagnosis::{DiagnosticExplanation, explain_report};
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::recurrence::{build_history, find_fingerprint_in_project};
use androidforge_core::run::RunReport;
use androidforge_core::util::{
    ensure_dir, now_rfc3339, read_json, sha256_hex, short_id, write_atomic,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cli::ReportBundleArgs;
use crate::commands::common::success;
use crate::context::CliContext;

pub const BUNDLE_SCHEMA_VERSION: u32 = 1;
const DEFAULT_PER_FILE_LIMIT: u64 = 1024 * 1024;
const HIERARCHY_LIMIT: u64 = 512 * 1024;
const LOGCAT_EXCERPT_LIMIT: u64 = 256 * 1024;
const LOGCAT_FULL_LIMIT: u64 = 5 * 1024 * 1024;
const MEDIA_LIMIT: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleEntry {
    pub path: String,
    pub kind: String,
    pub size_bytes: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExcludedItem {
    pub source: String,
    pub kind: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleManifest {
    pub schema_version: u32,
    pub bundle_id: String,
    pub created_at: String,
    pub tool_version: String,
    pub run_id: String,
    pub project: String,
    pub project_root: String,
    pub source_run_dir: String,
    pub redaction_enabled: bool,
    pub format: String,
    pub entries: Vec<BundleEntry>,
    pub excluded: Vec<ExcludedItem>,
    pub total_payload_bytes: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub checksums_verified: bool,
    pub original_run_modified: bool,
}

struct Payload {
    path: String,
    kind: String,
    bytes: Vec<u8>,
    source: Option<String>,
}

pub async fn bundle_command(ctx: &CliContext, args: &ReportBundleArgs) -> Result<CommandSuccess> {
    let format = bundle_format(args)?;
    let dir = match args.run.as_str() {
        "latest" => crate::commands::report::resolve_latest_run_dir(ctx, false)?,
        "latest-failure" => crate::commands::report::resolve_latest_run_dir(ctx, true)?,
        _ => resolve_run(ctx, &args.run)?,
    };
    let report: RunReport = read_json(&dir.join("report.json"))?;
    let explanation = explain_report(&report);
    let (butter_reports, unreadable) = load_reports(ctx);
    let history = build_history(&butter_reports, unreadable);
    let recurrence =
        find_fingerprint_in_project(&history, &explanation.fingerprint, &report.project_root)
            .cloned();
    let redactor = ctx.redactor();
    let redaction_enabled = ctx.config.file.redaction.enabled && ctx.config.file.diagnostics.redact;

    let mut warnings = Vec::new();
    let mut excluded = Vec::new();
    let mut payloads = Vec::new();
    let summary = serde_json::json!({
        "run": report,
        "explanation": explanation,
        "recurrence": recurrence,
        "reproduction": {
            "exact_argv": report.invocation.as_ref().map(|invocation| invocation.argv.clone()),
            "recorded_cwd": report.invocation.as_ref().map(|invocation| invocation.cwd.clone()),
            "project_root": report.project_root,
            "stored_command": report.command,
            "steps": report.steps.iter().map(|step| serde_json::json!({
                "name": step.name,
                "command": step.command,
                "status": step.status,
            })).collect::<Vec<_>>(),
            "exact": report.invocation.is_some(),
            "note": if report.invocation.is_some() {
                "argv was captured when the run was created"
            } else {
                "This legacy run does not store its exact invocation; the recorded command and steps are shown instead."
            },
        },
    });
    push_json_payload(
        &mut payloads,
        "summary.json",
        "summary",
        &summary,
        &redactor,
        &mut warnings,
    )?;

    let explanation_markdown = bundle_markdown(&report, &explanation, recurrence.as_ref());
    push_payload(
        &mut payloads,
        "explanation.md",
        "markdown",
        explanation_markdown.as_bytes(),
        None,
        &redactor,
        &mut warnings,
        DEFAULT_PER_FILE_LIMIT,
    )?;

    let mut included_sources = BTreeSet::new();
    for (index, evidence) in explanation.evidence.iter().enumerate() {
        if args.metadata_only {
            excluded.push(ExcludedItem {
                source: evidence.source.clone(),
                kind: "evidence".to_string(),
                reason: "metadata-only bundle requested".to_string(),
                size_bytes: None,
            });
            continue;
        }
        let Ok(path) = safe_source(&dir, &evidence.source) else {
            excluded.push(ExcludedItem {
                source: evidence.source.clone(),
                kind: "evidence".to_string(),
                reason: "path is outside the run directory".to_string(),
                size_bytes: None,
            });
            continue;
        };
        let metadata = std::fs::metadata(&path).ok();
        let Some(metadata) = metadata else {
            excluded.push(ExcludedItem {
                source: evidence.source.clone(),
                kind: "evidence".to_string(),
                reason: "artifact file is missing".to_string(),
                size_bytes: None,
            });
            continue;
        };
        if !metadata.is_file() {
            excluded.push(ExcludedItem {
                source: evidence.source.clone(),
                kind: "evidence".to_string(),
                reason: "artifact is not a regular file".to_string(),
                size_bytes: Some(metadata.len()),
            });
            continue;
        }
        if !text_extension(&path) {
            excluded.push(ExcludedItem {
                source: evidence.source.clone(),
                kind: "evidence".to_string(),
                reason: "binary evidence is available but is not included by default".to_string(),
                size_bytes: Some(metadata.len()),
            });
            continue;
        }
        if metadata.len() > DEFAULT_PER_FILE_LIMIT {
            excluded.push(ExcludedItem {
                source: evidence.source.clone(),
                kind: "evidence".to_string(),
                reason: format!(
                    "evidence exceeds the {} byte default per-file limit",
                    DEFAULT_PER_FILE_LIMIT
                ),
                size_bytes: Some(metadata.len()),
            });
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            excluded.push(ExcludedItem {
                source: evidence.source.clone(),
                kind: "evidence".to_string(),
                reason: "artifact could not be read".to_string(),
                size_bytes: Some(metadata.len()),
            });
            continue;
        };
        included_sources.insert(evidence.source.clone());
        push_payload(
            &mut payloads,
            &unique_path("evidence", index, &path),
            "evidence",
            &bytes,
            Some(evidence.source.clone()),
            &redactor,
            &mut warnings,
            DEFAULT_PER_FILE_LIMIT,
        )?;
    }

    for (index, artifact) in report.artifacts.iter().enumerate() {
        let kind = artifact.kind.to_ascii_lowercase();
        let path = artifact.path.to_ascii_lowercase();
        let is_screenshot =
            kind.contains("screenshot") || path.ends_with(".png") || path.ends_with(".jpg");
        let is_recording =
            kind.contains("record") || path.ends_with(".mp4") || path.ends_with(".webm");
        let is_logcat = kind.contains("logcat");
        let is_hierarchy =
            kind.contains("hierarchy") || (path.ends_with(".xml") && path.contains("hierarchy"));
        if included_sources.contains(&artifact.path) {
            continue;
        }
        if args.metadata_only {
            excluded.retain(|item| item.source != artifact.path);
            excluded.push(ExcludedItem {
                source: artifact.path.clone(),
                kind: artifact.kind.clone(),
                reason: "metadata-only bundle requested".to_string(),
                size_bytes: artifact.size_bytes,
            });
            continue;
        }
        // Replace a generic exclusion (for example "binary evidence") with a
        // more specific reason for artifacts we deliberately handle here.
        excluded.retain(|item| item.source != artifact.path);
        if !(is_screenshot || is_recording || is_logcat || is_hierarchy) {
            continue;
        }
        let Ok(source) = safe_source(&dir, &artifact.path) else {
            excluded.push(ExcludedItem {
                source: artifact.path.clone(),
                kind: artifact.kind.clone(),
                reason: "path is outside the run directory".to_string(),
                size_bytes: artifact.size_bytes,
            });
            continue;
        };
        let Ok(metadata) = std::fs::metadata(&source) else {
            excluded.push(ExcludedItem {
                source: artifact.path.clone(),
                kind: artifact.kind.clone(),
                reason: "artifact file is missing".to_string(),
                size_bytes: artifact.size_bytes,
            });
            continue;
        };
        if !metadata.is_file() {
            excluded.push(ExcludedItem {
                source: artifact.path.clone(),
                kind: artifact.kind.clone(),
                reason: "artifact is not a regular file".to_string(),
                size_bytes: Some(metadata.len()),
            });
            continue;
        }
        if is_screenshot && !args.include_screenshots {
            excluded.push(ExcludedItem {
                source: artifact.path.clone(),
                kind: artifact.kind.clone(),
                reason: "screenshots are opt-in; pass --include-screenshots".to_string(),
                size_bytes: Some(metadata.len()),
            });
            continue;
        }
        if is_recording && !args.include_recordings {
            excluded.push(ExcludedItem {
                source: artifact.path.clone(),
                kind: artifact.kind.clone(),
                reason: "screen recordings are opt-in; pass --include-recordings".to_string(),
                size_bytes: Some(metadata.len()),
            });
            continue;
        }
        let limit = if is_logcat {
            if args.include_full_logs {
                LOGCAT_FULL_LIMIT
            } else {
                LOGCAT_EXCERPT_LIMIT
            }
        } else if is_hierarchy {
            HIERARCHY_LIMIT
        } else if is_screenshot || is_recording {
            MEDIA_LIMIT
        } else {
            DEFAULT_PER_FILE_LIMIT
        };
        if metadata.len() > limit {
            excluded.push(ExcludedItem {
                source: artifact.path.clone(),
                kind: artifact.kind.clone(),
                reason: format!("artifact exceeds the {limit} byte limit for this bundle"),
                size_bytes: Some(metadata.len()),
            });
            continue;
        }
        let Ok(bytes) = std::fs::read(&source) else {
            excluded.push(ExcludedItem {
                source: artifact.path.clone(),
                kind: artifact.kind.clone(),
                reason: "artifact could not be read".to_string(),
                size_bytes: Some(metadata.len()),
            });
            continue;
        };
        let bytes = if is_logcat {
            if args.include_full_logs {
                bytes
            } else {
                tail_excerpt(&bytes, LOGCAT_EXCERPT_LIMIT as usize)
            }
        } else {
            bytes
        };
        if is_screenshot || is_recording {
            // Media must not be text-redacted; it is included only on explicit
            // opt-in and its checksum is recorded. Do not claim it is redacted.
            warnings.push(format!(
                "{} is included as binary media; pixel content is not text-redacted.",
                artifact.path
            ));
            payloads.push(Payload {
                path: unique_path(
                    if is_screenshot {
                        "screenshots"
                    } else {
                        "recordings"
                    },
                    index,
                    &source,
                ),
                kind: artifact.kind.clone(),
                bytes,
                source: Some(artifact.path.clone()),
            });
        } else {
            push_payload(
                &mut payloads,
                &unique_path(if is_logcat { "logs" } else { "hierarchy" }, index, &source),
                &artifact.kind,
                &bytes,
                Some(artifact.path.clone()),
                &redactor,
                &mut warnings,
                limit,
            )?;
        }
    }

    for (index, artifact) in report.artifacts.iter().enumerate() {
        if included_sources.contains(&artifact.path)
            || payloads
                .iter()
                .any(|payload| payload.source.as_deref() == Some(artifact.path.as_str()))
        {
            continue;
        }
        if excluded.iter().any(|item| item.source == artifact.path) {
            continue;
        }
        excluded.push(ExcludedItem {
            source: artifact.path.clone(),
            kind: artifact.kind.clone(),
            reason: format!(
                "artifact kind '{}' is not bundled by default",
                artifact.kind
            ),
            size_bytes: artifact.size_bytes,
        });
        let _ = index;
    }

    // Apply the total size limit deterministically: payloads are already in a
    // deterministic order, and later payloads are excluded rather than silently
    // truncated.
    let max_bytes = args.max_bytes.max(1024);
    let mut total = 0u64;
    let mut kept = Vec::new();
    for payload in payloads {
        if total + payload.bytes.len() as u64 > max_bytes {
            excluded.push(ExcludedItem {
                source: payload
                    .source
                    .clone()
                    .unwrap_or_else(|| payload.path.clone()),
                kind: payload.kind,
                reason: format!("bundle max size of {max_bytes} bytes would be exceeded"),
                size_bytes: Some(payload.bytes.len() as u64),
            });
            continue;
        }
        total += payload.bytes.len() as u64;
        kept.push(payload);
    }
    payloads = kept;

    let entries = payloads
        .iter()
        .map(|payload| BundleEntry {
            path: payload.path.clone(),
            kind: payload.kind.clone(),
            size_bytes: payload.bytes.len() as u64,
            sha256: sha256_hex(&payload.bytes),
            source: payload.source.clone(),
        })
        .collect::<Vec<_>>();

    if ctx.dry_run {
        let result = serde_json::json!({
            "dry_run": true,
            "run_id": report.run_id,
            "format": format,
            "output": output_path(ctx, args, &report.run_id, &format),
            "entries": entries,
            "excluded": excluded,
            "total_payload_bytes": total,
            "max_bytes": max_bytes,
            "redaction_enabled": redaction_enabled,
            "warnings": warnings,
            "would_write": format == "zip",
        });
        let human = format!(
            "Bundle preview for {}: {} payload(s), {} bytes, {} excluded. No files were written.\n",
            report.run_id,
            entries.len(),
            total,
            excluded.len()
        );
        return Ok(success("report.bundle", result, human));
    }

    let mut manifest = BundleManifest {
        schema_version: BUNDLE_SCHEMA_VERSION,
        bundle_id: format!("bundle-{}", short_id()),
        created_at: now_rfc3339(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        run_id: report.run_id.clone(),
        project: report.project.clone(),
        project_root: report.project_root.clone(),
        source_run_dir: dir.to_string_lossy().into_owned(),
        redaction_enabled,
        format: format.clone(),
        entries,
        excluded,
        total_payload_bytes: total,
        warnings,
        checksums_verified: false,
        original_run_modified: false,
    };

    let output = output_path(ctx, args, &report.run_id, &format);
    match format.as_str() {
        "markdown" => {
            let markdown = bundle_summary_markdown(&manifest, &summary);
            if let Some(output) = &output {
                ensure_dir(output.parent().unwrap_or(Path::new(".")))?;
                write_atomic(output, markdown.as_bytes())?;
            }
            let result = serde_json::json!({
                "format": "markdown",
                "output": output,
                "markdown": markdown,
                "manifest": manifest,
            });
            let human = if ctx.json {
                format!("Prepared Markdown evidence for {}.\n", report.run_id)
            } else {
                markdown.clone()
            };
            Ok(success("report.bundle", result, human))
        }
        "json" => {
            let value = serde_json::json!({
                "manifest": manifest,
                "summary": summary,
            });
            let bytes = serde_json::to_vec_pretty(&value)?;
            if let Some(output) = &output {
                ensure_dir(output.parent().unwrap_or(Path::new(".")))?;
                write_atomic(output, &bytes)?;
            }
            let result = serde_json::json!({
                "format": "json",
                "output": output,
                "manifest": manifest,
            });
            Ok(success(
                "report.bundle",
                result,
                format!("Prepared JSON evidence for {}.\n", report.run_id),
            ))
        }
        _ => {
            let output = output.ok_or_else(|| {
                ForgeError::invalid_argument(
                    "a ZIP bundle requires --output or the default artifact location",
                )
            })?;
            let bytes = build_zip(&payloads, &manifest)?;
            let verified = verify_zip(&bytes, &manifest)?;
            if !verified {
                return Err(ForgeError::new(
                    ErrorCode::ArtifactWriteFailed,
                    "the generated ZIP failed checksum verification and was not written",
                )
                .with_suggestion(
                    "This indicates a bug or filesystem corruption; the run directory was not modified.",
                ));
            }
            // The manifest is not part of its own checksum set, so rebuild it
            // after verification. The ZIP written to disk must not claim
            // `checksums_verified:false` after a successful verification.
            manifest.checksums_verified = true;
            let bytes = build_zip(&payloads, &manifest)?;
            ensure_dir(output.parent().unwrap_or(Path::new(".")))?;
            write_atomic(&output, &bytes)?;
            // Reopen the written file and verify it again so a short write or
            // filesystem issue cannot be reported as success.
            let written = std::fs::read(&output).map_err(|error| {
                ForgeError::new(
                    ErrorCode::ArtifactWriteFailed,
                    format!("could not reopen {}: {error}", output.display()),
                )
            })?;
            if !verify_zip(&written, &manifest)? {
                return Err(ForgeError::new(
                    ErrorCode::ArtifactWriteFailed,
                    "the written ZIP did not pass checksum verification",
                ));
            }
            let result = serde_json::json!({
                "format": "zip",
                "output": output,
                "bytes": bytes.len(),
                "verified": true,
                "manifest": manifest,
            });
            Ok(success(
                "report.bundle",
                result,
                format!(
                    "Created and verified diagnostic bundle {}\n{} payload(s), {} bytes uncompressed, {} excluded.\n",
                    output.display(),
                    manifest.entries.len(),
                    manifest.total_payload_bytes,
                    manifest.excluded.len()
                ),
            ))
        }
    }
}

fn bundle_format(args: &ReportBundleArgs) -> Result<String> {
    if args.markdown && args.format.is_some() {
        return Err(ForgeError::invalid_argument(
            "choose --markdown or --format, not both",
        ));
    }
    let format = if args.markdown {
        "markdown".to_string()
    } else {
        args.format.as_deref().unwrap_or("zip").to_ascii_lowercase()
    };
    if !matches!(format.as_str(), "zip" | "json" | "markdown") {
        return Err(ForgeError::invalid_argument(format!(
            "unknown bundle format '{format}'; use zip, json or markdown"
        )));
    }
    Ok(format)
}

fn output_path(
    ctx: &CliContext,
    args: &ReportBundleArgs,
    run_id: &str,
    format: &str,
) -> Option<PathBuf> {
    if let Some(output) = &args.output {
        return Some(if output.is_absolute() {
            output.clone()
        } else {
            ctx.paths.cwd.join(output)
        });
    }
    if format == "zip" {
        return Some(
            ctx.paths
                .artifacts_dir
                .join("bundles")
                .join(format!("{run_id}-evidence.zip")),
        );
    }
    None
}

fn resolve_run(ctx: &CliContext, spec: &str) -> Result<PathBuf> {
    // Reuse the report resolver's project-aware behavior by trying the local
    // store first and then `.artifacts` directories under the project tree.
    if let Ok(dir) = androidforge_core::run::resolve_run_dir(&ctx.paths.artifacts_dir, spec) {
        return Ok(dir);
    }
    let mut candidates = Vec::new();
    collect_run_dirs(&ctx.paths.project_root, 0, &mut candidates);
    candidates.sort();
    if spec == "latest" {
        return candidates.pop().ok_or_else(|| {
            ForgeError::new(ErrorCode::InvalidArgument, "no AndroidForge runs exist yet")
                .with_suggestion("Run a command that creates a run report first.")
        });
    }
    let matching = candidates
        .into_iter()
        .filter(|dir| {
            dir.file_name()
                .map(|name| name.to_string_lossy() == spec)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    match matching.len() {
        0 => Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            format!("run '{spec}' was not found"),
        )
        .with_suggestion("Use `androidforge report list --all` to find run ids.")),
        1 => Ok(matching[0].clone()),
        _ => Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            format!("run '{spec}' exists in more than one project artifact store"),
        )
        .with_suggestion("Pass --project <dir> to select the project.")),
    }
}

fn collect_run_dirs(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 4 {
        return;
    }
    let runs = root.join(".artifacts").join("runs");
    if runs.is_dir()
        && let Ok(entries) = std::fs::read_dir(&runs)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join("report.json").is_file() {
                out.push(path);
            }
        }
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.')
            || matches!(
                name.as_ref(),
                "target" | "node_modules" | "build" | "dist" | "vendor" | "crates" | "src"
            )
        {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_run_dirs(&path, depth + 1, out);
        }
    }
}

fn load_reports(ctx: &CliContext) -> (Vec<RunReport>, usize) {
    let mut reports = std::collections::BTreeMap::new();
    let mut unreadable = 0usize;
    let runs = ctx.paths.artifacts_dir.join("runs");
    let Ok(entries) = std::fs::read_dir(&runs) else {
        return (Vec::new(), 0);
    };
    for entry in entries.flatten() {
        let path = entry.path().join("report.json");
        if !path.is_file() {
            continue;
        }
        match read_json::<RunReport>(&path) {
            Ok(report) => {
                reports.entry(report.run_id.clone()).or_insert(report);
            }
            Err(_) => unreadable += 1,
        }
    }
    (reports.into_values().collect(), unreadable)
}

fn safe_source(run_dir: &Path, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            "artifact path escapes the run directory",
        ));
    }
    let path = run_dir.join(relative);
    let canonical_run = run_dir.canonicalize().map_err(|error| {
        ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            format!("could not canonicalize {}: {error}", run_dir.display()),
        )
    })?;
    let Ok(canonical) = path.canonicalize() else {
        // Missing files are handled by the caller with an explicit excluded
        // entry. The lexical check above already prevents traversal.
        return Ok(path);
    };
    if !canonical.starts_with(&canonical_run) {
        return Err(ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            "artifact path resolves outside the run directory",
        ));
    }
    Ok(canonical)
}

fn text_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "txt" | "log" | "xml" | "json" | "md" | "csv" | "yaml" | "yml"
    )
}

fn unique_path(folder: &str, index: usize, source: &Path) -> String {
    let name = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".to_string());
    let sanitized = sanitize_name(&name);
    format!("{folder}/{:03}-{sanitized}", index + 1)
}

fn sanitize_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let sanitized = sanitized.trim_matches('.').to_string();
    if sanitized.is_empty() {
        "artifact".to_string()
    } else {
        sanitized
    }
}

fn tail_excerpt(bytes: &[u8], max: usize) -> Vec<u8> {
    if bytes.len() <= max {
        return bytes.to_vec();
    }
    let start = bytes.len() - max;
    let start = bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|offset| start + offset + 1)
        .unwrap_or(start);
    let mut out = b"[... logcat excerpt; earlier lines omitted ...]\n".to_vec();
    out.extend_from_slice(&bytes[start..]);
    out
}

fn redact_text(redactor: &androidforge_core::redact::Redactor, bytes: &[u8]) -> (Vec<u8>, bool) {
    let text = String::from_utf8_lossy(bytes);
    let mut redacted = redactor.redact(&text);
    let pairing = regex::Regex::new(r"(?i)((?:pairing[_ -]?code|pair[_ -]?code)\s*[:=]?\s*)\d{6}")
        .expect("valid pairing-code regex");
    redacted = pairing
        .replace_all(&redacted, "${1}[REDACTED]")
        .into_owned();
    let changed = redacted != text;
    (redacted.into_bytes(), changed)
}

#[allow(clippy::too_many_arguments)]
fn push_payload(
    payloads: &mut Vec<Payload>,
    path: &str,
    kind: &str,
    bytes: &[u8],
    source: Option<String>,
    redactor: &androidforge_core::redact::Redactor,
    warnings: &mut Vec<String>,
    limit: u64,
) -> Result<()> {
    if bytes.len() as u64 > limit {
        warnings.push(format!(
            "{} exceeded the {limit} byte per-file limit and was not included",
            source.as_deref().unwrap_or(path)
        ));
        return Ok(());
    }
    let (bytes, redacted) = redact_text(redactor, bytes);
    if redacted && source.is_none() {
        warnings.push(format!("redaction changed {path}"));
    }
    payloads.push(Payload {
        path: path.to_string(),
        kind: kind.to_string(),
        bytes,
        source,
    });
    Ok(())
}

fn push_json_payload(
    payloads: &mut Vec<Payload>,
    path: &str,
    kind: &str,
    value: &Value,
    redactor: &androidforge_core::redact::Redactor,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let mut value = value.clone();
    redactor.redact_value(&mut value);
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    bytes.push(b'\n');
    push_payload(
        payloads,
        path,
        kind,
        &bytes,
        None,
        redactor,
        warnings,
        DEFAULT_PER_FILE_LIMIT,
    )
}

fn build_zip(payloads: &[Payload], manifest: &BundleManifest) -> Result<Vec<u8>> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600);
    for payload in payloads {
        writer
            .start_file(payload.path.clone(), options)
            .map_err(zip_error)?;
        writer.write_all(&payload.bytes).map_err(|error| {
            ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!("could not write {} into the bundle: {error}", payload.path),
            )
        })?;
    }
    // The manifest is not included in its own checksum list. It is written
    // last so readers can verify every payload before trusting it.
    let mut manifest_value = serde_json::to_value(manifest)?;
    if let Some(object) = manifest_value.as_object_mut() {
        object.insert(
            "manifest_checksum_scope".to_string(),
            json_string("payload entries only"),
        );
        object.insert(
            "manifest_note".to_string(),
            json_string("manifest.json is not listed in its own entries"),
        );
    }
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest_value)?;
    manifest_bytes.push(b'\n');
    writer
        .start_file("manifest.json", options)
        .map_err(zip_error)?;
    writer.write_all(&manifest_bytes).map_err(|error| {
        ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            format!("could not write manifest.json into the bundle: {error}"),
        )
    })?;
    let cursor = writer.finish().map_err(zip_error)?;
    Ok(cursor.into_inner())
}

fn json_string(value: &str) -> Value {
    Value::String(value.to_string())
}

fn zip_error(error: zip::result::ZipError) -> ForgeError {
    ForgeError::new(
        ErrorCode::ArtifactWriteFailed,
        format!("could not build the diagnostic ZIP: {error}"),
    )
}

fn verify_zip(bytes: &[u8], manifest: &BundleManifest) -> Result<bool> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(zip_error)?;
    let mut names = BTreeSet::new();
    for entry in &manifest.entries {
        names.insert(entry.path.clone());
    }
    let mut seen = BTreeSet::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(zip_error)?;
        let name = entry.name().map_err(zip_error)?.to_string();
        if name == "manifest.json" {
            continue;
        }
        if !names.contains(&name) {
            return Ok(false);
        }
        let mut payload = Vec::new();
        entry.read_to_end(&mut payload).map_err(|error| {
            ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!("could not read {name} from the generated ZIP: {error}"),
            )
        })?;
        let expected = manifest
            .entries
            .iter()
            .find(|candidate| candidate.path == name)
            .map(|candidate| candidate.sha256.as_str())
            .unwrap_or_default();
        if sha256_hex(&payload) != expected {
            return Ok(false);
        }
        seen.insert(name);
    }
    Ok(seen.len() == manifest.entries.len())
}

fn bundle_markdown(
    report: &RunReport,
    explanation: &DiagnosticExplanation,
    recurrence: Option<&androidforge_core::recurrence::FingerprintRecurrence>,
) -> String {
    let mut out = format!(
        "# Failure evidence: `{}`\n\n- Project: `{}`\n- Status: **{:?}**\n- Category: **{}**\n- Confidence: **{:?}**\n- Fingerprint: `{}`\n",
        report.run_id,
        report.project,
        report.status,
        explanation.diagnosis.category.as_str(),
        explanation.diagnosis.confidence,
        explanation.fingerprint
    );
    if let Some(recurrence) = recurrence {
        out.push_str(&format!(
            "- Recurrence: **{}**, {} occurrence(s), first seen `{}`, currently resolved: {}\n",
            recurrence.status.as_str(),
            recurrence.occurrence_count,
            recurrence.first_seen,
            recurrence.currently_resolved
        ));
    }
    out.push_str(&format!(
        "\n## Summary\n\n{}\n\n## Reproduction\n\n",
        explanation.diagnosis.summary
    ));
    match &report.invocation {
        Some(invocation) => {
            out.push_str("```powershell\n");
            out.push_str(&shell_join(&invocation.argv));
            out.push_str("\n```\n");
        }
        None => {
            out.push_str(
                "The exact original argv was not stored for this legacy run. Recorded command: `",
            );
            out.push_str(&report.command);
            out.push_str("`.\n");
            for step in &report.steps {
                out.push_str(&format!("- `{}` — {}\n", step.name, step.command));
            }
        }
    }
    out.push_str("\n## Evidence\n\n");
    if explanation.evidence.is_empty() {
        out.push_str("No artifact evidence was recorded.\n");
    } else {
        for evidence in &explanation.evidence {
            out.push_str(&format!(
                "- `{}` — {}\n",
                evidence.source, evidence.description
            ));
        }
    }
    out.push_str("\n## Next actions\n\n");
    for action in &explanation.next_actions {
        out.push_str(&format!(
            "- {}: `{}`\n",
            action.reason,
            shell_join(&action.command.argv)
        ));
    }
    out.push_str(
        "\n## Redaction\n\nText payloads are redacted with AndroidForge's configured patterns. Binary screenshots and recordings, when explicitly included, are not pixel-redacted.\n",
    );
    out
}

fn bundle_summary_markdown(manifest: &BundleManifest, summary: &Value) -> String {
    let mut out = format!(
        "# Diagnostic bundle: `{}`\n\n- Bundle: `{}`\n- Created: {}\n- Project: `{}`\n- Source run: `{}`\n- Redaction enabled: {}\n\n## Included payloads ({} files, {} bytes)\n\n",
        manifest.run_id,
        manifest.bundle_id,
        manifest.created_at,
        manifest.project,
        manifest.source_run_dir,
        manifest.redaction_enabled,
        manifest.entries.len(),
        manifest.total_payload_bytes
    );
    for entry in &manifest.entries {
        out.push_str(&format!(
            "- `{}` ({}, {} bytes) SHA-256 `{}`\n",
            entry.path, entry.kind, entry.size_bytes, entry.sha256
        ));
    }
    out.push_str("\n## Available but excluded\n\n");
    if manifest.excluded.is_empty() {
        out.push_str("Nothing was excluded.\n");
    } else {
        for item in &manifest.excluded {
            out.push_str(&format!(
                "- `{}` ({}) — {}\n",
                item.source, item.kind, item.reason
            ));
        }
    }
    if let Some(explanation) = summary.get("explanation") {
        out.push_str("\n## Diagnosis\n\n");
        if let Some(summary_text) = explanation
            .get("diagnosis")
            .and_then(|diagnosis| diagnosis.get("summary"))
            .and_then(Value::as_str)
        {
            out.push_str(summary_text);
            out.push('\n');
        }
        out.push_str(&format!(
            "\nFingerprint: `{}`\n",
            explanation
                .get("fingerprint")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ));
    }
    out.push_str(
        "\n## Manifest\n\n`manifest.json` records every payload checksum. It is intentionally not listed in its own entries.\n",
    );
    out
}

fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| {
            if arg.is_empty()
                || arg.chars().any(|ch| {
                    ch.is_whitespace() || matches!(ch, '"' | '\'' | '&' | '|' | '<' | '>' | '^')
                })
            {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use androidforge_core::redact::Redactor;

    fn manifest() -> BundleManifest {
        BundleManifest {
            schema_version: BUNDLE_SCHEMA_VERSION,
            bundle_id: "bundle-test".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            tool_version: "test".to_string(),
            run_id: "run-1".to_string(),
            project: "Example".to_string(),
            project_root: "A:/Example".to_string(),
            source_run_dir: "A:/Example/.artifacts/runs/run-1".to_string(),
            redaction_enabled: true,
            format: "zip".to_string(),
            entries: Vec::new(),
            excluded: Vec::new(),
            total_payload_bytes: 0,
            warnings: Vec::new(),
            checksums_verified: false,
            original_run_modified: false,
        }
    }

    fn payload(path: &str, bytes: &[u8]) -> Payload {
        Payload {
            path: path.to_string(),
            kind: "test".to_string(),
            bytes: bytes.to_vec(),
            source: None,
        }
    }

    #[test]
    fn zip_round_trip_verifies_checksums() {
        let payloads = vec![payload("summary.json", b"{\"ok\":true}\n")];
        let mut manifest = manifest();
        manifest.entries = payloads
            .iter()
            .map(|payload| BundleEntry {
                path: payload.path.clone(),
                kind: payload.kind.clone(),
                size_bytes: payload.bytes.len() as u64,
                sha256: sha256_hex(&payload.bytes),
                source: None,
            })
            .collect();
        manifest.total_payload_bytes = payloads[0].bytes.len() as u64;
        let bytes = build_zip(&payloads, &manifest).unwrap();
        assert!(verify_zip(&bytes, &manifest).unwrap());
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert!(archive.by_name("manifest.json").is_ok());
        assert!(archive.by_name("summary.json").is_ok());
    }

    #[test]
    fn rebuilt_manifest_records_successful_verification() {
        let payloads = vec![payload("summary.json", b"{\"ok\":true}\n")];
        let mut manifest = manifest();
        manifest.entries = payloads
            .iter()
            .map(|payload| BundleEntry {
                path: payload.path.clone(),
                kind: payload.kind.clone(),
                size_bytes: payload.bytes.len() as u64,
                sha256: sha256_hex(&payload.bytes),
                source: None,
            })
            .collect();
        manifest.total_payload_bytes = payloads[0].bytes.len() as u64;
        let bytes = build_zip(&payloads, &manifest).unwrap();
        assert!(verify_zip(&bytes, &manifest).unwrap());
        manifest.checksums_verified = true;
        let bytes = build_zip(&payloads, &manifest).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut text = String::new();
        archive
            .by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        let bundled: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(bundled["checksums_verified"], true);
    }

    #[test]
    fn checksum_mismatch_is_rejected() {
        let payloads = vec![payload("summary.json", b"one")];
        let mut manifest = manifest();
        manifest.entries = vec![BundleEntry {
            path: "summary.json".to_string(),
            kind: "test".to_string(),
            size_bytes: 3,
            sha256: sha256_hex(b"two"),
            source: None,
        }];
        let bytes = build_zip(&payloads, &manifest).unwrap();
        assert!(!verify_zip(&bytes, &manifest).unwrap());
    }

    #[test]
    fn pairing_codes_are_redacted() {
        let redactor = Redactor::default();
        let (bytes, changed) = redact_text(
            &redactor,
            b"pairing code: 123456\ncode accepted\npassword=hunter2\n",
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(changed);
        assert!(!text.contains("123456"));
        assert!(!text.contains("hunter2"));
        assert!(text.contains("[REDACTED]"));
    }

    #[test]
    fn paths_cannot_escape_the_run_directory() {
        let temp = tempfile::tempdir().unwrap();
        let run = temp.path().join("run");
        std::fs::create_dir_all(&run).unwrap();
        assert!(safe_source(&run, "../outside.txt").is_err());
        assert!(safe_source(&run, "C:/Windows/win.ini").is_err());
    }

    #[test]
    fn names_are_sanitized_and_unique() {
        assert_eq!(sanitize_name("..\\evil?.txt"), "_evil_.txt");
        assert_eq!(
            unique_path("evidence", 2, Path::new("a b/c?.txt")),
            "evidence/003-c_.txt"
        );
    }

    #[test]
    fn tail_excerpt_is_bounded() {
        let bytes = b"line one\nline two\nline three\n";
        let excerpt = tail_excerpt(bytes, 16);
        assert!(excerpt.len() < bytes.len() + 64);
        let text = String::from_utf8_lossy(&excerpt);
        assert!(text.contains("logcat excerpt"));
    }
}
