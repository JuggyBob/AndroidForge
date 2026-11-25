use std::ops::Range;

use regex::{Regex, RegexBuilder};
use serde_json::Value;

/// Maximum number of text bytes examined by one secret scan.
///
/// Scanning is intentionally bounded so untrusted logs cannot make reporting
/// unbounded in time or memory.
pub const SECRET_SCAN_MAX_BYTES: usize = 1024 * 1024;

/// Maximum number of findings returned by one secret scan.
pub const SECRET_SCAN_MAX_FINDINGS: usize = 100;

/// Category of a suspected secret. Never carries the secret value itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretKind {
    KeyValue,
    Bearer,
    PrivateKey,
    AwsAccessKey,
    GitHubToken,
    GoogleApiKey,
    OpenAiKey,
    Custom,
}

impl SecretKind {
    /// Stable machine-readable label for reports and CLI output.
    pub fn label(self) -> &'static str {
        match self {
            Self::KeyValue => "key_value",
            Self::Bearer => "bearer_token",
            Self::PrivateKey => "private_key",
            Self::AwsAccessKey => "aws_access_key",
            Self::GitHubToken => "github_token",
            Self::GoogleApiKey => "google_api_key",
            Self::OpenAiKey => "openai_key",
            Self::Custom => "custom_pattern",
        }
    }

    /// Builds a preview from the category and length only. No bytes from the
    /// matched text are copied into the preview.
    fn redacted_preview(self, byte_len: usize) -> String {
        match self {
            Self::KeyValue => "key=value [REDACTED]".to_string(),
            Self::Bearer => "Bearer [REDACTED]".to_string(),
            Self::PrivateKey => "-----BEGIN PRIVATE KEY----- [REDACTED]".to_string(),
            Self::AwsAccessKey => format!("AKIA[REDACTED] ({byte_len} bytes)"),
            Self::GitHubToken => format!("github token [REDACTED] ({byte_len} bytes)"),
            Self::GoogleApiKey => format!("AIza[REDACTED] ({byte_len} bytes)"),
            Self::OpenAiKey => format!("sk-[REDACTED] ({byte_len} bytes)"),
            Self::Custom => format!("[REDACTED] ({byte_len} bytes)"),
        }
    }
}

/// A safe description of a suspected secret found in text.
///
/// Findings expose location metadata and a synthesized preview. The matched
/// value is never stored in the finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretFinding {
    pub kind: SecretKind,
    /// Byte offset of the first matched byte in the scanned text.
    pub start: usize,
    /// Byte offset one past the last matched byte.
    pub end: usize,
    /// 1-based line number of the match.
    pub line: usize,
    /// Redacted, category-derived preview. Never contains the secret value.
    pub preview: String,
}

impl SecretFinding {
    pub fn byte_range(&self) -> Range<usize> {
        self.start..self.end
    }

    pub fn label(&self) -> &'static str {
        self.kind.label()
    }
}

const DEFAULT_PATTERNS: &[(&str, bool, SecretKind)] = &[
    // Quoted values may contain spaces. Keep the quotes and delimiters while
    // replacing only the value.
    (
        r#"(?i)((?:password|passwd|pwd)"?\s*[:=]\s*["'])([^"'\n]*)(["'])"#,
        true,
        SecretKind::KeyValue,
    ),
    (
        r#"(?i)((?:secret|token|api[_-]?key|authorization)"?\s*[:=]\s*["'])([^"'\n]*)(["'])"#,
        false,
        SecretKind::KeyValue,
    ),
    // Boolean XML/JSON attributes named "password" are structural, not secrets.
    (
        r#"(?i)((?:password|passwd|pwd)"?\s*[:=]\s*["']?)([^"'\s,;&}]+)"#,
        true,
        SecretKind::KeyValue,
    ),
    (
        r#"(?i)((?:secret|token|api[_-]?key|authorization)"?\s*[:=]\s*(?:bearer\s+)?["']?)([^"'\s,;&}]+)"#,
        false,
        SecretKind::KeyValue,
    ),
    (
        r"(?i)((?:bearer\s+)[A-Za-z0-9._~+/=-]{8,})",
        false,
        SecretKind::Bearer,
    ),
    (
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        false,
        SecretKind::PrivateKey,
    ),
    (r"\bAKIA[0-9A-Z]{16}\b", false, SecretKind::AwsAccessKey),
    (
        r"\bgh[pousr]_[A-Za-z0-9]{20,}\b",
        false,
        SecretKind::GitHubToken,
    ),
    (
        r"\bgithub_pat_[A-Za-z0-9_]{20,}\b",
        false,
        SecretKind::GitHubToken,
    ),
    // Bare provider keys. Simple character classes keep matching linear.
    // A leading word boundary avoids matches inside names like "task-sk-...".
    // No trailing boundary: the character class includes '-' and '_'.
    (r"\bAIza[0-9A-Za-z_-]{30,}", false, SecretKind::GoogleApiKey),
    (r"\bsk-[A-Za-z0-9_-]{20,}", false, SecretKind::OpenAiKey),
];

