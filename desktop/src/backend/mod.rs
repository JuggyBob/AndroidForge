use std::path::PathBuf;

use thiserror::Error;

pub mod cli_locator;
pub mod cli_runner;
pub mod compatibility;
pub mod evidence;
pub mod json_parser;
pub mod process_manager;
pub mod project;
pub mod update;

pub type Result<T> = std::result::Result<T, DesktopError>;

#[derive(Debug, Error)]
pub enum DesktopError {
    #[error("AndroidForge CLI was not found")]
    CliNotFound { searched: Vec<PathBuf> },

    #[error("could not start AndroidForge CLI at {path}: {source}")]
    CliSpawn {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("AndroidForge CLI did not finish within {seconds}s")]
    CliTimeout { seconds: u64 },

    #[error("AndroidForge CLI exited with code {code}")]
    CliExit { code: i32, stderr: String },

    #[error("AndroidForge CLI returned invalid JSON: {message}")]
    InvalidJson { message: String, raw: String },

    #[error("AndroidForge CLI protocol is not supported (schema {schema})")]
    UnsupportedSchema { schema: u32 },

    #[error("desktop expects AndroidForge {expected}, but the configured CLI is {actual}")]
    VersionMismatch { expected: String, actual: String },

    #[error("AndroidForge command failed: {code}: {message}")]
    CliError {
        code: String,
        message: String,
        raw: String,
    },

    #[error("path is outside the selected run directory")]
    PathOutsideRun,

    #[error("network request failed: {0}")]
    Network(String),

    #[error("release asset verification failed: {0}")]
    Verification(String),

    #[error("update failed: {0}")]
    Update(String),

    #[error("evidence file exceeds the preview limit")]
    EvidenceTooLarge,

    #[error("invalid path: {0}")]
    InvalidPath(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),
}
