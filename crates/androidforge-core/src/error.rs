use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::model::ArtifactRef;

macro_rules! error_codes {
    ($( $variant:ident => ($code:literal, $exit:literal, $recoverable:literal) ),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum ErrorCode {
            $($variant),*
        }

        impl ErrorCode {
            pub const ALL: &'static [ErrorCode] = &[$(Self::$variant),*];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $code),*
                }
            }

            pub fn exit_code(self) -> i32 {
                match self {
                    $(Self::$variant => $exit),*
                }
            }

            pub fn default_recoverable(self) -> bool {
                match self {
                    $(Self::$variant => $recoverable),*
                }
            }

            pub fn from_code(code: &str) -> Option<Self> {
                match code {
                    $($code => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

error_codes! {
    InvalidCommand => ("E_INVALID_COMMAND", 2, true),
    InvalidArgument => ("E_INVALID_ARGUMENT", 2, true),
    DeviceNotFound => ("E_DEVICE_NOT_FOUND", 4, true),
    DeviceAmbiguous => ("E_DEVICE_AMBIGUOUS", 4, true),
    DeviceOffline => ("E_DEVICE_OFFLINE", 4, true),
    DeviceUnauthorized => ("E_DEVICE_UNAUTHORIZED", 4, true),
    DeviceBusy => ("E_DEVICE_BUSY", 4, true),
    DeviceDisconnected => ("E_DEVICE_DISCONNECTED", 4, true),
    EmulatorNotFound => ("E_EMULATOR_NOT_FOUND", 4, true),
    EmulatorBootTimeout => ("E_EMULATOR_BOOT_TIMEOUT", 5, true),
    SdkNotFound => ("E_SDK_NOT_FOUND", 3, true),
    AdbNotFound => ("E_ADB_NOT_FOUND", 3, true),
    ToolNotFound => ("E_TOOL_NOT_FOUND", 3, true),
    MaestroNotFound => ("E_MAESTRO_NOT_FOUND", 3, true),
    CommandTimeout => ("E_COMMAND_TIMEOUT", 5, true),
    CommandFailed => ("E_COMMAND_FAILED", 6, true),
    PackageNotFound => ("E_PACKAGE_NOT_FOUND", 6, true),
    FileNotFound => ("E_FILE_NOT_FOUND", 6, true),
    AppNotDebuggable => ("E_APP_NOT_DEBUGGABLE", 6, true),
    InstallFailed => ("E_INSTALL_FAILED", 6, true),
    InstallSignatureMismatch => ("E_INSTALL_SIGNATURE_MISMATCH", 6, true),
    InstallVerificationFailed => ("E_INSTALL_VERIFICATION_FAILED", 6, true),
    ApkInvalid => ("E_APK_INVALID", 6, false),
    SignatureVerificationFailed => ("E_SIGNATURE_VERIFICATION_FAILED", 7, false),
    SelectorNotFound => ("E_SELECTOR_NOT_FOUND", 7, true),
    UiHierarchyInvalid => ("E_UI_HIERARCHY_INVALID", 6, true),
    UiAutomatorUnavailable => ("E_UI_AUTOMATOR_UNAVAILABLE", 6, true),
    SelectorAmbiguous => ("E_SELECTOR_AMBIGUOUS", 7, true),
    UiAssertionFailed => ("E_UI_ASSERTION_FAILED", 7, true),
    PostconditionTimeout => ("E_POSTCONDITION_TIMEOUT", 7, true),
    ObservationNotFound => ("E_OBSERVATION_NOT_FOUND", 4, true),
    ObservationExpired => ("E_OBSERVATION_EXPIRED", 4, true),
    ObservationSessionMismatch => ("E_OBSERVATION_SESSION_MISMATCH", 4, true),
    ElementRefUnknown => ("E_ELEMENT_REF_UNKNOWN", 4, true),
    ElementRefStale => ("E_ELEMENT_REF_STALE", 4, true),
    ElementIdentityChanged => ("E_ELEMENT_IDENTITY_CHANGED", 4, true),
    ElementNotActionable => ("E_ELEMENT_NOT_ACTIONABLE", 7, true),
    ElementDeviceMismatch => ("E_ELEMENT_DEVICE_MISMATCH", 4, true),
    WirelessEndpointInvalid => ("E_WIRELESS_ENDPOINT_INVALID", 2, true),
    WirelessPairingFailed => ("E_WIRELESS_PAIRING_FAILED", 4, true),
    WirelessConnectFailed => ("E_WIRELESS_CONNECT_FAILED", 4, true),
    DeviceHealthUnavailable => ("E_DEVICE_HEALTH_UNAVAILABLE", 4, true),
    RetryNotAllowed => ("E_RETRY_NOT_ALLOWED", 2, true),
    MatrixTargetInvalid => ("E_MATRIX_TARGET_INVALID", 2, true),
    MatrixDuplicateTarget => ("E_MATRIX_DUPLICATE_TARGET", 2, true),
    LeaseExpired => ("E_LEASE_EXPIRED", 4, true),
    TextVerificationFailed => ("E_TEXT_VERIFICATION_FAILED", 7, true),
    ImeVisible => ("E_IME_VISIBLE", 7, true),
    PermissionDenied => ("E_PERMISSION_DENIED", 6, true),
    SnapshotNotFound => ("E_SNAPSHOT_NOT_FOUND", 4, true),
    SessionExpired => ("E_SESSION_EXPIRED", 4, true),
    SessionOwnershipMismatch => ("E_SESSION_OWNERSHIP_MISMATCH", 4, true),
    SessionProjectMismatch => ("E_SESSION_PROJECT_MISMATCH", 4, true),
    SessionNotFound => ("E_SESSION_NOT_FOUND", 4, true),
    ArtifactWriteFailed => ("E_ARTIFACT_WRITE_FAILED", 9, true),
    ConfigInvalid => ("E_CONFIG_INVALID", 8, false),
    AdapterInvalid => ("E_ADAPTER_INVALID", 8, false),
    FixtureInvalid => ("E_FIXTURE_INVALID", 8, false),
    FixtureFailed => ("E_FIXTURE_FAILED", 6, true),
    HookNotFound => ("E_HOOK_NOT_FOUND", 3, true),
    HookVerificationFailed => ("E_HOOK_VERIFICATION_FAILED", 7, true),
    JunitInvalid => ("E_JUNIT_INVALID", 8, false),
    UnsupportedOperation => ("E_UNSUPPORTED_OPERATION", 3, true),
    PartialFailure => ("E_PARTIAL_FAILURE", 10, true),
    Interrupted => ("E_INTERRUPTED", 130, true),
    Internal => ("E_INTERNAL", 9, false),
}

impl Serialize for ErrorCode {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_code(&value)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown error code {value}")))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default)]
    pub recoverable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
}

impl ErrorBody {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        let recoverable = code.default_recoverable();
        Self {
            code,
            message: message.into(),
            recoverable,
            suggestions: Vec::new(),
            context: BTreeMap::new(),
            command: None,
            device_serial: None,
            session_id: None,
            cause: None,
            artifacts: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct ForgeError {
    body: Box<ErrorBody>,
    exit_code_override: Option<i32>,
}

pub type Result<T> = std::result::Result<T, ForgeError>;

impl ForgeError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            body: Box::new(ErrorBody::new(code, message)),
            exit_code_override: None,
        }
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }

    pub fn invalid_command(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidCommand, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::UnsupportedOperation, message)
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        let suggestion = suggestion.into();
        if !self.body.suggestions.contains(&suggestion) {
            self.body.suggestions.push(suggestion);
        }
        self
    }

    pub fn with_suggestions<I, S>(mut self, suggestions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for suggestion in suggestions {
            self = self.with_suggestion(suggestion);
        }
        self
    }

    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.body.context.insert(key.into(), value.into());
        self
    }

    pub fn with_context_string(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.body
            .context
            .insert(key.into(), Value::String(value.into()));
        self
    }

    pub fn with_cause(mut self, cause: impl fmt::Display) -> Self {
        self.body.cause = Some(cause.to_string());
        self
    }

    pub fn recoverable(mut self, recoverable: bool) -> Self {
        self.body.recoverable = recoverable;
        self
    }

    pub fn command(mut self, command: impl Into<String>) -> Self {
        self.body.command = Some(command.into());
        self
    }

    pub fn device(mut self, serial: impl Into<String>) -> Self {
        self.body.device_serial = Some(serial.into());
        self
    }

    pub fn session(mut self, session_id: impl Into<String>) -> Self {
        self.body.session_id = Some(session_id.into());
        self
    }

    pub fn with_artifacts(mut self, artifacts: Vec<ArtifactRef>) -> Self {
        self.body.artifacts = artifacts;
        self
    }

    pub fn with_exit_code(mut self, exit_code: i32) -> Self {
        self.exit_code_override = Some(exit_code);
        self
    }

    pub fn code(&self) -> ErrorCode {
        self.body.code
    }

    pub fn message(&self) -> &str {
        &self.body.message
    }

    pub fn body(&self) -> &ErrorBody {
        &self.body
    }

    pub fn body_mut(&mut self) -> &mut ErrorBody {
        &mut self.body
    }

    pub fn into_body(self) -> ErrorBody {
        *self.body
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code_override
            .unwrap_or_else(|| self.body.code.exit_code())
    }

    pub fn interrupted() -> Self {
        Self::new(ErrorCode::Interrupted, "Operation interrupted").with_suggestion(
            "Inspect the run report and artifacts before retrying; device state may have changed.",
        )
    }
}

impl fmt::Display for ForgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.body.code.as_str(), self.body.message)
    }
}

