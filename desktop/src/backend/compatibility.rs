use std::path::PathBuf;

use super::cli_runner::{CliRunner, Invocation};
use super::json_parser::parse_result;
use super::{DesktopError, Result};
use crate::models::Envelope;

#[derive(Debug, Clone)]
pub struct CliInfo {
    pub path: PathBuf,
    pub version: String,
    pub report_schema: u32,
}

pub fn probe(runner: &CliRunner) -> Result<CliInfo> {
    let expected = env!("CARGO_PKG_VERSION").to_string();
    let version = runner.version()?;
    if version != expected {
        return Err(DesktopError::VersionMismatch {
            expected,
            actual: version,
        });
    }

    let output = runner.run_success(
        &Invocation::new(["schema", "list"]).timeout(std::time::Duration::from_secs(15)),
    )?;
    let envelope: Envelope = output.envelope;
    let result: SchemaList = parse_result(&envelope)?;
    if !result
        .schemas
        .iter()
        .any(|schema| schema == "androidforge.schema.json")
    {
        return Err(DesktopError::UnsupportedSchema {
            schema: envelope.schema_version,
        });
    }

    Ok(CliInfo {
        path: runner.cli_path.clone(),
        version,
        report_schema: 1,
    })
}

#[derive(Debug, serde::Deserialize)]
struct SchemaList {
    #[serde(default)]
    schemas: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_list_shape_is_optional_but_parsed() {
        let value = serde_json::json!({"schemas":["androidforge.schema.json"]});
        let list: SchemaList = serde_json::from_value(value).unwrap();
        assert_eq!(list.schemas.len(), 1);
    }
}
