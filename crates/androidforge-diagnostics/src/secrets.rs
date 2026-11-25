//! Bounded secret scanning for diagnostic text and stored artifacts.
//!
//! The scanner reports only location metadata and category-derived previews;
//! it never returns the matched secret value.

use androidforge_core::redact::SecretScanner;

pub use androidforge_core::redact::{
    SECRET_SCAN_MAX_BYTES, SECRET_SCAN_MAX_FINDINGS, SecretFinding, SecretKind,
};

/// Finds suspected secrets in `text`.
///
/// Input is capped at [`SECRET_SCAN_MAX_BYTES`] and at most
/// [`SECRET_SCAN_MAX_FINDINGS`] findings are returned.
pub fn scan_secrets(text: &str) -> Vec<SecretFinding> {
    SecretScanner::default().scan(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn google_key() -> String {
        format!("AIza{}", "x".repeat(35))
    }

    fn openai_key() -> String {
        format!("sk-{}", "a".repeat(30))
    }

    #[test]
    fn detects_provider_keys() {
        let aws = "AKIAIOSFODNN7EXAMPLE";
        let github = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let google = google_key();
        let openai = openai_key();
        let text = format!("line one\n{aws}\n{github} {google} {openai}\n");

        let findings = scan_secrets(&text);
        assert_eq!(findings.len(), 4, "{findings:?}");
        let kinds: Vec<SecretKind> = findings.iter().map(|finding| finding.kind).collect();
        assert_eq!(
            kinds,
            vec![
                SecretKind::AwsAccessKey,
                SecretKind::GitHubToken,
                SecretKind::GoogleApiKey,
                SecretKind::OpenAiKey,
            ]
        );
        assert_eq!(findings[0].line, 2);
        for finding in &findings {
            let matched = &text[finding.byte_range()];
            assert!(!finding.preview.contains(matched), "{finding:?}");
            assert!(!finding.preview.is_empty());
        }
    }

    #[test]
    fn ignores_non_secrets() {
        let samples = [
            "I/ActivityManager: Start proc com.example for activity",
            "material design component",
            "task-abcdefghijklmnopqrstuvwxyz",
            "AIzaShort",
            "sk-short",
        ];
        for sample in samples {
            assert!(scan_secrets(sample).is_empty(), "sample: {sample}");
        }
    }

    #[test]
    fn redaction_removes_what_scanning_reports() {
        let redactor = androidforge_core::redact::Redactor::default();
        let google = google_key();
        let openai = openai_key();
        let text = format!("artifact line: {google} and {openai}");
        assert_eq!(scan_secrets(&text).len(), 2);
        let redacted = redactor.redact(&text);
        assert!(scan_secrets(&redacted).is_empty(), "{redacted}");
        assert!(
            redacted.starts_with("artifact line: [REDACTED] and [REDACTED]"),
            "{redacted}"
        );
    }
}