impl std::error::Error for ForgeError {}

impl From<androidforge_process::ProcessError> for ForgeError {
    fn from(error: androidforge_process::ProcessError) -> Self {
        use androidforge_process::ProcessError as Process;
        match error {
            Process::Timeout {
                program,
                timeout_ms,
                stderr,
                ..
            } => {
                let mut forge = ForgeError::new(
                    ErrorCode::CommandTimeout,
                    format!("{program} did not finish within {timeout_ms} ms"),
                )
                .with_context_string("program", program)
                .with_suggestion("Retry with a larger --timeout after checking the device state.");
                let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
                if !stderr.is_empty() {
                    forge = forge.with_context_string("stderr", stderr);
                }
                forge
            }
            Process::Failed {
                program,
                exit_code,
                stderr,
                stdout,
                ..
            } => {
                let exit = exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let mut forge = ForgeError::new(
                    ErrorCode::CommandFailed,
                    format!("{program} exited with code {exit}"),
                )
                .with_context_string("program", program)
                .with_context_string("exit_code", exit);
                let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&stdout).trim().to_string();
                if !stderr.is_empty() {
                    forge = forge.with_context_string("stderr", stderr);
                }
                if !stdout.is_empty() {
                    forge = forge.with_context_string("stdout", stdout);
                }
                forge
            }
            Process::Spawn { program, source } => {
                let suggestion = format!("Verify that {program} exists and is executable.");
                ForgeError::new(
                    ErrorCode::CommandFailed,
                    format!("could not start {program}: {source}"),
                )
                .with_context_string("program", program)
                .with_suggestion(suggestion)
            }
            Process::Io { program, source } => ForgeError::new(
                ErrorCode::CommandFailed,
                format!("I/O error while running {program}: {source}"),
            )
            .with_context_string("program", program),
        }
    }
}

