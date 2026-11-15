/// Quote one argument for a POSIX shell used by `adb shell`.
///
/// Android's `adb shell` joins arguments and hands the resulting string to the
/// device shell. Every remotely executed argument must therefore be quoted by
/// us. This is deliberately a small single-quote implementation rather than a
/// general shell language.
pub fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '_' | '@' | '%' | '+' | '=' | ':' | ',' | '.' | '/' | '-')
    }) {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

pub fn shell_quote_join(args: &[String]) -> String {
    args.iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Convert user text into the value expected by Android's `input text`.
///
/// Spaces become `%s`, which is the encoding understood by `input text`.
/// The result is *not* shell quoted: pass it through the ADB shell argument
/// helper so shell metacharacters in the text are quoted exactly once. Literal
/// newlines, NULs and the literal sequence `%s` cannot be represented reliably
/// by the standard Android input command, so they are reported instead of
/// silently entering the wrong text.
pub fn escape_input_text(text: &str) -> Result<String, String> {
    if text.contains('\0') {
        return Err("NUL characters cannot be entered through adb input".to_string());
    }
    if text.contains('\n') || text.contains('\r') {
        return Err(
            "newlines cannot be entered through the standard adb input command".to_string(),
        );
    }
    if text.contains("%s") {
        return Err(
            "the literal sequence %s is interpreted as a space by Android's input command and cannot be entered verbatim"
                .to_string(),
        );
    }
    Ok(text.replace(' ', "%s"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_shell_metacharacters() {
        assert_eq!(shell_quote("hello"), "hello");
        assert_eq!(shell_quote("hello world"), "'hello world'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_quote("$HOME;rm -rf /"), "'$HOME;rm -rf /'");
    }

    #[test]
    fn spaces_become_input_percent_s() {
        assert_eq!(escape_input_text("hello world").unwrap(), "hello%sworld");
        assert_eq!(escape_input_text("two  spaces").unwrap(), "two%s%sspaces");
    }

    #[test]
    fn rejects_unrepresentable_text() {
        assert!(escape_input_text("line\nbreak").is_err());
        assert!(escape_input_text("literal %s").is_err());
    }
}
