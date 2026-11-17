use std::future::Future;
use std::time::Instant;

use androidforge_core::error::{ForgeError, Result};
use androidforge_core::model::Device;
use androidforge_core::output::CommandSuccess;
use androidforge_core::retry::{
    MAX_TOTAL_RETRY_WINDOW, ReadOnlyOperation, RetryAttempt, RetryReport, delay_for, is_retryable,
    validate_retry_request,
};
use androidforge_core::session::Session;
use androidforge_emulator::DevicePrepareOptions;
use androidforge_ui::Selector;

use crate::cli::{PrepareArgs, SelectorArgs};

pub fn success(
    command: impl Into<String>,
    result: serde_json::Value,
    human: impl Into<String>,
) -> CommandSuccess {
    CommandSuccess::new(command, result, human)
}

pub async fn retry_read_only<F, Fut, T>(
    operation: ReadOnlyOperation,
    retries: u32,
    mut action: F,
) -> Result<(T, RetryReport)>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    validate_retry_request(operation, retries)?;
    let started = Instant::now();
    let mut attempts = Vec::new();
    let mut attempt_number = 1u32;
    loop {
        let attempt_started = Instant::now();
        match action().await {
            Ok(value) => {
                attempts.push(RetryAttempt {
                    attempt: attempt_number,
                    outcome: "success".to_string(),
                    code: None,
                    message: None,
                    retryable: false,
                    delay_before_next_ms: 0,
                    elapsed_ms: attempt_started.elapsed().as_millis() as u64,
                });
                return Ok((
                    value,
                    RetryReport {
                        operation: operation.as_str().to_string(),
                        requested_retries: retries,
                        max_attempts: retries + 1,
                        retries_used: attempt_number - 1,
                        attempts,
                        total_elapsed_ms: started.elapsed().as_millis() as u64,
                        final_result: "success".to_string(),
                    },
                ));
            }
            Err(error) => {
                let retryable = is_retryable(&error);
                let delay = delay_for(attempt_number - 1);
                let can_retry = retryable
                    && attempt_number <= retries
                    && started.elapsed() + delay <= MAX_TOTAL_RETRY_WINDOW;
                attempts.push(RetryAttempt {
                    attempt: attempt_number,
                    outcome: "error".to_string(),
                    code: Some(error.body().code.as_str().to_string()),
                    message: Some(error.message().to_string()),
                    retryable,
                    delay_before_next_ms: if can_retry {
                        delay.as_millis() as u64
                    } else {
                        0
                    },
                    elapsed_ms: attempt_started.elapsed().as_millis() as u64,
                });
                if can_retry {
                    tokio::time::sleep(delay).await;
                    attempt_number += 1;
                    continue;
                }
                let report = RetryReport {
                    operation: operation.as_str().to_string(),
                    requested_retries: retries,
                    max_attempts: retries + 1,
                    retries_used: attempt_number - 1,
                    attempts,
                    total_elapsed_ms: started.elapsed().as_millis() as u64,
                    final_result: "error".to_string(),
                };
                return Err(error.with_context(
                    "retry",
                    serde_json::to_value(&report).unwrap_or(serde_json::Value::Null),
                ));
            }
        }
    }
}

pub fn require_selector(args: &SelectorArgs) -> Result<Selector> {
    if !args.has_selector() {
        return Err(ForgeError::invalid_argument(
            "a UI selector is required (for example --text, --content-desc or --resource-id)",
        )
        .with_suggestion("Run `androidforge ui dump` to inspect the current hierarchy.")
        .with_suggestion("Use --contains for partial text matching."));
    }
    Ok(args.clone().into_selector())
}

pub fn device_json(device: &Device, owner: Option<&Session>) -> serde_json::Value {
    let mut value = serde_json::to_value(device.compact()).unwrap_or(serde_json::Value::Null);
    if let Some(owner) = owner
        && let Some(object) = value.as_object_mut()
    {
        object.insert(
            "owner".to_string(),
            serde_json::json!({
                "session_id": owner.id,
                "owner": owner.owner,
                "kind": owner.kind,
                "state": owner.state,
            }),
        );
    }
    value
}

pub fn device_human(device: &Device, owner: Option<&Session>) -> String {
    let owner = owner
        .map(|session| format!(" owner={} ({})", session.id, session.owner))
        .unwrap_or_else(|| " unowned".to_string());
    format!(
        "{} {} {}{}",
        device.serial,
        device.state.as_str(),
        device.kind_label(),
        owner
    )
}

pub fn prepare_options(args: &PrepareArgs) -> DevicePrepareOptions {
    let any = args.animations
        || args.keep_awake
        || args.wake
        || args.locale.is_some()
        || args.timezone.is_some()
        || args.font_scale.is_some()
        || args.dark_mode.is_some()
        || args.orientation.is_some()
        || args.density.is_some();
    if !any {
        DevicePrepareOptions::default()
    } else {
        DevicePrepareOptions {
            animations: args.animations,
            keep_awake: args.keep_awake,
            wake: args.wake,
            locale: args.locale.clone(),
            timezone: args.timezone.clone(),
            font_scale: args.font_scale.clone(),
            dark_mode: args.dark_mode,
            orientation: args.orientation.clone(),
            density: args.density,
        }
    }
}

pub fn default_owner() -> String {
    std::env::var("ANDROIDFORGE_OWNER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("USERNAME")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            std::env::var("USER")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "androidforge".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn retries_transient_read_failures_and_reports_attempts() {
        let attempts = AtomicU32::new(0);
        let (value, report) = retry_read_only(ReadOnlyOperation::UiObserve, 2, || {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt < 2 {
                    Err(ForgeError::new(
                        androidforge_core::error::ErrorCode::DeviceDisconnected,
                        "device disconnected while reading",
                    ))
                } else {
                    Ok("observed")
                }
            }
        })
        .await
        .expect("retry succeeds");
        assert_eq!(value, "observed");
        assert_eq!(report.attempts.len(), 3);
        assert_eq!(report.retries_used, 2);
        assert_eq!(report.final_result, "success");
        assert!(report.attempts[0].retryable);
        assert!(report.attempts[0].delay_before_next_ms > 0);
    }

    #[tokio::test]
    async fn non_retryable_errors_are_not_repeated() {
        let attempts = AtomicU32::new(0);
        let error = retry_read_only::<_, _, ()>(ReadOnlyOperation::UiObserve, 3, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async {
                Err(ForgeError::new(
                    androidforge_core::error::ErrorCode::SelectorNotFound,
                    "no element matched",
                ))
            }
        })
        .await
        .unwrap_err();
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(
            error.code(),
            androidforge_core::error::ErrorCode::SelectorNotFound
        );
        assert!(error.body().context.contains_key("retry"));
    }

    #[tokio::test]
    async fn excessive_retry_requests_are_rejected() {
        let error =
            retry_read_only::<_, _, ()>(ReadOnlyOperation::UiObserve, 99, || async { Ok(()) })
                .await
                .unwrap_err();
        assert_eq!(
            error.code(),
            androidforge_core::error::ErrorCode::InvalidArgument
        );
    }
}
