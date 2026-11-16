use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{DesktopError, Result};
use crate::models::{Manifest, ManifestEntry};

const TEXT_PREVIEW_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct TextPreview {
    pub text: String,
    pub size_bytes: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct ManifestStatus {
    pub path: String,
    pub expected: String,
    pub actual: Option<String>,
    pub size_bytes: u64,
    pub status: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct ManifestVerification {
    pub entries: Vec<ManifestStatus>,
    pub ok: usize,
    pub missing: usize,
    pub mismatched: usize,
    pub total: usize,
    pub manifest_present: bool,
    pub error: Option<String>,
}

pub fn safe_join(run_dir: &Path, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    if relative.is_absolute() {
        return Err(DesktopError::PathOutsideRun);
    }
    for component in relative.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(DesktopError::PathOutsideRun),
        }
    }
    let joined = run_dir.join(relative);
    if joined.exists() {
        let run_canonical = normalize_path(
            run_dir
                .canonicalize()
                .unwrap_or_else(|_| run_dir.to_path_buf()),
        );
        let joined_canonical =
            normalize_path(joined.canonicalize().unwrap_or_else(|_| joined.clone()));
        if !joined_canonical.starts_with(&run_canonical) {
            return Err(DesktopError::PathOutsideRun);
        }
    }
    Ok(joined)
}

fn normalize_path(path: PathBuf) -> PathBuf {
    if let Some(value) = path.to_str() {
        if let Some(stripped) = value.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }
    path
}

pub fn read_text_preview(path: &Path) -> Result<TextPreview> {
    let metadata = std::fs::metadata(path)?;
    let size_bytes = metadata.len();
    if size_bytes as usize <= TEXT_PREVIEW_LIMIT {
        let bytes = std::fs::read(path)?;
        return Ok(TextPreview {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            size_bytes,
            truncated: false,
        });
    }

    let head_limit = TEXT_PREVIEW_LIMIT.saturating_mul(3) / 4;
    let tail_limit = TEXT_PREVIEW_LIMIT.saturating_sub(head_limit);
    let mut bytes = Vec::with_capacity(TEXT_PREVIEW_LIMIT + 128);
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let mut head = vec![0u8; head_limit];
    let head_read = file.read(&mut head)?;
    bytes.extend_from_slice(&head[..head_read]);
    bytes.extend_from_slice(
        format!(
            "\n\n[... {} bytes omitted from preview; open externally for the full file ...]\n\n",
            size_bytes.saturating_sub((head_read + tail_limit) as u64)
        )
        .as_bytes(),
    );
    let tail_start = size_bytes.saturating_sub(tail_limit as u64);
    file.seek(SeekFrom::Start(tail_start))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    bytes.extend_from_slice(&tail);
    Ok(TextPreview {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        size_bytes,
        truncated: true,
    })
}

pub fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

pub fn verify_manifest(run_dir: &Path) -> ManifestVerification {
    let manifest_path = run_dir.join("manifest.json");
    if !manifest_path.is_file() {
        return ManifestVerification {
            manifest_present: false,
            error: Some("manifest.json is not present".to_string()),
            ..Default::default()
        };
    }
    let manifest: Manifest = match read_json_file(&manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            return ManifestVerification {
                manifest_present: true,
                error: Some(error.to_string()),
                ..Default::default()
            };
        }
    };
    let mut verification = ManifestVerification {
        manifest_present: true,
        total: manifest.files.len(),
        ..Default::default()
    };
    for entry in manifest.files {
        verification
            .entries
            .push(check_manifest_entry(run_dir, &entry));
    }
    for entry in &verification.entries {
        match entry.status {
            "ok" => verification.ok += 1,
            "missing" => verification.missing += 1,
            _ => verification.mismatched += 1,
        }
    }
    verification
}

fn check_manifest_entry(run_dir: &Path, entry: &ManifestEntry) -> ManifestStatus {
    let Ok(path) = safe_join(run_dir, &entry.path) else {
        return ManifestStatus {
            path: entry.path.clone(),
            expected: entry.sha256.clone(),
            actual: None,
            size_bytes: entry.size_bytes,
            status: "invalid path",
        };
    };
    if !path.is_file() {
        return ManifestStatus {
            path: entry.path.clone(),
            expected: entry.sha256.clone(),
            actual: None,
            size_bytes: entry.size_bytes,
            status: "missing",
        };
    }
    match sha256_file(&path) {
        Ok(actual) if actual.eq_ignore_ascii_case(&entry.sha256) => ManifestStatus {
            path: entry.path.clone(),
            expected: entry.sha256.clone(),
            actual: Some(actual),
            size_bytes: entry.size_bytes,
            status: "ok",
        },
        Ok(actual) => ManifestStatus {
            path: entry.path.clone(),
            expected: entry.sha256.clone(),
            actual: Some(actual),
            size_bytes: entry.size_bytes,
            status: "mismatch",
        },
        Err(_) => ManifestStatus {
            path: entry.path.clone(),
            expected: entry.sha256.clone(),
            actual: None,
            size_bytes: entry.size_bytes,
            status: "unreadable",
        },
    }
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

pub fn format_xml(text: &str) -> String {
    let Ok(document) = roxmltree::Document::parse(text) else {
        return text.to_string();
    };
    let mut output = String::new();
    format_node(document.root_element(), 0, &mut output);
    output
}

fn format_node(node: roxmltree::Node<'_, '_>, indent: usize, output: &mut String) {
    if node.is_text() {
        let text = node.text().unwrap_or("").trim();
        if !text.is_empty() {
            output.push_str(&"  ".repeat(indent));
            output.push_str(text);
            output.push('\n');
        }
        return;
    }
    if !node.is_element() {
        return;
    }
    output.push_str(&"  ".repeat(indent));
    output.push('<');
    output.push_str(node.tag_name().name());
    for attribute in node.attributes() {
        output.push(' ');
        output.push_str(attribute.name());
        output.push_str("=\"");
        output.push_str(attribute.value());
        output.push('"');
    }
    if node
        .children()
        .any(|child| child.is_element() || child.is_text())
    {
        output.push_str(">\n");
        for child in node.children() {
            format_node(child, indent + 1, output);
        }
        output.push_str(&"  ".repeat(indent));
        output.push_str("</");
        output.push_str(node.tag_name().name());
        output.push_str(">\n");
    } else {
        output.push_str(" />\n");
    }
}

pub fn truncate_middle(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars || max_chars < 8 {
        return value.to_string();
    }
    let head = (max_chars - 3) / 2;
    let tail = max_chars - 3 - head;
    let start: String = value.chars().take(head).collect();
    let end: String = value.chars().skip(count - tail).collect();
    format!("{start}...{end}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_directory_escape() {
        let temp = tempfile::tempdir().unwrap();
        assert!(safe_join(temp.path(), "../secret").is_err());
        assert!(safe_join(temp.path(), "C:/secret").is_err());
        assert!(safe_join(temp.path(), "logs/ok.txt").is_ok());
    }

    #[test]
    fn formats_xml_readably() {
        let formatted = format_xml("<root><child value=\"1\">text</child></root>");
        assert!(formatted.contains("<root>"));
        assert!(formatted.contains("  <child value=\"1\">"));
    }

    #[test]
    fn truncates_in_the_middle() {
        let value = "0123456789abcdef";
        let truncated = truncate_middle(value, 10);
        assert!(truncated.starts_with("012"));
        assert!(truncated.contains("..."));
        assert!(truncated.ends_with("def"));
    }
}
