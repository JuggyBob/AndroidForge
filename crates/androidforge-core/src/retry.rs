//! Central policy for bounded retries of read-only operations.
//!
//! Retries are opt-in, bounded and never applied to actions that change device
//! or application state. The CLI exposes `--retries` only on commands in the
//! eligible list; this module is the single place that classifies errors and
//! computes delays.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, ForgeError, Result};

pub const MAX_RETRIES: u32 = 5;
pub const MAX_TOTAL_RETRY_WINDOW: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadOnlyOperation {
    UiObserve,
    UiDiffCapture,
    LogcatPeek,
    LogcatTail,
    DeviceInspect,
    AppStatus,
}

impl ReadOnlyOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UiObserve => "ui.observe",
            Self::UiDiffCapture => "ui.diff_capture",
            Self::LogcatPeek => "diagnostics.logcat_peek",
            Self::LogcatTail => "diagnostics.logcat_tail",
            Self::DeviceInspect => "devices.inspect",
            Self::AppStatus => "app.status",
        }
    }

    /// All currently exposed retryable operations are read-only. This remains
    /// an explicit allow-list so a future state-changing command cannot inherit
    /// retries merely by exposing the same flag.
    pub fn is_eligible(self) -> bool {
        matches!(
            self,
            Self::UiObserve
                | Self::UiDiffCapture
                | Self::LogcatPeek
                | Self::LogcatTail
                | Self::DeviceInspect
                | Self::AppStatus
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryAttempt {
    pub attempt: u32,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub retryable: bool,
    pub delay_before_next_ms: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryReport {
    pub operation: String,
    pub requested_retries: u32,
    pub max_attempts: u32,
    pub attempts: Vec<RetryAttempt>,
    pub retries_used: u32,
    pub total_elapsed_ms: u64,
    pub final_result: String,
}

impl RetryReport {
    pub fn retried(&self) -> bool {
        self.retries_used > 0
    }
}

pub fn validate_retry_request(operation: ReadOnlyOperation, retries: u32) -> Result<()> {
    if !operation.is_eligible() {
        return Err(ForgeError::new(
            ErrorCode::RetryNotAllowed,
            format!(
                "retries are disabled for {} because it is not a read-only operation",
                operation.as_str()
            ),
        )
        .with_suggestion(
            "Retry only read-only operations; state-changing actions are dispatched at most once.",
        ));
    }
    if retries > MAX_RETRIES {
        return Err(ForgeError::invalid_argument(format!(
            "--retries {retries} exceeds the conservative maximum of {MAX_RETRIES}"
        ))
        .with_suggestion("Use a value between 0 and 5."));
    }
    Ok(())
}

/// Returns true only for transient failures where repeating a read is safe.
pub fn is_retryable(error: &ForgeError) -> bool {
    match error.code() {
        ErrorCode::DeviceDisconnected
        | ErrorCode::DeviceOffline
        | ErrorCode::CommandTimeout
        | ErrorCode::UiAutomatorUnavailable => true,
        ErrorCode::CommandFailed | ErrorCode::Internal => {
            let message = error.message().to_ascii_lowercase();
            [
                "device offline",
                "device disconnected",
                "connection reset",
                "connection closed",
                "closed",
                "timed out",
                "timeout",
                "uiautomator",
                "helper",
                "temporarily unavailable",
            ]
            .iter()
            .any(|needle| message.contains(needle))
        }
        _ => false,
    }
}

pub fn delay_for(attempt: u32) -> Duration {
    let exponent = attempt.min(4);
    let millis = 250u64.saturating_mul(1u64 << exponent);
    Duration::from_millis(millis.min(2_000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_read_only_operations_are_eligible() {
        assert!(ReadOnlyOperation::UiObserve.is_eligible());
        assert!(ReadOnlyOperation::LogcatPeek.is_eligible());
        assert!(ReadOnlyOperation::UiDiffCapture.is_eligible());
        assert!(validate_retry_request(ReadOnlyOperation::UiObserve, 2).is_ok());
        assert!(validate_retry_request(ReadOnlyOperation::UiObserve, MAX_RETRIES + 1).is_err());
    }

    #[test]
    fn transient_errors_are_retryable_and_selectors_are_not() {
        assert!(is_retryable(&ForgeError::new(
            ErrorCode::UiAutomatorUnavailable,
            "uiautomator helper failed"
        )));
        assert!(is_retryable(&ForgeError::new(
            ErrorCode::CommandFailed,
            "device offline while reading hierarchy"
        )));
        assert!(!is_retryable(&ForgeError::new(
            ErrorCode::SelectorNotFound,
            "no element matched"
        )));
        assert!(!is_retryable(&ForgeError::new(
            ErrorCode::InvalidArgument,
            "timeout must be positive"
        )));
    }

    #[test]
    fn backoff_is_bounded() {
        assert_eq!(delay_for(0), Duration::from_millis(250));
        assert_eq!(delay_for(1), Duration::from_millis(500));
        assert_eq!(delay_for(4), Duration::from_millis(2_000));
        assert_eq!(delay_for(99), Duration::from_millis(2_000));
    }
}
