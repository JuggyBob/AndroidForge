use regex::{Regex, RegexBuilder};
use serde_json::Value;

const DEFAULT_PATTERNS: &[(&str, bool)] = &[
    // Boolean XML/JSON attributes named "password" are structural, not secrets.
    (
        r#"(?i)((?:password|passwd|pwd)\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s,;]+)"#,
        true,
    ),
    (
        r"(?i)((?:secret|token|api[_-]?key|authorization)\s*[:=]\s*(?:bearer\s+)?)([^\s,;]+)",
        false,
    ),
    (r"(?i)((?:bearer\s+)[A-Za-z0-9._~+/=-]{8,})", false),
    (
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        false,
    ),
    (r"\bAKIA[0-9A-Z]{16}\b", false),
    (r"\bgh[pousr]_[A-Za-z0-9]{20,}\b", false),
    (r"\bgithub_pat_[A-Za-z0-9_]{20,}\b", false),
];

#[derive(Debug, Clone)]
struct RedactionRule {
    regex: Regex,
    preserve_boolean_values: bool,
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
            for (pattern, preserve_boolean_values) in DEFAULT_PATTERNS {
                if let Ok(regex) = RegexBuilder::new(pattern).build() {
                    rules.push(RedactionRule {
                        regex,
                        preserve_boolean_values: *preserve_boolean_values,
                    });
                }
            }
            for pattern in configured_patterns.iter().chain(extra_patterns.iter()) {
                if let Ok(regex) = RegexBuilder::new(pattern).build() {
                    rules.push(RedactionRule {
                        regex,
                        preserve_boolean_values: false,
                    });
                }
            }
        }
        Self { enabled, rules }
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
                        format!("{}[REDACTED]", &caps[1])
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
                for item in map.values_mut() {
                    self.redact_value(item);
                }
            }
            _ => {}
        }
    }
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
    fn supports_configured_patterns() {
        let redactor = Redactor::new(true, &[], &[r"my-private-[0-9]+".to_string()]);
        assert_eq!(
            redactor.redact("value my-private-123 here"),
            "value [REDACTED] here"
        );
    }
}