#[derive(Debug, Clone)]
struct RedactionRule {
    regex: Regex,
    preserve_boolean_values: bool,
    kind: SecretKind,
}

#[derive(Debug, Clone)]
pub struct Redactor {
    enabled: bool,
    rules: Vec<RedactionRule>,
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new(true, &[], &[])
    }
}

impl Redactor {
    pub fn new(enabled: bool, extra_patterns: &[String], configured_patterns: &[String]) -> Self {
        let mut rules = Vec::new();
        if enabled {
            for (pattern, preserve_boolean_values, kind) in DEFAULT_PATTERNS {
                if let Ok(regex) = RegexBuilder::new(pattern).build() {
                    rules.push(RedactionRule {
                        regex,
                        preserve_boolean_values: *preserve_boolean_values,
                        kind: *kind,
                    });
                }
            }
            for pattern in configured_patterns.iter().chain(extra_patterns.iter()) {
                if let Ok(regex) = RegexBuilder::new(pattern).build() {
                    rules.push(RedactionRule {
                        regex,
                        preserve_boolean_values: false,
                        kind: SecretKind::Custom,
                    });
                }
            }
        }
        Self { enabled, rules }
    }

    /// Build a redactor whose extra inputs are *literal token prefixes* rather
    /// than regular expressions. Each prefix matches itself followed by the
    /// rest of the non-whitespace token, so `sk-` redacts `sk-abc123` without
    /// ever treating user input as regex syntax.
    pub fn with_literal_prefixes(
        enabled: bool,
        prefixes: &[String],
        configured_patterns: &[String],
    ) -> Self {
        if !enabled {
            return Self::disabled();
        }
        let extras = prefixes
            .iter()
            .filter(|prefix| !prefix.trim().is_empty())
            .map(|prefix| format!(r"{}\S+", regex::escape(prefix.trim())))
            .collect::<Vec<_>>();
        Self::new(true, &extras, configured_patterns)
    }

