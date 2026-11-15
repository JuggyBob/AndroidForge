/// Decode arbitrary device/process output without touching the console code page.
///
/// Android tools emit UTF-8. A small number of third-party tools emit arbitrary
/// bytes; those are replaced with U+FFFD rather than being interpreted through a
/// locale-dependent Windows code page.
pub fn decode_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Backwards-compatible alias retained for callers that only need a String.
pub fn decode_lossless(bytes: &[u8]) -> String {
    decode_text(bytes)
}

/// Best-effort detection used when deciding whether stdout can be placed in a
/// JSON string or must be base64 encoded.
pub fn looks_like_text(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return false;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\u{1b}')),
        Err(_) => false,
    }
}
