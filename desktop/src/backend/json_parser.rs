use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{DesktopError, Result};
use crate::models::Envelope;

pub const REPORT_SCHEMA_VERSION: u32 = 1;
pub const ENVELOPE_SCHEMA_VERSION: u32 = 1;

pub fn parse_envelope(raw: &str, stderr: &str) -> Result<Envelope> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(DesktopError::InvalidJson {
            message: if stderr.trim().is_empty() {
                "stdout was empty".to_string()
            } else {
                stderr.trim().to_string()
            },
            raw: raw.to_string(),
        });
    }
    let envelope: Envelope =
        serde_json::from_str(trimmed).map_err(|error| DesktopError::InvalidJson {
            message: error.to_string(),
            raw: raw.to_string(),
        })?;
    if envelope.schema_version != ENVELOPE_SCHEMA_VERSION {
        return Err(DesktopError::UnsupportedSchema {
            schema: envelope.schema_version,
        });
    }
    Ok(envelope)
}

pub fn parse_result<T: DeserializeOwned>(envelope: &Envelope) -> Result<T> {
    let value = envelope.result.clone().unwrap_or(Value::Null);
    serde_json::from_value(value).map_err(DesktopError::Json)
}

pub fn error_from_envelope(raw: &str, envelope: &Envelope) -> DesktopError {
    match &envelope.error {
        Some(error) => DesktopError::CliError {
            code: error.code.clone(),
            message: error.message.clone(),
            raw: raw.to_string(),
        },
        None => DesktopError::CliExit {
            code: envelope.exit_code,
            stderr: raw.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_success_envelope() {
        let raw = r#"{"schema_version":1,"status":"success","command":"devices.list","exit_code":0,"result":{"count":0,"devices":[]},"duration_ms":12}"#;
        let envelope = parse_envelope(raw, "").unwrap();
        assert_eq!(envelope.command, "devices.list");
        assert_eq!(
            parse_result::<crate::models::DeviceList>(&envelope)
                .unwrap()
                .count,
            0
        );
    }

    #[test]
    fn rejects_unsupported_schema() {
        let raw = r#"{"schema_version":2,"status":"success","exit_code":0}"#;
        assert!(matches!(
            parse_envelope(raw, ""),
            Err(DesktopError::UnsupportedSchema { schema: 2 })
        ));
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(matches!(
            parse_envelope("not json", "boom"),
            Err(DesktopError::InvalidJson { .. })
        ));
    }
}