    pub fn disabled() -> Self {
        Self {
            enabled: false,
            rules: Vec::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn redact(&self, text: &str) -> String {
        if !self.enabled || self.rules.is_empty() {
            return text.to_string();
        }
        let mut output = text.to_string();
        for rule in &self.rules {
            output = rule
                .regex
                .replace_all(&output, |caps: &regex::Captures<'_>| {
                    if rule.preserve_boolean_values && caps.len() >= 3 {
                        let value = caps[2].trim_matches(|ch| ch == '"' || ch == '\'');
                        if matches!(
                            value.to_ascii_lowercase().as_str(),
                            "true" | "false" | "null" | "none"
                        ) {
                            return caps[0].to_string();
                        }
                    }
                    if caps.len() >= 2 {
                        let suffix = caps.get(3).map(|value| value.as_str()).unwrap_or("");
                        format!("{}[REDACTED]{}", &caps[1], suffix)
                    } else {
                        "[REDACTED]".to_string()
                    }
                })
                .into_owned();
        }
        output
    }

    pub fn redact_bytes(&self, bytes: &[u8]) -> Vec<u8> {
        if !self.enabled {
            return bytes.to_vec();
        }
        match std::str::from_utf8(bytes) {
            Ok(text) => self.redact(text).into_bytes(),
            Err(_) => bytes.to_vec(),
        }
    }

    pub fn redact_value(&self, value: &mut Value) {
        if !self.enabled {
            return;
        }
        match value {
            Value::String(text) => {
                *text = self.redact(text);
            }
            Value::Array(items) => {
                for item in items {
                    self.redact_value(item);
                }
            }
            Value::Object(map) => {
                for (key, item) in map.iter_mut() {
                    if is_sensitive_key(key) {
                        if !matches!(item, Value::Bool(_) | Value::Null) {
                            *item = Value::String("[REDACTED]".to_string());
                        }
                    } else {
                        self.redact_value(item);
                    }
                }
            }
            _ => {}
        }
    }

    /// Finds suspected secrets without ever retaining the matched value.
    ///
    /// Input is truncated at [`SECRET_SCAN_MAX_BYTES`] and output is capped at
    /// [`SECRET_SCAN_MAX_FINDINGS`] matches. Overlapping matches are collapsed
    /// in favour of the earlier (more specific) built-in rule.
    pub fn scan_secrets(&self, text: &str) -> Vec<SecretFinding> {
        if self.rules.is_empty() {
            return Vec::new();
        }

        let mut limit = text.len().min(SECRET_SCAN_MAX_BYTES);
        while limit > 0 && !text.is_char_boundary(limit) {
            limit -= 1;
        }
        let scanned = &text[..limit];
        let bytes = scanned.as_bytes();

        let mut candidates: Vec<(usize, usize, usize, SecretKind)> = Vec::new();
        for (rule_index, rule) in self.rules.iter().enumerate() {
            for found in rule.regex.find_iter(scanned) {
                if found.start() < found.end() {
                    candidates.push((found.start(), found.end(), rule_index, rule.kind));
                }
            }
        }
        // Sort by position, then by rule order so named key/value rules win
        // over the bare provider-token rules that match the same bytes.
        candidates.sort_unstable_by_key(|candidate| (candidate.0, candidate.2));

        let mut findings = Vec::new();
        let mut last_end = 0usize;
        let mut cursor = 0usize;
        let mut line = 1usize;
        for (start, end, _, kind) in candidates {
            if findings.len() >= SECRET_SCAN_MAX_FINDINGS {
                break;
            }
            if start < last_end {
                continue;
            }
            for byte in &bytes[cursor..start] {
                if *byte == b'\n' {
                    line += 1;
                }
            }
            let finding_line = line;
            for byte in &bytes[start..end] {
                if *byte == b'\n' {
                    line += 1;
                }
            }
            cursor = end;
            last_end = end;
            findings.push(SecretFinding {
                kind,
                start,
                end,
                line: finding_line,
                preview: kind.redacted_preview(end - start),
            });
        }
        findings
    }
}

/// Reusable, always-enabled secret scanner.
///
/// Uses the same patterns as redaction so a finding can always be redacted.
#[derive(Debug, Clone, Default)]
pub struct SecretScanner {
    redactor: Redactor,
}

impl SecretScanner {
    /// Builds a scanner with additional caller-supplied patterns.
    pub fn new(extra_patterns: &[String]) -> Self {
        Self {
            redactor: Redactor::new(true, extra_patterns, &[]),
        }
    }