impl From<std::io::Error> for ForgeError {
    fn from(error: std::io::Error) -> Self {
        ForgeError::new(ErrorCode::Internal, error.to_string()).with_cause(error)
    }
}

impl From<toml::de::Error> for ForgeError {
    fn from(error: toml::de::Error) -> Self {
        ForgeError::new(ErrorCode::ConfigInvalid, error.to_string()).with_cause(error)
    }
}

impl From<serde_json::Error> for ForgeError {
    fn from(error: serde_json::Error) -> Self {
        ForgeError::new(ErrorCode::Internal, error.to_string()).with_cause(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_round_trip_as_strings() {
        for code in ErrorCode::ALL {
            assert_eq!(ErrorCode::from_code(code.as_str()), Some(*code));
            assert!(code.as_str().starts_with("E_"));
        }
    }

    #[test]
    fn exit_codes_document_matches_the_error_enum() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("EXIT_CODES.md");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let mut documented = std::collections::BTreeSet::new();
        for line in text.lines() {
            let line = line.trim();
            if !line.starts_with("| `E_") {
                continue;
            }
            let parts = line.split('|').map(str::trim).collect::<Vec<_>>();
            if parts.len() < 5 {
                continue;
            }
            let code = parts[1].trim_matches('`');
            documented.insert(code.to_string());
            let expected = ErrorCode::from_code(code)
                .unwrap_or_else(|| panic!("EXIT_CODES.md documents unknown error code {code}"));
            assert_eq!(
                parts[2],
                expected.exit_code().to_string(),
                "exit code drift for {code}"
            );
            assert_eq!(
                parts[3],
                if expected.default_recoverable() {
                    "yes"
                } else {
                    "no"
                },
                "recoverability drift for {code}"
            );
        }
        for code in ErrorCode::ALL {
            assert!(
                documented.contains(code.as_str()),
                "EXIT_CODES.md is missing {}",
                code.as_str()
            );
        }
    }

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(ErrorCode::InvalidArgument.exit_code(), 2);
        assert_eq!(ErrorCode::AdbNotFound.exit_code(), 3);
        assert_eq!(ErrorCode::DeviceBusy.exit_code(), 4);
        assert_eq!(ErrorCode::CommandTimeout.exit_code(), 5);
        assert_eq!(ErrorCode::CommandFailed.exit_code(), 6);
        assert_eq!(ErrorCode::SelectorAmbiguous.exit_code(), 7);
        assert_eq!(ErrorCode::ConfigInvalid.exit_code(), 8);
        assert_eq!(ErrorCode::Internal.exit_code(), 9);
        assert_eq!(ErrorCode::PartialFailure.exit_code(), 10);
        assert_eq!(ErrorCode::Interrupted.exit_code(), 130);
    }
}