    pub fn scan(&self, text: &str) -> Vec<SecretFinding> {
        self.redactor.scan_secrets(text)
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', ' '], "_");
    matches!(
        normalized.as_str(),
        "password" | "passwd" | "pwd" | "secret" | "token" | "api_key" | "apikey" | "authorization"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_common_secret_shapes() {
        let redactor = Redactor::default();
        let text = "token=super-secret password: hunter2 Authorization: Bearer abcdefghijklmnop";
        let redacted = redactor.redact(text);
        assert!(!redacted.contains("super-secret"));
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("abcdefghijklmnop"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn preserves_ordinary_log_lines() {
        let redactor = Redactor::default();
        let text = "I/ActivityManager: Start proc com.example for activity";
        assert_eq!(redactor.redact(text), text);
    }

    #[test]
    fn preserves_boolean_password_attributes() {
        let redactor = Redactor::default();
        let xml = r#"<node password="false" focused="true" />"#;
        assert_eq!(redactor.redact(xml), xml);
        assert!(redactor.redact("password=hunter2").contains("[REDACTED]"));
    }

    #[test]
    fn redacts_json_style_secrets() {
        let redactor = Redactor::default();
        let redacted = redactor.redact(r#"{"password":"hunter2","token":"abc123"}"#);
        assert!(!redacted.contains("hunter2"), "{redacted}");
        assert!(!redacted.contains("abc123"), "{redacted}");
        assert!(redacted.contains("[REDACTED]"));

        let spaced = redactor.redact(r#"{"token": "abc def ghi"}"#);
        assert!(!spaced.contains("abc def ghi"), "{spaced}");
        assert!(spaced.contains("\"token\": \"[REDACTED]\""), "{spaced}");

        assert_eq!(
            redactor.redact("password=BODYSECRET&note=visible"),
            "password=[REDACTED]&note=visible"
        );

        let mut value = serde_json::json!({
            "password": "hunter2",
            "nested": {"token": "abc123", "safe": "visible"},
            "password_enabled": false
        });
        redactor.redact_value(&mut value);
        assert_eq!(value["password"], "[REDACTED]");
        assert_eq!(value["nested"]["token"], "[REDACTED]");
        assert_eq!(value["nested"]["safe"], "visible");
        assert_eq!(value["password_enabled"], false);
    }

    #[test]
    fn supports_configured_patterns() {
        let redactor = Redactor::new(true, &[], &[r"my-private-[0-9]+".to_string()]);
        assert_eq!(
            redactor.redact("value my-private-123 here"),
            "value [REDACTED] here"
        );
    }

    fn google_key() -> String {
        format!("AIza{}", "x".repeat(35))
    }

    fn openai_key() -> String {
        format!("sk-{}", "A".repeat(32))
    }

    #[test]
    fn redacts_google_and_openai_keys() {
        let redactor = Redactor::default();
        let google = google_key();
        let openai = openai_key();
        let redacted = redactor.redact(&format!("google={google} openai={openai}"));
        assert!(!redacted.contains(&google), "{redacted}");
        assert!(!redacted.contains(&openai), "{redacted}");
        assert!(redacted.contains("google=[REDACTED]"), "{redacted}");
        assert!(redacted.contains("openai=[REDACTED]"), "{redacted}");
    }

    #[test]
    fn key_patterns_ignore_short_and_embedded_tokens() {
        let redactor = Redactor::default();
        let short_google = format!("AIza{}", "x".repeat(29));
        let short_openai = format!("sk-{}", "x".repeat(19));
        let samples = [
            "AIzaShort",
            short_google.as_str(),
            "sk-short",
            short_openai.as_str(),
            // Contains "sk-" but not at a word boundary.
            "task-abcdefghijklmnopqrstuvwxyz",
            "I/ActivityManager: Start proc com.example for activity",
        ];
        for sample in samples {
            assert_eq!(redactor.redact(sample), sample, "sample: {sample}");
        }
    }

    #[test]
    fn scanner_reports_kind_range_line_and_safe_preview() {
        let aws = "AKIAIOSFODNN7EXAMPLE";
        let github = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let google = google_key();
        let openai = openai_key();
        let text = format!("header\nsecond line {aws}\n{github} {google} {openai}\n");

        let findings = Redactor::default().scan_secrets(&text);
        assert_eq!(findings.len(), 4, "{findings:?}");
        let expected = [
            (SecretKind::AwsAccessKey, 2usize, aws),
            (SecretKind::GitHubToken, 3, github),
            (SecretKind::GoogleApiKey, 3, google.as_str()),
            (SecretKind::OpenAiKey, 3, openai.as_str()),
        ];
        for (finding, (kind, line, value)) in findings.iter().zip(expected) {
            assert_eq!(finding.kind, kind, "{finding:?}");
            assert_eq!(finding.line, line, "{finding:?}");
            assert_eq!(&text[finding.byte_range()], value, "{finding:?}");
            assert!(!finding.preview.contains(value), "{finding:?}");
            assert!(finding.preview.len() <= 64, "{finding:?}");
        }
    }

    #[test]
    fn scanner_is_bounded() {
        let repeated = format!("{} ", "AKIAIOSFODNN7EXAMPLE").repeat(SECRET_SCAN_MAX_FINDINGS + 50);
        let findings = SecretScanner::default().scan(&repeated);
        assert_eq!(findings.len(), SECRET_SCAN_MAX_FINDINGS);

        let mut oversized = "safe log line\n".repeat(SECRET_SCAN_MAX_BYTES / 14 + 2);
        oversized.push_str(&google_key());
        assert!(SecretScanner::default().scan(&oversized).is_empty());
    }

    #[test]
    fn scanner_ignores_non_secrets() {
        let scanner = SecretScanner::default();
        assert!(
            scanner
                .scan("I/ActivityManager: Start proc com.example for activity")
                .is_empty()
        );
        assert!(scanner.scan("task-abcdefghijklmnopqrstuvwxyz").is_empty());
        assert!(scanner.scan(&format!("AIza{}", "x".repeat(29))).is_empty());
        assert!(scanner.scan(&format!("sk-{}", "x".repeat(19))).is_empty());
    }

    #[test]
    fn redaction_preserves_non_secret_artifact_bytes() {
        let redactor = Redactor::default();
        let text = "line one\nline two\nI/ActivityManager: Start proc com.example\n";
        assert_eq!(redactor.redact_bytes(text.as_bytes()), text.as_bytes());

        let google = google_key();
        let mixed = format!("plain prefix {google} plain suffix");
        let redacted = redactor.redact(&mixed);
        assert!(
            redacted.starts_with("plain prefix [REDACTED] plain suffix"),
            "{redacted}"
        );
        assert!(!redacted.contains(&google));
    }
}
