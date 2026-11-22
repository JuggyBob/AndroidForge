use androidforge_core::error::{ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::run::RunContext;
use androidforge_core::session::Session;
use androidforge_core::util::write_atomic;
use androidforge_diagnostics::{FrameResult, capture_frame};
use androidforge_ui::{Selector, UiHierarchy};
use std::time::{Duration, Instant};
use tokio::time::sleep;

use crate::cli::{
    PostconditionArgs, SelectorArgs, UiAssertNoTextArgs, UiCommand, UiDismissOverlaysArgs,
    UiDragArgs, UiDumpArgs, UiInspectArgs, UiKeyEventArgs, UiScrollArgs, UiScrollToArgs,
    UiSelectorArgs, UiSettleArgs, UiSwipeArgs, UiTextArgs, UiWaitArgs,
};
use crate::commands::common::{require_selector, success};
use crate::commands::observation;
use crate::commands::ocr;
use crate::commands::receipt;
use crate::context::CliContext;

pub async fn ui(ctx: &CliContext, command: &UiCommand) -> Result<CommandSuccess> {
    if matches!(command, UiCommand::Observe(_) | UiCommand::Diff(_))
        && ctx.load_session(false)?.is_none()
    {
        return Err(ForgeError::new(
            androidforge_core::error::ErrorCode::SessionNotFound,
            "ui observation requires a session",
        )
        .with_suggestion("Pass --session <ID> or run `androidforge session ensure ... --json`.")
        .with_suggestion("Run `androidforge context --json` to inspect available sessions."));
    }
    let session = ctx.load_session(true)?;
    let device = ctx.resolve_device(session.as_ref()).await?;
    let serial = device.serial.clone();
    match command {
        UiCommand::Dump(args) => {
            execute_in_run!(
                ctx,
                "ui.dump",
                Some("ui dump"),
                session.as_ref(),
                Some(&device),
                true,
                |run| dump(ctx, run, &serial, args),
            )
            .await
        }
        UiCommand::Inspect(args) => inspect(ctx, session.as_ref(), &device, args).await,
        UiCommand::Observe(args) => {
            let Some(session) = session.as_ref() else {
                return Err(ForgeError::new(
                    androidforge_core::error::ErrorCode::SessionNotFound,
                    "ui observe requires a session",
                )
                .with_suggestion("Pass --session <ID> or run `androidforge session ensure`."));
            };
            observation::observe_command(ctx, session, &device, args).await
        }
        UiCommand::Diff(args) => {
            let Some(session) = session.as_ref() else {
                return Err(ForgeError::new(
                    androidforge_core::error::ErrorCode::SessionNotFound,
                    "ui diff requires a session",
                )
                .with_suggestion("Pass --session <ID> or run `androidforge session ensure`."));
            };
            observation::diff_command(ctx, session, &device, args).await
        }
        UiCommand::Find(args) => {
            reject_ref_for("ui find", &args.selector)?;
            let selector = require_selector(&args.selector)?;
            execute_in_run!(
                ctx,
                "ui.find",
                Some("ui find"),
                session.as_ref(),
                Some(&device),
                true,
                |_run| async {
                    let ui = ctx.ui_manager()?;
                    let mut result = ui.find(&serial, &selector).await?;
                    if result.matches.is_empty() && args.selector.ocr_fallback {
                        let ocr = ocr::find(ctx, &serial, &selector).await?;
                        result = androidforge_ui::FindResult {
                            ambiguous: ocr.len() > 1,
                            ocr_used: true,
                            matches: ocr
                                .into_iter()
                                .enumerate()
                                .map(|(index, item)| {
                                    androidforge_ui::UiMatch::ocr(index, item.text, item.bounds)
                                })
                                .collect(),
                        };
                    }
                    let human = if result.matches.is_empty() {
                        "No matching UI elements.\n".to_string()
                    } else {
                        format!(
                            "{} matching element(s){}{}\n{}\n",
                            result.matches.len(),
                            if result.ambiguous { " (ambiguous)" } else { "" },
                            if result.ocr_used { " [OCR]" } else { "" },
                            result
                                .matches
                                .iter()
                                .map(|item| {
                                    let label = item
                                        .text
                                        .as_deref()
                                        .or(item.content_desc.as_deref())
                                        .or(item.resource_id.as_deref())
                                        .unwrap_or("(no label)");
                                    let bounds = format!(
                                        "[{},{}][{},{}]",
                                        item.bounds.left,
                                        item.bounds.top,
                                        item.bounds.right,
                                        item.bounds.bottom
                                    );
                                    format!(
                                        "  --index {} {label} {bounds} ({})",
                                        item.index, item.source
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        )
                    };
                    Ok(success("ui.find", serde_json::to_value(&result)?, human)
                        .with_optional_session(session_id(session.as_ref()))
                        .with_device(serial.clone()))
                },
            )
            .await
        }
        UiCommand::Tap(args) => {
            validate_reference_args(&args.selector)?;
            if args.selector.has_ref() {
                if args.x.is_some() || args.y.is_some() {
                    return Err(ForgeError::invalid_argument(
                        "--ref cannot be combined with --x/--y",
                    )
                    .with_suggestion("Use the referenced element's own bounds."));
                }
                let reference = args.selector.reference.clone().unwrap_or_default();
                let token = args.selector.expect_token.clone().unwrap_or_default();
                let receipt_requested = args.receipt || args.receipt_full;
                let full = args.receipt_full;
                return execute_in_run!(
                    ctx,
                    "ui.tap",
                    Some("ui tap referenced element"),
                    session.as_ref(),
                    Some(&device),
                    true,
                    |_run| async {
                        let Some(session) = session.as_ref() else {
                            return Err(ForgeError::new(
                                androidforge_core::error::ErrorCode::SessionNotFound,
                                "element references require a session",
                            )
                            .with_suggestion(
                                "Pass --session <ID> or run `androidforge session ensure`.",
                            ));
                        };
                        let resolved = observation::resolve_element_reference(
                            ctx,
                            session,
                            &device,
                            &reference,
                            &token,
                            observation::ReferenceUse::Tap,
                        )
                        .await?;
                        let node_value = serde_json::to_value(&resolved.node)?;
                        let human_label = resolved
                            .node
                            .text
                            .as_deref()
                            .or(resolved.node.content_desc.as_deref())
                            .or(resolved.node.resource_id.as_deref())
                            .unwrap_or("referenced element")
                            .to_string();
                        let ui = ctx.ui_manager()?;
                        if receipt_requested {
                            let mut builder = receipt::ReceiptBuilder::new(
                                "tap",
                                Some(serde_json::json!({
                                    "ref": reference,
                                    "expect_token": token,
                                })),
                                session,
                                &device,
                            );
                            builder.set_reference(receipt::ReceiptElementReference {
                                reference: reference.clone(),
                                expected_token: token.clone(),
                                resolved_node: Some(node_value.clone()),
                                validation: "validated".to_string(),
                            });
                            let before = match observation::collect_receipt_observation(ctx, session, &device, None)
                            .await
                            {
                                Ok(observation) => observation,
                                Err(error) => {
                                    let receipt = builder.finish(
                                        receipt::DispatchState::NotDispatched,
                                        true,
                                        Some(node_value.clone()),
                                        None,
                                        None,
                                        None,
                                        Some(error.message().to_string()),
                                        full,
                                    );
                                    return Err(receipt_error(
                                        error,
                                        &receipt,
                                        "before observation failed; the referenced action was not dispatched",
                                    ));
                                }
                            };
                            builder.set_before(before);
                            let before_frame =
                                capture_named(ctx, &serial, args.capture_before.as_ref()).await?;
                            let result = match ui.tap_expected(&serial, &resolved.expectation).await
                            {
                                Ok(result) => result,
                                Err(error) => {
                                    let receipt = dispatch_error_receipt(
                                        builder, &error, true, None, full,
                                    )?;
                                    return Err(receipt_error(
                                        error,
                                        &receipt,
                                        "referenced action validation or dispatch failed",
                                    ));
                                }
                            };
                            let mut action = serde_json::to_value(&result)?;
                            insert_named_capture(
                                &mut action,
                                "before_screenshot",
                                before_frame.as_ref(),
                            );
                            let (mut value, suffix) = complete_receipt(
                                ctx,
                                session,
                                &device,
                                builder,
                                true,
                                true,
                                Some(node_value),
                                action,
                                None,
                                &args.postcondition,
                                full,
                            )
                            .await?;
                            let after_frame =
                                capture_named(ctx, &serial, args.capture_after.as_ref()).await?;
                            insert_named_capture(
                                &mut value,
                                "after_screenshot",
                                after_frame.as_ref(),
                            );
                            return Ok(success(
                                "ui.tap",
                                value,
                                format!(
                                    "Tapped referenced element {human_label} at ({}, {}){suffix}\n",
                                    result.tap_x, result.tap_y
                                ),
                            )
                            .with_session(session.id.clone())
                            .with_device(serial.clone()));
                        }
                        let before_frame =
                            capture_named(ctx, &serial, args.capture_before.as_ref()).await?;
                        let result = ui.tap_expected(&serial, &resolved.expectation).await?;
                        let mut action = serde_json::to_value(&result)?;
                        insert_named_capture(
                            &mut action,
                            "before_screenshot",
                            before_frame.as_ref(),
                        );
                        if let Some(object) = action.as_object_mut() {
                            object.insert(
                                "element_ref".to_string(),
                                serde_json::Value::String(reference.clone()),
                            );
                            object.insert(
                                "observation_token".to_string(),
                                serde_json::Value::String(token.clone()),
                            );
                        }
                        let (mut value, suffix) = apply_postconditions(
                            ctx,
                            Some(session),
                            &device,
                            None,
                            &args.postcondition,
                            action,
                        )
                        .await?;
                        let after_frame =
                            capture_named(ctx, &serial, args.capture_after.as_ref()).await?;
                        insert_named_capture(
                            &mut value,
                            "after_screenshot",
                            after_frame.as_ref(),
                        );
                        Ok(success(
                            "ui.tap",
                            value,
                            format!(
                                "Tapped referenced element {human_label} at ({}, {}){suffix}\n",
                                result.tap_x, result.tap_y
                            ),
                        )
                        .with_session(session.id.clone())
                        .with_device(serial.clone()))
                    },
                )
                .await;
            }
            if args.x.is_some() || args.y.is_some() {
                let (x, y) = match (args.x, args.y) {
                    (Some(x), Some(y)) => (x, y),
                    _ => {
                        return Err(ForgeError::invalid_argument(
                            "coordinate tap needs both --x and --y",
                        )
                        .with_suggestion(
                            "Prefer a semantic selector such as --text or --resource-id.",
                        ));
                    }
                };
                if args.selector.has_selector() {
                    return Err(ForgeError::invalid_argument(
                        "choose either a semantic selector or --x/--y, not both",
                    ));
                }
                let receipt_requested = args.receipt || args.receipt_full;
                let full = args.receipt_full;
                execute_in_run!(
                    ctx,
                    "ui.tap",
                    Some("ui tap coordinates"),
                    session.as_ref(),
                    Some(&device),
                    true,
                    |_run| async {
                        let ui = ctx.ui_manager()?;
                        if receipt_requested {
                            let Some(session) = session.as_ref() else {
                                return Err(ForgeError::new(
                                    androidforge_core::error::ErrorCode::SessionNotFound,
                                    "action receipts require a session",
                                )
                                .with_suggestion(
                                    "Pass --session <ID> or run `androidforge session ensure`.",
                                ));
                            };
                            let selector_value = serde_json::json!({"x": x, "y": y});
                            let mut builder = receipt::ReceiptBuilder::new(
                                "tap",
                                Some(selector_value.clone()),
                                session,
                                &device,
                            );
                            let before = match observation::collect_receipt_observation(
                                ctx, session, &device, None,
                            )
                            .await
                            {
                                Ok(observation) => observation,
                                Err(error) => {
                                    let receipt = builder.finish(
                                        receipt::DispatchState::NotDispatched,
                                        false,
                                        None,
                                        None,
                                        None,
                                        None,
                                        Some(error.message().to_string()),
                                        full,
                                    );
                                    return Err(receipt_error(
                                        error,
                                        &receipt,
                                        "before observation failed; the action was not dispatched",
                                    ));
                                }
                            };
                            builder.set_before(before);
                            let before_frame =
                                capture_named(ctx, &serial, args.capture_before.as_ref()).await?;
                            let display = match ui.tap_xy(&serial, x, y).await {
                                Ok(display) => display,
                                Err(error) => {
                                    let receipt =
                                        dispatch_error_receipt(builder, &error, true, None, full)?;
                                    return Err(receipt_error(
                                        error,
                                        &receipt,
                                        "action dispatch failed",
                                    ));
                                }
                            };
                            let mut action =
                                serde_json::json!({"x": x, "y": y, "display_size": display});
                            insert_named_capture(
                                &mut action,
                                "before_screenshot",
                                before_frame.as_ref(),
                            );
                            let (mut value, suffix) = complete_receipt(
                                ctx,
                                session,
                                &device,
                                builder,
                                true,
                                true,
                                Some(selector_value),
                                action,
                                None,
                                &args.postcondition,
                                full,
                            )
                            .await?;
                            let after_frame =
                                capture_named(ctx, &serial, args.capture_after.as_ref()).await?;
                            insert_named_capture(
                                &mut value,
                                "after_screenshot",
                                after_frame.as_ref(),
                            );
                            return Ok(success(
                                "ui.tap",
                                value,
                                format!("Tapped ({x}, {y}) on {serial}{suffix}\n"),
                            )
                            .with_session(session.id.clone())
                            .with_device(serial.clone()));
                        }
                        let before_frame =
                            capture_named(ctx, &serial, args.capture_before.as_ref()).await?;
                        let display = ui.tap_xy(&serial, x, y).await?;
                        let mut action =
                            serde_json::json!({"x": x, "y": y, "display_size": display});
                        insert_named_capture(
                            &mut action,
                            "before_screenshot",
                            before_frame.as_ref(),
                        );
                        let (mut value, suffix) = apply_postconditions(
                            ctx,
                            session.as_ref(),
                            &device,
                            None,
                            &args.postcondition,
                            action,
                        )
                        .await?;
                        let after_frame =
                            capture_named(ctx, &serial, args.capture_after.as_ref()).await?;
                        insert_named_capture(&mut value, "after_screenshot", after_frame.as_ref());
                        Ok(success(
                            "ui.tap",
                            value,
                            format!("Tapped ({x}, {y}) on {serial}{suffix}\n"),
                        )
                        .with_optional_session(session_id(session.as_ref()))
                        .with_device(serial.clone()))
                    },
                )
                .await
            } else {
                let selector = require_selector(&args.selector)?;
                let receipt_requested = args.receipt || args.receipt_full;
                let full = args.receipt_full;
                execute_in_run!(
                    ctx,
                    "ui.tap",
                    Some("ui tap"),
                    session.as_ref(),
                    Some(&device),
                    true,
                    |_run| async {
                        let ui = ctx.ui_manager()?;
                        if receipt_requested {
                            let Some(session) = session.as_ref() else {
                                return Err(ForgeError::new(
                                    androidforge_core::error::ErrorCode::SessionNotFound,
                                    "action receipts require a session",
                                )
                                .with_suggestion(
                                    "Pass --session <ID> or run `androidforge session ensure`.",
                                ));
                            };
                            let selector_value = serde_json::to_value(&selector)?;
                            let mut builder = receipt::ReceiptBuilder::new(
                                "tap",
                                Some(selector_value.clone()),
                                session,
                                &device,
                            );
                            let before = match observation::collect_receipt_observation(
                                ctx, session, &device, None,
                            )
                            .await
                            {
                                Ok(observation) => observation,
                                Err(error) => {
                                    let receipt = builder.finish(
                                        receipt::DispatchState::NotDispatched,
                                        false,
                                        None,
                                        None,
                                        None,
                                        None,
                                        Some(error.message().to_string()),
                                        full,
                                    );
                                    return Err(receipt_error(
                                        error,
                                        &receipt,
                                        "before observation failed; the action was not dispatched",
                                    ));
                                }
                            };
                            builder.set_before(before);
                            let before_frame =
                                capture_named(ctx, &serial, args.capture_before.as_ref()).await?;
                            let result = match tap_or_ocr(
                                ctx,
                                &ui,
                                &serial,
                                &selector,
                                args.selector.ocr_fallback,
                            )
                            .await
                            {
                                Ok(result) => result,
                                Err(error) => {
                                    let resolved = !matches!(
                                        error.code(),
                                        androidforge_core::error::ErrorCode::SelectorNotFound
                                            | androidforge_core::error::ErrorCode::SelectorAmbiguous
                                    );
                                    let receipt = dispatch_error_receipt(
                                        builder, &error, resolved, None, full,
                                    )?;
                                    return Err(receipt_error(
                                        error,
                                        &receipt,
                                        "action dispatch failed",
                                    ));
                                }
                            };
                            let target = serde_json::to_value(&result.matched)?;
                            let mut action = serde_json::to_value(&result)?;
                            insert_named_capture(
                                &mut action,
                                "before_screenshot",
                                before_frame.as_ref(),
                            );
                            let (mut value, suffix) = complete_receipt(
                                ctx,
                                session,
                                &device,
                                builder,
                                true,
                                true,
                                Some(target),
                                action,
                                Some(&selector),
                                &args.postcondition,
                                full,
                            )
                            .await?;
                            let after_frame =
                                capture_named(ctx, &serial, args.capture_after.as_ref()).await?;
                            insert_named_capture(
                                &mut value,
                                "after_screenshot",
                                after_frame.as_ref(),
                            );
                            return Ok(success(
                                "ui.tap",
                                value,
                                format!(
                                    "Tapped {} at ({}, {}){}{}\n",
                                    result
                                        .matched
                                        .text
                                        .as_deref()
                                        .or(result.matched.content_desc.as_deref())
                                        .or(result.matched.resource_id.as_deref())
                                        .unwrap_or("matched element"),
                                    result.tap_x,
                                    result.tap_y,
                                    if result.used_clickable_ancestor {
                                        " via clickable ancestor"
                                    } else {
                                        ""
                                    },
                                    suffix
                                ),
                            )
                            .with_session(session.id.clone())
                            .with_device(serial.clone()));
                        }
                        let before_frame =
                            capture_named(ctx, &serial, args.capture_before.as_ref()).await?;
                        let result =
                            tap_or_ocr(ctx, &ui, &serial, &selector, args.selector.ocr_fallback)
                                .await?;
                        let mut action = serde_json::to_value(&result)?;
                        insert_named_capture(
                            &mut action,
                            "before_screenshot",
                            before_frame.as_ref(),
                        );
                        let (mut value, suffix) = apply_postconditions(
                            ctx,
                            session.as_ref(),
                            &device,
                            Some(&selector),
                            &args.postcondition,
                            action,
                        )
                        .await?;
                        let after_frame =
                            capture_named(ctx, &serial, args.capture_after.as_ref()).await?;
                        insert_named_capture(&mut value, "after_screenshot", after_frame.as_ref());
                        Ok(success(
                            "ui.tap",
                            value,
                            format!(
                                "Tapped {} at ({}, {}){}{}\n",
                                result
                                    .matched
                                    .text
                                    .as_deref()
                                    .or(result.matched.content_desc.as_deref())
                                    .or(result.matched.resource_id.as_deref())
                                    .unwrap_or("matched element"),
                                result.tap_x,
                                result.tap_y,
                                if result.used_clickable_ancestor {
                                    " via clickable ancestor"
                                } else {
                                    ""
                                },
                                suffix
                            ),
                        )
                        .with_optional_session(session_id(session.as_ref()))
                        .with_device(serial.clone()))
                    },
                )
                .await
            }
        }
        UiCommand::LongPress(args) => {
            validate_reference_args(&args.selector)?;
            if args.selector.has_ref() {
                if args.x.is_some() || args.y.is_some() {
                    return Err(ForgeError::invalid_argument(
                        "--ref cannot be combined with --x/--y",
                    ));
                }
                let reference = args.selector.reference.clone().unwrap_or_default();
                let token = args.selector.expect_token.clone().unwrap_or_default();
                let receipt_requested = args.receipt || args.receipt_full;
                let full = args.receipt_full;
                return execute_in_run!(
                    ctx,
                    "ui.long-press",
                    Some("ui long press referenced element"),
                    session.as_ref(),
                    Some(&device),
                    true,
                    |_run| async {
                        let Some(session) = session.as_ref() else {
                            return Err(ForgeError::new(
                                androidforge_core::error::ErrorCode::SessionNotFound,
                                "element references require a session",
                            )
                            .with_suggestion(
                                "Pass --session <ID> or run `androidforge session ensure`.",
                            ));
                        };
                        let resolved = observation::resolve_element_reference(
                            ctx,
                            session,
                            &device,
                            &reference,
                            &token,
                            observation::ReferenceUse::LongPress,
                        )
                        .await?;
                        let node_value = serde_json::to_value(&resolved.node)?;
                        if receipt_requested {
                            let mut builder = receipt::ReceiptBuilder::new(
                                "long_press",
                                Some(serde_json::json!({
                                    "ref": reference,
                                    "expect_token": token,
                                })),
                                session,
                                &device,
                            );
                            builder.set_reference(receipt::ReceiptElementReference {
                                reference: reference.clone(),
                                expected_token: token.clone(),
                                resolved_node: Some(node_value.clone()),
                                validation: "validated".to_string(),
                            });
                            let before = match observation::collect_receipt_observation(ctx, session, &device, None)
                            .await
                            {
                                Ok(observation) => observation,
                                Err(error) => {
                                    let receipt = builder.finish(
                                        receipt::DispatchState::NotDispatched,
                                        true,
                                        Some(node_value.clone()),
                                        None,
                                        None,
                                        None,
                                        Some(error.message().to_string()),
                                        full,
                                    );
                                    return Err(receipt_error(
                                        error,
                                        &receipt,
                                        "before observation failed; the referenced action was not dispatched",
                                    ));
                                }
                            };
                            builder.set_before(before);
                            let (result, mid_capture) = match long_press_action(
                                ctx,
                                &serial,
                                None,
                                Some(&resolved.expectation),
                                args.duration.unwrap_or(900),
                                args.capture_mid.as_ref(),
                            )
                            .await
                            {
                                Ok(action) => action,
                                Err(error) => {
                                    let receipt = dispatch_error_receipt(
                                        builder, &error, true, None, full,
                                    )?;
                                    return Err(receipt_error(
                                        error,
                                        &receipt,
                                        "referenced action validation or dispatch failed",
                                    ));
                                }
                            };
                            let mut action = serde_json::to_value(&result)?;
                            insert_mid_capture(&mut action, mid_capture.as_ref());
                            let (value, suffix) = complete_receipt(
                                ctx,
                                session,
                                &device,
                                builder,
                                true,
                                true,
                                Some(node_value),
                                action,
                                None,
                                &args.postcondition,
                                full,
                            )
                            .await?;
                            return Ok(success(
                                "ui.long-press",
                                value,
                                format!(
                                    "Long-pressed referenced element at ({}, {}).{suffix}\n",
                                    result.tap_x, result.tap_y
                                ),
                            )
                            .with_session(session.id.clone())
                            .with_device(serial.clone()));
                        }
                        let (result, mid_capture) = long_press_action(
                            ctx,
                            &serial,
                            None,
                            Some(&resolved.expectation),
                            args.duration.unwrap_or(900),
                            args.capture_mid.as_ref(),
                        )
                        .await?;
                        let mut action = serde_json::to_value(&result)?;
                        insert_mid_capture(&mut action, mid_capture.as_ref());
                        if let Some(object) = action.as_object_mut() {
                            object.insert(
                                "element_ref".to_string(),
                                serde_json::Value::String(reference.clone()),
                            );
                            object.insert(
                                "observation_token".to_string(),
                                serde_json::Value::String(token.clone()),
                            );
                        }
                        let (value, suffix) = apply_postconditions(
                            ctx,
                            Some(session),
                            &device,
                            None,
                            &args.postcondition,
                            action,
                        )
                        .await?;
                        Ok(success(
                            "ui.long-press",
                            value,
                            format!(
                                "Long-pressed referenced element at ({}, {}).{suffix}\n",
                                result.tap_x, result.tap_y
                            ),
                        )
                        .with_session(session.id.clone())
                        .with_device(serial.clone()))
                    },
                )
                .await;
            }
            let selector = require_selector(&args.selector)?;
            let receipt_requested = args.receipt || args.receipt_full;
            let full = args.receipt_full;
            execute_in_run!(
                ctx,
                "ui.long-press",
                Some("ui long press"),
                session.as_ref(),
                Some(&device),
                true,
                |_run| async {
                    if receipt_requested {
                        let Some(session) = session.as_ref() else {
                            return Err(ForgeError::new(
                                androidforge_core::error::ErrorCode::SessionNotFound,
                                "action receipts require a session",
                            )
                            .with_suggestion(
                                "Pass --session <ID> or run `androidforge session ensure`.",
                            ));
                        };
                        let selector_value = serde_json::to_value(&selector)?;
                        let mut builder = receipt::ReceiptBuilder::new(
                            "long_press",
                            Some(selector_value.clone()),
                            session,
                            &device,
                        );
                        let before = match observation::collect_receipt_observation(
                            ctx, session, &device, None,
                        )
                        .await
                        {
                            Ok(observation) => observation,
                            Err(error) => {
                                let receipt = builder.finish(
                                    receipt::DispatchState::NotDispatched,
                                    false,
                                    None,
                                    None,
                                    None,
                                    None,
                                    Some(error.message().to_string()),
                                    full,
                                );
                                return Err(receipt_error(
                                    error,
                                    &receipt,
                                    "before observation failed; the action was not dispatched",
                                ));
                            }
                        };
                        builder.set_before(before);
                        let (result, mid_capture) = match long_press_action(
                            ctx,
                            &serial,
                            Some(&selector),
                            None,
                            args.duration.unwrap_or(900),
                            args.capture_mid.as_ref(),
                        )
                        .await
                        {
                            Ok(action) => action,
                            Err(error) => {
                                let resolved = !matches!(
                                    error.code(),
                                    androidforge_core::error::ErrorCode::SelectorNotFound
                                        | androidforge_core::error::ErrorCode::SelectorAmbiguous
                                );
                                let receipt =
                                    dispatch_error_receipt(builder, &error, resolved, None, full)?;
                                return Err(receipt_error(
                                    error,
                                    &receipt,
                                    "action dispatch failed",
                                ));
                            }
                        };
                        let target = serde_json::to_value(&result.matched)?;
                        let mut action = serde_json::to_value(&result)?;
                        insert_mid_capture(&mut action, mid_capture.as_ref());
                        let (value, suffix) = complete_receipt(
                            ctx,
                            session,
                            &device,
                            builder,
                            true,
                            true,
                            Some(target),
                            action,
                            Some(&selector),
                            &args.postcondition,
                            full,
                        )
                        .await?;
                        return Ok(success(
                            "ui.long-press",
                            value,
                            format!(
                                "Long-pressed at ({}, {}).{suffix}\n",
                                result.tap_x, result.tap_y
                            ),
                        )
                        .with_session(session.id.clone())
                        .with_device(serial.clone()));
                    }
                    let (result, mid_capture) = long_press_action(
                        ctx,
                        &serial,
                        Some(&selector),
                        None,
                        args.duration.unwrap_or(900),
                        args.capture_mid.as_ref(),
                    )
                    .await?;
                    let mut action = serde_json::to_value(&result)?;
                    insert_mid_capture(&mut action, mid_capture.as_ref());
                    let (value, suffix) = apply_postconditions(
                        ctx,
                        session.as_ref(),
                        &device,
                        Some(&selector),
                        &args.postcondition,
                        action,
                    )
                    .await?;
                    Ok(success(
                        "ui.long-press",
                        value,
                        format!(
                            "Long-pressed at ({}, {}).{suffix}\n",
                            result.tap_x, result.tap_y
                        ),
                    )
                    .with_optional_session(session_id(session.as_ref()))
                    .with_device(serial.clone()))
                },
            )
            .await
        }
        UiCommand::Drag(args) => drag(ctx, session.as_ref(), &device, args).await,
        UiCommand::Text(args) => text(ctx, session.as_ref(), &device, args).await,
        UiCommand::Clear(args) => clear(ctx, session.as_ref(), &device, args).await,
        UiCommand::DismissOverlays(args) => {
            dismiss_overlays(ctx, session.as_ref(), &device, args).await
        }
        UiCommand::Wait(args) | UiCommand::Assert(args) => {
            wait_or_assert(
                ctx,
                session.as_ref(),
                &device,
                args,
                matches!(command, UiCommand::Assert(_)),
            )
            .await
        }
        UiCommand::AssertNoText(args) => assert_no_text(ctx, session.as_ref(), &device, args).await,
        UiCommand::Settle(args) => settle(ctx, session.as_ref(), &device, args).await,
        UiCommand::Scroll(args) => scroll(ctx, session.as_ref(), &device, args).await,
        UiCommand::ScrollTo(args) => scroll_to(ctx, session.as_ref(), &device, args).await,
        UiCommand::Swipe(args) => swipe(ctx, session.as_ref(), &device, args).await,
        UiCommand::Back(args) => {
            execute_in_run!(
                ctx,
                "ui.back",
                Some("ui back"),
                session.as_ref(),
                Some(&device),
                true,
                |_run| async {
                    let ui = ctx.ui_manager()?;
                    let result = ui.back(&serial, args.hide_keyboard).await?;
                    Ok(success(
                        "ui.back",
                        serde_json::to_value(&result)?,
                        format!(
                            "Back key sent{}.\n",
                            if result.keyboard_hidden {
                                " after hiding keyboard"
                            } else {
                                ""
                            }
                        ),
                    )
                    .with_optional_session(session_id(session.as_ref()))
                    .with_device(serial.clone()))
                },
            )
            .await
        }
        UiCommand::Home => {
            execute_in_run!(
                ctx,
                "ui.home",
                Some("ui home"),
                session.as_ref(),
                Some(&device),
                true,
                |_run| async {
                    let ui = ctx.ui_manager()?;
                    let result = ui.home(&serial).await?;
                    Ok(success(
                        "ui.home",
                        serde_json::to_value(&result)?,
                        "Home key sent.\n".to_string(),
                    )
                    .with_optional_session(session_id(session.as_ref()))
                    .with_device(serial.clone()))
                },
            )
            .await
        }
        UiCommand::Keyevent(args) => keyevent(ctx, session.as_ref(), &device, args).await,
        UiCommand::HideKeyboard => {
            execute_in_run!(
                ctx,
                "ui.hide-keyboard",
                Some("ui hide keyboard"),
                session.as_ref(),
                Some(&device),
                true,
                |_run| async {
                    let ui = ctx.ui_manager()?;
                    let was_visible = ui.hide_keyboard(&serial).await?;
                    Ok(success(
                        "ui.hide-keyboard",
                        serde_json::json!({"was_visible": was_visible}),
                        format!(
                            "{}\n",
                            if was_visible {
                                "Hid the soft keyboard."
                            } else {
                                "The soft keyboard was not visible."
                            }
                        ),
                    )
                    .with_optional_session(session_id(session.as_ref()))
                    .with_device(serial.clone()))
                },
            )
            .await
        }
        UiCommand::Focused => {
            execute_in_run!(
                ctx,
                "ui.focused",
                Some("ui focused"),
                session.as_ref(),
                Some(&device),
                true,
                |_run| async {
                    let ui = ctx.ui_manager()?;
                    let focused = ui.focused(&serial).await?;
                    let human = if focused.is_empty() {
                        "No focused nodes.\n".to_string()
                    } else {
                        format!(
                            "{}\n",
                            focused
                                .iter()
                                .map(|node| node.short_label())
                                .collect::<Vec<_>>()
                                .join("\n")
                        )
                    };
                    Ok(
                        success("ui.focused", serde_json::json!({"focused": focused}), human)
                            .with_optional_session(session_id(session.as_ref()))
                            .with_device(serial.clone()),
                    )
                },
            )
            .await
        }
    }
}

const SYSTEM_OVERLAY_LABELS: &[&str] = &["got it", "ok", "dismiss", "close"];

fn system_overlay_label(node: &androidforge_ui::UiNode) -> Option<String> {
    let package = node.package.as_deref().unwrap_or("");
    if package != "com.android.systemui" || !node.visible() || !node.clickable {
        return None;
    }
    let label = node
        .text
        .as_deref()
        .or(node.content_desc.as_deref())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    SYSTEM_OVERLAY_LABELS
        .contains(&label.as_str())
        .then_some(label)
}

async fn dismiss_overlays(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiDismissOverlaysArgs,
) -> Result<CommandSuccess> {
    execute_in_run!(
        ctx,
        "ui.dismiss-overlays",
        Some("ui dismiss-overlays"),
        session,
        Some(device),
        true,
        |_run| async {
            let ui = ctx.ui_manager()?;
            let hierarchy = ui.dump(&device.serial).await?;
            let mut candidates = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            for node in &hierarchy.nodes {
                let Some(label) = system_overlay_label(node) else {
                    continue;
                };
                let key = node
                    .resource_id
                    .clone()
                    .unwrap_or_else(|| format!("{}:{label}", node.path));
                if !seen.insert(key.clone()) {
                    continue;
                }
                candidates.push(serde_json::json!({
                    "label": label,
                    "resource_id": node.resource_id,
                    "package": node.package,
                    "path": node.path,
                    "clickable": node.clickable,
                    "key": key,
                }));
            }
            let limit = args.max.max(1);
            let selected = candidates
                .iter()
                .take(limit)
                .cloned()
                .collect::<Vec<_>>();
            if !ctx.dry_run {
                for candidate in &selected {
                    let selector = if let Some(resource_id) =
                        candidate.get("resource_id").and_then(|value| value.as_str())
                    {
                        Selector {
                            resource_id: Some(resource_id.to_string()),
                            clickable: Some(true),
                            visible: Some(true),
                            ..Selector::default()
                        }
                    } else {
                        Selector {
                            text: candidate
                                .get("label")
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            clickable: Some(true),
                            visible: Some(true),
                            ..Selector::default()
                        }
                    };
                    ui.tap(&device.serial, &selector).await?;
                    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                }
            }
            let remaining = if ctx.dry_run {
                candidates.len().saturating_sub(selected.len())
            } else {
                let after = ui.dump(&device.serial).await?;
                after
                    .nodes
                    .iter()
                    .filter(|node| system_overlay_label(node).is_some())
                    .count()
            };
            let result = serde_json::json!({
                "dry_run": ctx.dry_run,
                "serial": device.serial,
                "candidates": candidates,
                "dismissed": if ctx.dry_run { Vec::<serde_json::Value>::new() } else { selected.clone() },
                "selected": selected.len(),
                "remaining_overlays": remaining,
            });
            let human = if ctx.dry_run {
                format!(
                    "Would dismiss {} known SystemUI overlay(s); {} candidate(s) remain.\n",
                    selected.len(),
                    remaining
                )
            } else {
                format!(
                    "Dismissed {} known SystemUI overlay(s); {} visible overlay(s) remain.\n",
                    selected.len(),
                    remaining
                )
            };
            Ok(success(
                "ui.dismiss-overlays",
                result,
                human,
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(device.serial.clone()))
        },
    )
    .await
}

async fn tap_or_ocr(
    ctx: &CliContext,
    ui: &androidforge_ui::UiManager,
    serial: &str,
    selector: &Selector,
    ocr_fallback: bool,
) -> Result<androidforge_ui::TapResult> {
    match ui.tap(serial, selector).await {
        Ok(result) => Ok(result),
        Err(error)
            if ocr_fallback
                && error.code() == androidforge_core::error::ErrorCode::SelectorNotFound =>
        {
            let matches = ocr::find(ctx, serial, selector).await?;
            let Some(item) = matches.first() else {
                return Err(error);
            };
            let (x, y) = item.bounds.center();
            ui.tap_xy(serial, x, y).await?;
            Ok(androidforge_ui::TapResult {
                matched: androidforge_ui::UiMatch::ocr(0, item.text.clone(), item.bounds),
                tap_x: x,
                tap_y: y,
                used_clickable_ancestor: false,
                ancestor: None,
                display_size: None,
                rotation: None,
            })
        }
        Err(error) => Err(error),
    }
}

async fn inspect(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiInspectArgs,
) -> Result<CommandSuccess> {
    let ui = ctx.ui_manager()?;
    let hierarchy = ui.dump(&device.serial).await?;
    let mut output_path = None;
    if let Some(output) = &args.output {
        let path = if output.is_absolute() {
            output.clone()
        } else {
            ctx.paths.cwd.join(output)
        };
        write_atomic(&path, hierarchy.xml.as_bytes())?;
        output_path = Some(path.to_string_lossy().into_owned());
    }
    let human = format!(
        "Inspected {} node(s) from {}{}\n",
        hierarchy.nodes.len(),
        hierarchy
            .package_name
            .as_deref()
            .unwrap_or(device.serial.as_str()),
        output_path
            .as_ref()
            .map(|path| format!("; XML at {path}"))
            .unwrap_or_default()
    );
    Ok(success(
        "ui.inspect",
        serde_json::json!({
            "hierarchy": hierarchy,
            "output_path": output_path,
        }),
        human,
    )
    .with_optional_session(session.map(|session| session.id.clone()))
    .with_device(device.serial.clone()))
}

async fn dump(
    ctx: &CliContext,
    run: &mut RunContext,
    serial: &str,
    args: &UiDumpArgs,
) -> Result<CommandSuccess> {
    let ui = ctx.ui_manager()?;
    let hierarchy = ui.dump(serial).await?;
    let relative = "ui/hierarchy.xml";
    let artifact = run.write_text_artifact(
        relative,
        "ui_hierarchy",
        Some("UIAutomator hierarchy"),
        &hierarchy.xml,
    )?;
    let mut output_path = None;
    if let Some(output) = &args.output {
        let path = if output.is_absolute() {
            output.clone()
        } else {
            ctx.paths.cwd.join(output)
        };
        write_atomic(&path, hierarchy.xml.as_bytes())?;
        output_path = Some(path.to_string_lossy().into_owned());
    }
    let focused = hierarchy.focused();
    let result = serde_json::json!({
        "path": artifact.path,
        "output_path": output_path,
        "package": hierarchy.package_name,
        "rotation": hierarchy.rotation,
        "nodes": hierarchy.nodes.len(),
        "focused_nodes": focused.len(),
    });
    let human = format!(
        "Dumped {} node(s) from {}{}{}\n",
        hierarchy.nodes.len(),
        hierarchy
            .package_name
            .as_deref()
            .unwrap_or("unknown package"),
        args.output
            .as_ref()
            .map(|path| format!(" to {}", path.display()))
            .unwrap_or_default(),
        if focused.is_empty() {
            String::new()
        } else {
            format!("; {} focused node(s)", focused.len())
        }
    );
    Ok(success("ui.dump", result, human).with_artifacts(vec![artifact]))
}

async fn clear(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiSelectorArgs,
) -> Result<CommandSuccess> {
    validate_reference_args(&args.selector)?;
    let serial = device.serial.clone();
    if args.selector.has_ref() {
        let Some(session) = session else {
            return Err(ForgeError::new(
                androidforge_core::error::ErrorCode::SessionNotFound,
                "element references require a session",
            )
            .with_suggestion("Pass --session <ID> or run `androidforge session ensure`."));
        };
        let reference = args.selector.reference.clone().unwrap_or_default();
        let token = args.selector.expect_token.clone().unwrap_or_default();
        let resolved = observation::resolve_element_reference(
            ctx,
            session,
            device,
            &reference,
            &token,
            observation::ReferenceUse::Clear,
        )
        .await?;
        return execute_in_run!(
            ctx,
            "ui.clear",
            Some("ui clear referenced element"),
            Some(session),
            Some(device),
            true,
            |_run| async {
                let ui = ctx.ui_manager()?;
                let result = ui.clear_expected(&serial, &resolved.expectation).await?;
                let mut action = serde_json::to_value(&result)?;
                add_reference_action_context(
                    &mut action,
                    &args.selector,
                    serde_json::to_value(&resolved.node)?,
                );
                Ok(success(
                    "ui.clear",
                    action,
                    "Cleared the referenced text field.\n".to_string(),
                )
                .with_session(session.id.clone())
                .with_device(serial.clone()))
            },
        )
        .await;
    }
    let selector = if args.selector.has_selector() {
        Some(require_selector(&args.selector)?)
    } else {
        None
    };
    execute_in_run!(
        ctx,
        "ui.clear",
        Some("ui clear"),
        session,
        Some(device),
        true,
        |_run| async {
            let ui = ctx.ui_manager()?;
            let result = ui.clear(&serial, selector.as_ref()).await?;
            Ok(success(
                "ui.clear",
                serde_json::to_value(&result)?,
                "Cleared the focused text field.\n".to_string(),
            )
            .with_optional_session(session_id(session))
            .with_device(serial.clone()))
        },
    )
    .await
}
async fn text(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiTextArgs,
) -> Result<CommandSuccess> {
    validate_reference_args(&args.selector)?;
    let reference = args.selector.reference.clone();
    let token = args.selector.expect_token.clone();
    let resolved = if let (Some(reference), Some(token)) = (reference.as_deref(), token.as_deref())
    {
        let Some(session) = session else {
            return Err(ForgeError::new(
                androidforge_core::error::ErrorCode::SessionNotFound,
                "element references require a session",
            )
            .with_suggestion("Pass --session <ID> or run `androidforge session ensure`."));
        };
        Some(
            observation::resolve_element_reference(
                ctx,
                session,
                device,
                reference,
                token,
                observation::ReferenceUse::Text,
            )
            .await?,
        )
    } else {
        None
    };
    let selector = if resolved.is_some() {
        None
    } else if args.selector.has_selector() {
        Some(require_selector(&args.selector)?)
    } else {
        None
    };
    let serial = device.serial.clone();
    let receipt_requested = args.receipt || args.receipt_full;
    let full = args.receipt_full;
    execute_in_run!(
        ctx,
        "ui.text",
        Some("ui text"),
        session,
        Some(device),
        true,
        |_run| async {
            let mut ui = ctx.ui_manager()?;
            ui.text_verify = !args.no_verify;
            if receipt_requested {
                let Some(session) = session else {
                    return Err(ForgeError::new(
                        androidforge_core::error::ErrorCode::SessionNotFound,
                        "action receipts require a session",
                    )
                    .with_suggestion("Pass --session <ID> or run `androidforge session ensure`."));
                };
                let selector_value = reference_selector_value(&args.selector, selector.as_ref())?;
                let mut builder =
                    receipt::ReceiptBuilder::new("text", selector_value, session, device);
                if let Some(resolved) = &resolved {
                    builder.set_reference(receipt::ReceiptElementReference {
                        reference: args.selector.reference.clone().unwrap_or_default(),
                        expected_token: args.selector.expect_token.clone().unwrap_or_default(),
                        resolved_node: Some(serde_json::to_value(&resolved.node)?),
                        validation: "validated".to_string(),
                    });
                }
                let before = match observation::collect_receipt_observation(
                    ctx, session, device, None,
                )
                .await
                {
                    Ok(observation) => observation,
                    Err(error) => {
                        let receipt = builder.finish(
                            receipt::DispatchState::NotDispatched,
                            false,
                            None,
                            None,
                            None,
                            None,
                            Some(error.message().to_string()),
                            full,
                        );
                        return Err(receipt_error(
                            error,
                            &receipt,
                            "before observation failed; the action was not dispatched",
                        ));
                    }
                };
                builder.set_before(before);
                let result = match &resolved {
                    Some(resolved) => {
                        ui.text_expected(
                            &serial,
                            &resolved.expectation,
                            &args.value,
                            args.clear,
                            args.hide_keyboard,
                        )
                        .await
                    }
                    None => {
                        ui.text(
                            &serial,
                            &args.value,
                            selector.as_ref(),
                            args.clear,
                            args.hide_keyboard,
                        )
                        .await
                    }
                };
                let result = match result {
                    Ok(result) => result,
                    Err(error) => {
                        let resolved = !matches!(
                            error.code(),
                            androidforge_core::error::ErrorCode::SelectorNotFound
                                | androidforge_core::error::ErrorCode::SelectorAmbiguous
                        );
                        let receipt =
                            dispatch_error_receipt(builder, &error, resolved, None, full)?;
                        return Err(receipt_error(error, &receipt, "action dispatch failed"));
                    }
                };
                let action = serde_json::to_value(&result)?;
                let (value, suffix) = complete_receipt(
                    ctx,
                    session,
                    device,
                    builder,
                    true,
                    true,
                    resolved
                        .as_ref()
                        .map(|resolved| serde_json::to_value(&resolved.node))
                        .transpose()?,
                    action,
                    selector.as_ref(),
                    &args.postcondition,
                    full,
                )
                .await?;
                let human = format!(
                    "Text entry {} for {} character(s){}.{}\n",
                    if result.verified {
                        "verified"
                    } else {
                        "completed without verification"
                    },
                    args.value.chars().count(),
                    if result.cleared {
                        " after clearing the field"
                    } else {
                        ""
                    },
                    suffix
                );
                return Ok(success("ui.text", value, human)
                    .with_session(session.id.clone())
                    .with_device(serial.clone()));
            }
            let result = match &resolved {
                Some(resolved) => {
                    ui.text_expected(
                        &serial,
                        &resolved.expectation,
                        &args.value,
                        args.clear,
                        args.hide_keyboard,
                    )
                    .await?
                }
                None => {
                    ui.text(
                        &serial,
                        &args.value,
                        selector.as_ref(),
                        args.clear,
                        args.hide_keyboard,
                    )
                    .await?
                }
            };
            let mut action = serde_json::to_value(&result)?;
            if let Some(resolved) = &resolved {
                add_reference_action_context(
                    &mut action,
                    &args.selector,
                    serde_json::to_value(&resolved.node)?,
                );
            }
            let (value, suffix) = apply_postconditions(
                ctx,
                session,
                device,
                selector.as_ref(),
                &args.postcondition,
                action,
            )
            .await?;
            let human = format!(
                "Text entry {} for {} character(s){}.{}\n",
                if result.verified {
                    "verified"
                } else {
                    "completed without verification"
                },
                args.value.chars().count(),
                if result.cleared {
                    " after clearing the field"
                } else {
                    ""
                },
                suffix
            );
            Ok(success("ui.text", value, human)
                .with_optional_session(session_id(session))
                .with_device(serial.clone()))
        },
    )
    .await
}

async fn drag(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiDragArgs,
) -> Result<CommandSuccess> {
    let serial = device.serial.clone();
    execute_in_run!(
        ctx,
        "ui.drag",
        Some("ui drag"),
        session,
        Some(device),
        true,
        |_run| async {
            let ui = ctx.ui_manager()?;
            let from = drag_point(
                &ui,
                &serial,
                args.from_x,
                args.from_y,
                &args.from_text,
                &args.from_id,
                &args.from,
                "from",
            )
            .await?;
            let to = drag_point(
                &ui,
                &serial,
                args.to_x,
                args.to_y,
                &args.to_text,
                &args.to_id,
                &args.to,
                "to",
            )
            .await?;
            let steps = 12u64;
            let step_delay = Duration::from_millis((args.duration_ms / steps).max(1));
            ui.input_motion(&serial, "DOWN", from.0, from.1).await?;
            if args.hold_before_ms > 0 {
                sleep(Duration::from_millis(args.hold_before_ms)).await;
            }
            for step in 1..=steps {
                let x = from.0 as i64 + (to.0 as i64 - from.0 as i64) * step as i64 / steps as i64;
                let y = from.1 as i64 + (to.1 as i64 - from.1 as i64) * step as i64 / steps as i64;
                ui.input_motion(&serial, "MOVE", x as i32, y as i32).await?;
                sleep(step_delay).await;
            }
            let mut frame = None;
            if let Some(path) = &args.capture_mid {
                let output = output_path(ctx, path);
                match capture_frame(&ctx.adb()?, &serial, &output, Duration::from_secs(15)).await {
                    Ok(captured) => frame = Some(captured),
                    Err(error) => {
                        let _ = ui.input_motion(&serial, "UP", to.0, to.1).await;
                        return Err(error);
                    }
                }
            }
            if args.hold_after_ms > 0 {
                sleep(Duration::from_millis(args.hold_after_ms)).await;
            }
            ui.input_motion(&serial, "UP", to.0, to.1).await?;
            let mut value = serde_json::json!({
                "from_x": from.0,
                "from_y": from.1,
                "to_x": to.0,
                "to_y": to.1,
                "duration_ms": args.duration_ms,
                "hold_before_ms": args.hold_before_ms,
                "hold_after_ms": args.hold_after_ms,
                "steps": steps,
            });
            if let Some(frame) = frame {
                value["mid_capture"] = serde_json::to_value(frame)?;
            }
            let (value, suffix) =
                apply_postconditions(ctx, session, device, None, &args.postcondition, value)
                    .await?;
            Ok(success(
                "ui.drag",
                value,
                format!(
                    "Dragged ({}, {}) -> ({}, {}) over {}ms.{suffix}\n",
                    from.0, from.1, to.0, to.1, args.duration_ms
                ),
            )
            .with_optional_session(session_id(session))
            .with_device(serial.clone()))
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn drag_point(
    ui: &androidforge_ui::UiManager,
    serial: &str,
    x: Option<i32>,
    y: Option<i32>,
    text: &Option<String>,
    id: &Option<String>,
    expression: &Option<String>,
    role: &str,
) -> Result<(i32, i32)> {
    if let (Some(x), Some(y)) = (x, y) {
        return Ok((x, y));
    }
    if x.is_some() || y.is_some() {
        return Err(ForgeError::invalid_argument(format!(
            "ui drag requires both --{role}-x and --{role}-y"
        )));
    }
    let mut selector = if let Some(expression) = expression {
        parse_drag_selector(expression)?
    } else {
        Selector::default()
    };
    if selector.text.is_none() {
        selector.text = text.clone();
    }
    if selector.resource_id.is_none() {
        selector.resource_id = id.clone();
    }
    if selector.text.is_none() && selector.resource_id.is_none() {
        return Err(ForgeError::invalid_argument(format!(
            "ui drag requires --{role}-x/--{role}-y, --{role} <selector>, or --{role}-text/--{role}-id"
        )));
    }
    let selection = ui.select(serial, &selector).await?;
    let bounds = selection.selected_node().bounds;
    Ok(bounds.center())
}

fn parse_drag_selector(value: &str) -> Result<Selector> {
    let mut selector = Selector::default();
    let (key, value) = value.split_once('=').unwrap_or(("text", value));
    let value = value.trim();
    if value.is_empty() {
        return Err(ForgeError::invalid_argument(
            "ui drag selector value cannot be empty",
        ));
    }
    match key.trim().to_ascii_lowercase().as_str() {
        "text" => selector.text = Some(value.to_string()),
        "contains" => selector.text_contains = Some(value.to_string()),
        "desc" | "content-desc" => selector.content_desc = Some(value.to_string()),
        "id" | "resource-id" => selector.resource_id = Some(value.to_string()),
        "tag" | "test-tag" => selector.resource_id = Some(value.to_string()),
        "class" => selector.class = Some(value.to_string()),
        other => {
            return Err(ForgeError::invalid_argument(format!(
                "unsupported drag selector key '{other}'"
            ))
            .with_suggestion("Use text=, contains=, desc=, id=, tag= or class=."));
        }
    }
    Ok(selector)
}

async fn long_press_action(
    ctx: &CliContext,
    serial: &str,
    selector: Option<&Selector>,
    expected: Option<&androidforge_ui::NodeExpectation>,
    duration_ms: u64,
    capture_mid: Option<&std::path::PathBuf>,
) -> Result<(androidforge_ui::TapResult, Option<FrameResult>)> {
    let ui = ctx.ui_manager()?;
    let capture = if let Some(path) = capture_mid {
        let output = output_path(ctx, path);
        if let Some(parent) = output.parent() {
            androidforge_core::util::ensure_dir(parent)?;
        }
        let adb = ctx.adb()?;
        let serial = serial.to_string();
        let delay = Duration::from_millis((duration_ms / 2).max(1));
        Some(tokio::spawn(async move {
            sleep(delay).await;
            capture_frame(&adb, &serial, &output, Duration::from_secs(15)).await
        }))
    } else {
        None
    };
    let gesture = if let Some(expected) = expected {
        ui.long_press_expected(serial, expected, duration_ms).await
    } else {
        let selector = selector.ok_or_else(|| {
            ForgeError::invalid_argument("ui long-press requires a selector or element reference")
        })?;
        ui.long_press_with_duration(serial, selector, duration_ms)
            .await
    };
    let frame = match capture {
        Some(task) => match task.await {
            Ok(result) => Some(result?),
            Err(error) => {
                return Err(ForgeError::new(
                    androidforge_core::error::ErrorCode::CommandFailed,
                    format!("mid-gesture capture task failed: {error}"),
                )
                .device(serial.to_string()));
            }
        },
        None => None,
    };
    Ok((gesture?, frame))
}

fn insert_mid_capture(action: &mut serde_json::Value, frame: Option<&FrameResult>) {
    if let Some(frame) = frame
        && let Some(object) = action.as_object_mut()
    {
        object.insert(
            "mid_capture".to_string(),
            serde_json::to_value(frame).unwrap_or(serde_json::Value::Null),
        );
    }
}

async fn capture_named(
    ctx: &CliContext,
    serial: &str,
    path: Option<&std::path::PathBuf>,
) -> Result<Option<FrameResult>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let output = output_path(ctx, path);
    if let Some(parent) = output.parent() {
        androidforge_core::util::ensure_dir(parent)?;
    }
    let frame = capture_frame(&ctx.adb()?, serial, &output, Duration::from_secs(15)).await?;
    Ok(Some(frame))
}

fn insert_named_capture(action: &mut serde_json::Value, key: &str, frame: Option<&FrameResult>) {
    if let Some(frame) = frame
        && let Some(object) = action.as_object_mut()
    {
        object.insert(
            key.to_string(),
            serde_json::to_value(frame).unwrap_or(serde_json::Value::Null),
        );
    }
}

fn text_absence_payload(
    result: &androidforge_ui::TextAbsenceResult,
    texts: &[String],
) -> serde_json::Value {
    let mut value = serde_json::json!({
        "absent": result.absent,
        "attempts": result.attempts,
        "duration_ms": result.duration_ms,
        "texts": texts,
    });
    if let Some(package) = &result.hierarchy_package {
        value["hierarchy_package"] = serde_json::Value::String(package.clone());
    }
    value
}

fn output_path(ctx: &CliContext, path: &std::path::Path) -> std::path::PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        ctx.paths.cwd.join(path)
    }
}

async fn assert_no_text(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiAssertNoTextArgs,
) -> Result<CommandSuccess> {
    let mut texts = Vec::new();
    if let Some(text) = &args.text {
        texts.push(text.clone());
    }
    if let Some(contains) = &args.contains {
        texts.push(contains.clone());
    }
    if texts.is_empty() {
        return Err(ForgeError::invalid_argument(
            "ui assert-no-text requires --text or --contains",
        ));
    }
    let serial = device.serial.clone();
    let timeout = ctx.parse_timeout(args.timeout.as_deref(), 1)?;
    execute_in_run!(
        ctx,
        "ui.assert-no-text",
        Some("ui assert no text"),
        session,
        Some(device),
        true,
        |_run| async {
            let ui = ctx.ui_manager()?;
            let result = ui.wait_texts_absent(&serial, &texts, timeout).await?;
            if !result.absent {
                return Err(ForgeError::new(
                    androidforge_core::error::ErrorCode::UiAssertionFailed,
                    format!(
                        "text still present after {} attempt(s): {}",
                        result.attempts,
                        texts.join(", ")
                    ),
                )
                .with_context(
                    "lingering",
                    serde_json::to_value(&result.lingering).unwrap_or(serde_json::Value::Null),
                )
                .with_suggestion("Inspect the current screen or wait for the state to change.")
                .device(serial.to_string()));
            }
            Ok(success(
                "ui.assert-no-text",
                text_absence_payload(&result, &texts),
                format!(
                    "Confirmed absent after {} attempt(s) in {}ms: {}\n",
                    result.attempts,
                    result.duration_ms,
                    texts.join(", ")
                ),
            )
            .with_optional_session(session_id(session))
            .with_device(serial.clone()))
        },
    )
    .await
}

async fn wait_or_assert(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiWaitArgs,
    assertion: bool,
) -> Result<CommandSuccess> {
    let special = !args.for_text_gone.is_empty() || args.for_idle || args.for_stable;
    if special {
        if args.selector.has_selector() || args.selector.has_ref() || args.not_exists {
            return Err(ForgeError::invalid_argument(
                "--for-text-gone/--for-idle/--for-stable cannot be combined with a selector or --not-exists",
            ));
        }
        if args.for_idle && args.for_stable {
            return Err(ForgeError::invalid_argument(
                "--for-idle and --for-stable are mutually exclusive",
            ));
        }
        let serial = device.serial.clone();
        let timeout = ctx.parse_timeout(
            args.timeout.as_deref(),
            if assertion {
                1
            } else {
                ctx.config.file.ui.default_timeout_secs
            },
        )?;
        let stable_for = ctx.parse_timeout(args.stable_for.as_deref(), 2)?;
        let texts = args.for_text_gone.clone();
        let command = if assertion { "ui.assert" } else { "ui.wait" };
        return execute_in_run!(
            ctx,
            command,
            Some("ui wait state"),
            session,
            Some(device),
            true,
            |_run| async {
                let ui = ctx.ui_manager()?;
                if args.for_idle {
                    let result = ui.settle(&serial, timeout).await?;
                    return Ok(success(
                        command,
                        serde_json::to_value(&result)?,
                        format!(
                            "UI idle after {} dump(s) in {}ms.\n",
                            result.attempts, result.duration_ms
                        ),
                    )
                    .with_optional_session(session_id(session))
                    .with_device(serial.clone()));
                }
                if args.for_stable {
                    let result = ui.settle_for(&serial, stable_for, timeout).await?;
                    return Ok(success(
                        command,
                        serde_json::to_value(&result)?,
                        format!(
                            "UI stable for {}ms after {} dump(s).\n",
                            stable_for.as_millis(),
                            result.attempts
                        ),
                    )
                    .with_optional_session(session_id(session))
                    .with_device(serial.clone()));
                }
                let result = ui.wait_texts_absent(&serial, &texts, timeout).await?;
                if !result.absent {
                    return Err(ForgeError::new(
                        androidforge_core::error::ErrorCode::UiAssertionFailed,
                        format!(
                            "text still present after {} attempt(s): {}",
                            result.attempts,
                            texts.join(", ")
                        ),
                    )
                    .with_context(
                        "lingering",
                        serde_json::to_value(&result.lingering).unwrap_or(serde_json::Value::Null),
                    )
                    .with_suggestion("Inspect the current screen or increase --timeout.")
                    .device(serial.to_string()));
                }
                Ok(success(
                    command,
                    text_absence_payload(&result, &texts),
                    format!(
                        "Confirmed absent after {} attempt(s) in {}ms: {}\n",
                        result.attempts,
                        result.duration_ms,
                        texts.join(", ")
                    ),
                )
                .with_optional_session(session_id(session))
                .with_device(serial.clone()))
            },
        )
        .await;
    }
    validate_reference_args(&args.selector)?;
    let reference = args.selector.reference.clone();
    let token = args.selector.expect_token.clone();
    let resolved = if let (Some(reference), Some(token)) = (reference.as_deref(), token.as_deref())
    {
        let Some(session) = session else {
            return Err(ForgeError::new(
                androidforge_core::error::ErrorCode::SessionNotFound,
                "element references require a session",
            )
            .with_suggestion("Pass --session <ID> or run `androidforge session ensure`."));
        };
        Some(
            observation::resolve_element_reference(
                ctx,
                session,
                device,
                reference,
                token,
                if assertion {
                    observation::ReferenceUse::Assert
                } else {
                    observation::ReferenceUse::Wait
                },
            )
            .await?,
        )
    } else {
        None
    };
    let selector = if resolved.is_none() {
        Some(require_selector(&args.selector)?)
    } else {
        None
    };
    let serial = device.serial.clone();
    let timeout = ctx.parse_timeout(
        args.timeout.as_deref(),
        if assertion {
            1
        } else {
            ctx.config.file.ui.default_timeout_secs
        },
    )?;
    execute_in_run!(
        ctx,
        if assertion { "ui.assert" } else { "ui.wait" },
        Some(if assertion { "ui assert" } else { "ui wait" }),
        session,
        Some(device),
        true,
        |_run| async {
            let ui = ctx.ui_manager()?;
            let result = match &resolved {
                Some(resolved) => {
                    ui.wait_expected(&serial, &resolved.expectation, timeout, args.not_exists)
                        .await?
                }
                None => {
                    let selector = selector.as_ref().ok_or_else(|| {
                        ForgeError::invalid_argument(
                            "ui wait requires a selector or a stable element reference",
                        )
                    })?;
                    ui.wait(&serial, selector, timeout, args.not_exists).await?
                }
            };
            let mut value = serde_json::to_value(&result)?;
            if let Some(resolved) = &resolved {
                add_reference_action_context(
                    &mut value,
                    &args.selector,
                    serde_json::to_value(&resolved.node)?,
                );
            }
            let human = format!(
                "{}{} after {} attempt(s) in {}ms.\n",
                if args.not_exists {
                    "Confirmed absence"
                } else {
                    "Matched element"
                },
                result
                    .hierarchy_package
                    .as_ref()
                    .map(|package| format!(" in {package}"))
                    .unwrap_or_default(),
                result.attempts,
                result.duration_ms
            );
            Ok(success(
                if assertion { "ui.assert" } else { "ui.wait" },
                value,
                human,
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(serial.clone()))
        },
    )
    .await
}

async fn settle(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiSettleArgs,
) -> Result<CommandSuccess> {
    let serial = device.serial.clone();
    let timeout = ctx.parse_timeout(
        args.timeout.as_deref(),
        ctx.config.file.ui.default_timeout_secs,
    )?;
    execute_in_run!(
        ctx,
        "ui.settle",
        Some("ui settle"),
        session,
        Some(device),
        true,
        |_run| async {
            let ui = ctx.ui_manager()?;
            let result = ui.settle(&serial, timeout).await?;
            Ok(success(
                "ui.settle",
                serde_json::to_value(&result)?,
                format!("UI settled after {} dump(s).\n", result.attempts),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(serial.clone()))
        },
    )
    .await
}

async fn scroll(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiScrollArgs,
) -> Result<CommandSuccess> {
    validate_reference_args(&args.selector)?;
    let reference = args.selector.reference.clone();
    let token = args.selector.expect_token.clone();
    let resolved = if let (Some(reference), Some(token)) = (reference.as_deref(), token.as_deref())
    {
        let Some(session) = session else {
            return Err(ForgeError::new(
                androidforge_core::error::ErrorCode::SessionNotFound,
                "element references require a session",
            )
            .with_suggestion("Pass --session <ID> or run `androidforge session ensure`."));
        };
        Some(
            observation::resolve_element_reference(
                ctx,
                session,
                device,
                reference,
                token,
                observation::ReferenceUse::Scroll,
            )
            .await?,
        )
    } else {
        None
    };
    let selector = if resolved.is_some() {
        None
    } else if args.selector.has_selector() {
        Some(require_selector(&args.selector)?)
    } else {
        None
    };
    let serial = device.serial.clone();
    let receipt_requested = args.receipt || args.receipt_full;
    let full = args.receipt_full;
    execute_in_run!(
        ctx,
        "ui.scroll",
        Some("ui scroll"),
        session,
        Some(device),
        true,
        |_run| async {
            let ui = ctx.ui_manager()?;
            if receipt_requested {
                let Some(session) = session else {
                    return Err(ForgeError::new(
                        androidforge_core::error::ErrorCode::SessionNotFound,
                        "action receipts require a session",
                    )
                    .with_suggestion("Pass --session <ID> or run `androidforge session ensure`."));
                };
                let selector_value = reference_selector_value(&args.selector, selector.as_ref())?;
                let mut builder =
                    receipt::ReceiptBuilder::new("scroll", selector_value, session, device);
                if let Some(resolved) = &resolved {
                    builder.set_reference(receipt::ReceiptElementReference {
                        reference: args.selector.reference.clone().unwrap_or_default(),
                        expected_token: args.selector.expect_token.clone().unwrap_or_default(),
                        resolved_node: Some(serde_json::to_value(&resolved.node)?),
                        validation: "validated".to_string(),
                    });
                }
                let before = match observation::collect_receipt_observation(
                    ctx, session, device, None,
                )
                .await
                {
                    Ok(observation) => observation,
                    Err(error) => {
                        let receipt = builder.finish(
                            receipt::DispatchState::NotDispatched,
                            false,
                            None,
                            None,
                            None,
                            None,
                            Some(error.message().to_string()),
                            full,
                        );
                        return Err(receipt_error(
                            error,
                            &receipt,
                            "before observation failed; the action was not dispatched",
                        ));
                    }
                };
                builder.set_before(before);
                let result = match &resolved {
                    Some(resolved) => {
                        ui.scroll_expected(&serial, &args.direction, &resolved.expectation)
                            .await
                    }
                    None => ui.scroll(&serial, &args.direction, selector.as_ref()).await,
                };
                let result = match result {
                    Ok(result) => result,
                    Err(error) => {
                        let resolved = !matches!(
                            error.code(),
                            androidforge_core::error::ErrorCode::SelectorNotFound
                                | androidforge_core::error::ErrorCode::SelectorAmbiguous
                        );
                        let receipt =
                            dispatch_error_receipt(builder, &error, resolved, None, full)?;
                        return Err(receipt_error(error, &receipt, "action dispatch failed"));
                    }
                };
                let action = serde_json::to_value(&result)?;
                let (value, suffix) = complete_receipt(
                    ctx,
                    session,
                    device,
                    builder,
                    true,
                    true,
                    resolved
                        .as_ref()
                        .map(|resolved| serde_json::to_value(&resolved.node))
                        .transpose()?,
                    action,
                    selector.as_ref(),
                    &args.postcondition,
                    full,
                )
                .await?;
                return Ok(success(
                    "ui.scroll",
                    value,
                    format!(
                        "Scrolled {} within [{},{}][{},{}].{}\n",
                        args.direction,
                        result.target_bounds.left,
                        result.target_bounds.top,
                        result.target_bounds.right,
                        result.target_bounds.bottom,
                        suffix
                    ),
                )
                .with_session(session.id.clone())
                .with_device(serial.clone()));
            }
            let result = match &resolved {
                Some(resolved) => {
                    ui.scroll_expected(&serial, &args.direction, &resolved.expectation)
                        .await?
                }
                None => {
                    ui.scroll(&serial, &args.direction, selector.as_ref())
                        .await?
                }
            };
            let mut action = serde_json::to_value(&result)?;
            if let Some(resolved) = &resolved {
                add_reference_action_context(
                    &mut action,
                    &args.selector,
                    serde_json::to_value(&resolved.node)?,
                );
            }
            let (value, suffix) = apply_postconditions(
                ctx,
                session,
                device,
                selector.as_ref(),
                &args.postcondition,
                action,
            )
            .await?;
            Ok(success(
                "ui.scroll",
                value,
                format!(
                    "Scrolled {} within [{},{}][{},{}].{}\n",
                    args.direction,
                    result.target_bounds.left,
                    result.target_bounds.top,
                    result.target_bounds.right,
                    result.target_bounds.bottom,
                    suffix
                ),
            )
            .with_optional_session(session_id(session))
            .with_device(serial.clone()))
        },
    )
    .await
}

async fn scroll_to(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiScrollToArgs,
) -> Result<CommandSuccess> {
    reject_ref_for("ui scroll-to", &args.selector)?;
    let selector = require_selector(&args.selector)?;
    let serial = device.serial.clone();
    let timeout = ctx.parse_timeout(
        args.timeout.as_deref(),
        ctx.config.file.ui.default_timeout_secs,
    )?;
    execute_in_run!(
        ctx,
        "ui.scroll-to",
        Some("ui scroll-to"),
        session,
        Some(device),
        true,
        |_run| async {
            let ui = ctx.ui_manager()?;
            let result = ui
                .scroll_to(
                    &serial,
                    &selector,
                    &args.direction,
                    args.max_swipes,
                    timeout,
                )
                .await?;
            Ok(success(
                "ui.scroll-to",
                serde_json::to_value(&result)?,
                format!("Found element after {} swipe(s).\n", result.swipes),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(serial.clone()))
        },
    )
    .await
}

async fn swipe(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiSwipeArgs,
) -> Result<CommandSuccess> {
    let serial = device.serial.clone();
    execute_in_run!(
        ctx,
        "ui.swipe",
        Some("ui swipe"),
        session,
        Some(device),
        true,
        |_run| async {
            let ui = ctx.ui_manager()?;
            let display = ui
                .swipe(&serial, args.x1, args.y1, args.x2, args.y2, args.duration_ms)
                .await?;
            Ok(success(
                "ui.swipe",
                serde_json::json!({"from": [args.x1, args.y1], "to": [args.x2, args.y2], "duration_ms": args.duration_ms, "display_size": display}),
                format!("Swiped ({}, {}) -> ({}, {}).\n", args.x1, args.y1, args.x2, args.y2),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(serial.clone()))
        },
    )
    .await
}

async fn keyevent(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiKeyEventArgs,
) -> Result<CommandSuccess> {
    if args.keycodes.is_empty() {
        return Err(ForgeError::invalid_argument(
            "at least one keycode is required",
        ));
    }
    let serial = device.serial.clone();
    execute_in_run!(
        ctx,
        "ui.keyevent",
        Some("ui keyevent"),
        session,
        Some(device),
        true,
        |_run| async {
            let ui = ctx.ui_manager()?;
            let mut results = Vec::new();
            for keycode in &args.keycodes {
                results.push(ui.keyevent(&serial, keycode).await?);
            }
            Ok(success(
                "ui.keyevent",
                serde_json::json!({"keycodes": args.keycodes, "results": results}),
                format!("Sent keyevent(s): {}\n", args.keycodes.join(", ")),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
            .with_device(serial.clone()))
        },
    )
    .await
}

fn validate_reference_args(selector: &SelectorArgs) -> Result<()> {
    if selector.reference.is_some() {
        if selector.expect_token.is_none() {
            return Err(ForgeError::invalid_argument(
                "--ref requires --expect-token so the reference cannot be reused outside its observation",
            )
            .with_suggestion(
                "Pass the observation token returned by `ui observe`: --ref el-... --expect-token obs-...",
            ));
        }
        if selector.has_non_ref_selector() {
            return Err(ForgeError::invalid_argument(
                "--ref cannot be combined with text, resource-id, class, index or other selector filters",
            )
            .with_suggestion(
                "Use either a semantic selector or a stable ref; stale refs never fall back to text.",
            ));
        }
    } else if selector.expect_token.is_some() {
        return Err(ForgeError::invalid_argument(
            "--expect-token is only valid together with --ref",
        ));
    }
    Ok(())
}

fn reject_ref_for(command: &str, selector: &SelectorArgs) -> Result<()> {
    validate_reference_args(selector)?;
    if selector.has_ref() {
        return Err(ForgeError::invalid_argument(format!(
            "--ref is not supported for {command}"
        ))
        .with_suggestion(
            "Use `ui tap`, `ui long-press`, `ui text`, `ui clear`, `ui scroll`, `ui wait` or `ui assert` with --ref.",
        )
        .with_suggestion(
            "For other actions, observe again and use a semantic selector.",
        ));
    }
    Ok(())
}

fn reference_selector_value(
    selector: &SelectorArgs,
    compiled: Option<&Selector>,
) -> Result<Option<serde_json::Value>> {
    if selector.has_ref() {
        return Ok(Some(serde_json::json!({
            "ref": selector.reference,
            "expect_token": selector.expect_token,
        })));
    }
    Ok(compiled.map(serde_json::to_value).transpose()?)
}

fn add_reference_action_context(
    action: &mut serde_json::Value,
    selector: &SelectorArgs,
    resolved_node: serde_json::Value,
) {
    if let Some(object) = action.as_object_mut() {
        if let Some(reference) = &selector.reference {
            object.insert(
                "element_ref".to_string(),
                serde_json::Value::String(reference.clone()),
            );
        }
        if let Some(token) = &selector.expect_token {
            object.insert(
                "observation_token".to_string(),
                serde_json::Value::String(token.clone()),
            );
        }
        object.insert("resolved_node".to_string(), resolved_node);
    }
}

fn error_was_not_dispatched(error: &ForgeError) -> bool {
    matches!(
        error.code(),
        androidforge_core::error::ErrorCode::SelectorNotFound
            | androidforge_core::error::ErrorCode::SelectorAmbiguous
            | androidforge_core::error::ErrorCode::ElementRefUnknown
            | androidforge_core::error::ErrorCode::ElementRefStale
            | androidforge_core::error::ErrorCode::ElementIdentityChanged
            | androidforge_core::error::ErrorCode::ElementNotActionable
            | androidforge_core::error::ErrorCode::ElementDeviceMismatch
    )
}

fn session_id(session: Option<&Session>) -> Option<String> {
    session.map(|session| session.id.clone())
}

fn postconditions_requested(args: &PostconditionArgs) -> bool {
    !args.wait_for_text.is_empty()
        || !args.wait_for_value.is_empty()
        || !args.wait_for_package.is_empty()
        || !args.wait_for_activity.is_empty()
        || !args.wait_for_absent_text.is_empty()
}

fn contains_text(hierarchy: &UiHierarchy, expected: &str) -> bool {
    hierarchy.nodes.iter().any(|node| {
        node.visible()
            && !node.password
            && (node
                .text
                .as_deref()
                .is_some_and(|text| text.contains(expected))
                || node
                    .content_desc
                    .as_deref()
                    .is_some_and(|text| text.contains(expected)))
    })
}

fn value_matches(hierarchy: &UiHierarchy, selector: Option<&Selector>, expected: &str) -> bool {
    if let Some(selector) = selector {
        if let Ok(selection) = androidforge_ui::select(hierarchy.clone(), selector) {
            let node = selection.selected_node();
            return node
                .text
                .as_deref()
                .is_some_and(|text| text.contains(expected))
                || node
                    .hint
                    .as_deref()
                    .is_some_and(|text| text.contains(expected));
        }
        return false;
    }
    hierarchy.nodes.iter().any(|node| {
        node.focused
            && node.editable_like()
            && (node
                .text
                .as_deref()
                .is_some_and(|text| text.contains(expected))
                || node
                    .hint
                    .as_deref()
                    .is_some_and(|text| text.contains(expected)))
    })
}

struct PostconditionOutcome {
    requested: bool,
    result: receipt::PostconditionResult,
    duration_ms: u64,
    conditions: Vec<receipt::PostconditionCondition>,
    foreground_package: Option<String>,
    foreground_activity: Option<String>,
    observation: Option<observation::Observation>,
    evaluation_error: Option<String>,
}

impl PostconditionOutcome {
    fn not_requested() -> Self {
        Self {
            requested: false,
            result: receipt::PostconditionResult::NotRequested,
            duration_ms: 0,
            conditions: Vec::new(),
            foreground_package: None,
            foreground_activity: None,
            observation: None,
            evaluation_error: None,
        }
    }

    fn to_receipt(&self) -> receipt::PostconditionReceipt {
        receipt::PostconditionReceipt {
            result: self.result,
            duration_ms: self.duration_ms,
            conditions: self.conditions.clone(),
            foreground_package: self.foreground_package.clone(),
            foreground_activity: self.foreground_activity.clone(),
        }
    }
}

/// Poll the requested postconditions after an action. The action itself is
/// never repeated: only observations are refreshed until the deadline.
async fn evaluate_postconditions(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    selector: Option<&Selector>,
    post: &PostconditionArgs,
) -> PostconditionOutcome {
    if !postconditions_requested(post) {
        return PostconditionOutcome::not_requested();
    }
    let started = Instant::now();
    let timeout = match ctx.parse_timeout(post.timeout.as_deref(), 10) {
        Ok(timeout) => timeout,
        Err(error) => {
            return PostconditionOutcome {
                requested: true,
                result: receipt::PostconditionResult::NotEvaluated,
                duration_ms: 0,
                conditions: Vec::new(),
                foreground_package: None,
                foreground_activity: None,
                observation: None,
                evaluation_error: Some(error.message().to_string()),
            };
        }
    };
    let ui = match ctx.ui_manager() {
        Ok(ui) => ui,
        Err(error) => {
            return PostconditionOutcome {
                requested: true,
                result: receipt::PostconditionResult::NotEvaluated,
                duration_ms: 0,
                conditions: Vec::new(),
                foreground_package: None,
                foreground_activity: None,
                observation: None,
                evaluation_error: Some(error.message().to_string()),
            };
        }
    };
    let adb = ctx.adb().ok();
    let serial = device.serial.clone();
    let needs_hierarchy = !post.wait_for_text.is_empty()
        || !post.wait_for_absent_text.is_empty()
        || !post.wait_for_value.is_empty();
    loop {
        let hierarchy = if needs_hierarchy {
            match ui.dump(&serial).await {
                Ok(hierarchy) => Some(hierarchy),
                Err(error) => {
                    return PostconditionOutcome {
                        requested: true,
                        result: receipt::PostconditionResult::NotEvaluated,
                        duration_ms: started.elapsed().as_millis() as u64,
                        conditions: Vec::new(),
                        foreground_package: None,
                        foreground_activity: None,
                        observation: None,
                        evaluation_error: Some(error.message().to_string()),
                    };
                }
            }
        } else {
            None
        };
        let focus = match &adb {
            Some(adb) => adb.current_focus(&serial).await,
            None => None,
        };
        let foreground_package = focus
            .as_ref()
            .map(|focus| focus.package.clone())
            .or_else(|| {
                hierarchy
                    .as_ref()
                    .and_then(|hierarchy| hierarchy.package_name.clone())
            });
        let foreground_activity = focus.as_ref().and_then(|focus| focus.activity.clone());
        let mut conditions = Vec::new();
        for expected in &post.wait_for_text {
            conditions.push(receipt::PostconditionCondition {
                kind: "text_visible".to_string(),
                expected: expected.clone(),
                satisfied: hierarchy
                    .as_ref()
                    .is_some_and(|hierarchy| contains_text(hierarchy, expected)),
            });
        }
        for expected in &post.wait_for_absent_text {
            conditions.push(receipt::PostconditionCondition {
                kind: "text_absent".to_string(),
                expected: expected.clone(),
                satisfied: hierarchy
                    .as_ref()
                    .is_some_and(|hierarchy| !contains_text(hierarchy, expected)),
            });
        }
        for expected in &post.wait_for_value {
            conditions.push(receipt::PostconditionCondition {
                kind: "value_contains".to_string(),
                expected: expected.clone(),
                satisfied: hierarchy
                    .as_ref()
                    .is_some_and(|hierarchy| value_matches(hierarchy, selector, expected)),
            });
        }
        for expected in &post.wait_for_package {
            conditions.push(receipt::PostconditionCondition {
                kind: "package_foreground".to_string(),
                expected: expected.clone(),
                satisfied: foreground_package.as_deref() == Some(expected.as_str()),
            });
        }
        for expected in &post.wait_for_activity {
            conditions.push(receipt::PostconditionCondition {
                kind: "activity_foreground".to_string(),
                expected: expected.clone(),
                satisfied: foreground_activity
                    .as_deref()
                    .is_some_and(|activity| activity.contains(expected.as_str())),
            });
        }
        let satisfied = conditions.iter().all(|condition| condition.satisfied);
        if satisfied {
            let observation = match session {
                Some(session) => {
                    observation::collect_receipt_observation(ctx, session, device, None)
                        .await
                        .ok()
                }
                None => None,
            };
            return PostconditionOutcome {
                requested: true,
                result: receipt::PostconditionResult::Satisfied,
                duration_ms: started.elapsed().as_millis() as u64,
                conditions,
                foreground_package,
                foreground_activity,
                observation,
                evaluation_error: None,
            };
        }
        if started.elapsed() >= timeout {
            let observation = match session {
                Some(session) => {
                    observation::collect_receipt_observation(ctx, session, device, None)
                        .await
                        .ok()
                }
                None => None,
            };
            return PostconditionOutcome {
                requested: true,
                result: receipt::PostconditionResult::TimedOut,
                duration_ms: started.elapsed().as_millis() as u64,
                conditions,
                foreground_package,
                foreground_activity,
                observation,
                evaluation_error: None,
            };
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

fn postcondition_value(outcome: &PostconditionOutcome) -> serde_json::Value {
    serde_json::json!({
        "requested": outcome.requested,
        "result": match outcome.result {
            receipt::PostconditionResult::NotRequested => "not_requested",
            receipt::PostconditionResult::Satisfied => "satisfied",
            receipt::PostconditionResult::TimedOut => "timed_out",
            receipt::PostconditionResult::NotEvaluated => "not_evaluated",
        },
        "satisfied": outcome.result == receipt::PostconditionResult::Satisfied,
        "duration_ms": outcome.duration_ms,
        "conditions": outcome.conditions,
        "foreground_package": outcome.foreground_package,
        "foreground_activity": outcome.foreground_activity,
        "observation_id": outcome.observation.as_ref().map(|observation| observation.id.clone()),
        "evaluation_error": outcome.evaluation_error,
    })
}

/// Run postconditions and return the legacy wrapper used by actions without
/// `--receipt`. Action dispatch has already happened and is never repeated.
async fn apply_postconditions(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    selector: Option<&Selector>,
    post: &PostconditionArgs,
    action: serde_json::Value,
) -> Result<(serde_json::Value, String)> {
    let outcome = evaluate_postconditions(ctx, session, device, selector, post).await;
    if outcome.result == receipt::PostconditionResult::TimedOut {
        let timeout_ms = outcome.duration_ms;
        let mut error = ForgeError::new(
            androidforge_core::error::ErrorCode::PostconditionTimeout,
            format!(
                "the UI action completed but its postcondition was not satisfied within {}ms",
                timeout_ms
            ),
        )
        .with_context("action_executed", serde_json::json!(true))
        .with_context("postconditions", serde_json::json!(outcome.conditions))
        .with_context(
            "foreground_package",
            serde_json::json!(outcome.foreground_package),
        )
        .with_context(
            "foreground_activity",
            serde_json::json!(outcome.foreground_activity),
        )
        .with_context("timeout_ms", serde_json::json!(timeout_ms))
        .with_suggestion(
            "Run `androidforge ui observe --session <ID> --json` to inspect the actual screen.",
        )
        .with_suggestion(
            "Do not retry a destructive action blindly; verify whether it already changed state.",
        )
        .device(device.serial.clone());
        if let Some(session) = session {
            error = error.session(session.id.clone());
        }
        if let Some(observation) = &outcome.observation {
            error = error.with_context("observation_id", serde_json::json!(observation.id));
        }
        return Err(error);
    }
    if outcome.result == receipt::PostconditionResult::NotEvaluated {
        let mut error = ForgeError::new(
            androidforge_core::error::ErrorCode::CommandFailed,
            format!(
                "the UI action completed but its postcondition could not be evaluated: {}",
                outcome
                    .evaluation_error
                    .as_deref()
                    .unwrap_or("observation failed")
            ),
        )
        .with_context("action_executed", serde_json::json!(true))
        .with_context("postcondition_result", serde_json::json!("not_evaluated"))
        .with_suggestion(
            "Run `androidforge ui observe --session <ID> --json` to inspect the device state.",
        )
        .device(device.serial.clone());
        if let Some(session) = session {
            error = error.session(session.id.clone());
        }
        return Err(error);
    }
    if outcome.requested {
        let result = serde_json::json!({
            "action_executed": true,
            "action": action,
            "postcondition": postcondition_value(&outcome),
        });
        return Ok((
            result,
            format!(" Postcondition satisfied in {}ms.", outcome.duration_ms),
        ));
    }
    Ok((action, String::new()))
}

fn receipt_error(
    mut error: ForgeError,
    receipt: &receipt::ActionReceipt,
    message: &str,
) -> ForgeError {
    error = error.with_context(
        "receipt",
        serde_json::to_value(receipt).unwrap_or(serde_json::Value::Null),
    );
    error = error.with_context("action_dispatched", serde_json::json!(receipt.dispatched));
    error.with_context_string("receipt_message", message.to_string())
}

/// Complete a receipt-enabled action. The action has already been dispatched
/// exactly once. A postcondition timeout or evaluation failure is returned as
/// an error that still carries the full partial receipt.
#[allow(clippy::too_many_arguments)]
async fn complete_receipt(
    ctx: &CliContext,
    session: &Session,
    device: &androidforge_core::model::Device,
    builder: receipt::ReceiptBuilder,
    dispatched: bool,
    target_resolved: bool,
    target: Option<serde_json::Value>,
    action: serde_json::Value,
    selector: Option<&Selector>,
    post: &PostconditionArgs,
    full: bool,
) -> Result<(serde_json::Value, String)> {
    let outcome = evaluate_postconditions(ctx, Some(session), device, selector, post).await;
    let after = match outcome.observation.clone() {
        Some(observation) => Some(observation),
        None => observation::collect_receipt_observation(ctx, session, device, None)
            .await
            .ok(),
    };
    let receipt = builder.finish(
        if dispatched {
            receipt::DispatchState::Dispatched
        } else {
            receipt::DispatchState::NotDispatched
        },
        target_resolved,
        target,
        after,
        Some(action.clone()),
        outcome.requested.then(|| outcome.to_receipt()),
        None,
        full,
    );
    if outcome.result == receipt::PostconditionResult::TimedOut {
        let error = ForgeError::new(
            androidforge_core::error::ErrorCode::PostconditionTimeout,
            format!(
                "the UI action completed but its postcondition was not satisfied within {}ms",
                outcome.duration_ms
            ),
        )
        .with_suggestion(
            "Run `androidforge ui observe --session <ID> --json` to inspect the actual screen.",
        )
        .with_suggestion(
            "Do not retry a destructive action blindly; verify whether it already changed state.",
        )
        .session(session.id.clone())
        .device(device.serial.clone());
        return Err(receipt_error(
            error,
            &receipt,
            "action executed; postcondition timed out",
        ));
    }
    if outcome.result == receipt::PostconditionResult::NotEvaluated {
        let error = ForgeError::new(
            androidforge_core::error::ErrorCode::CommandFailed,
            format!(
                "the UI action completed but its postcondition could not be evaluated: {}",
                outcome
                    .evaluation_error
                    .as_deref()
                    .unwrap_or("observation failed")
            ),
        )
        .with_suggestion(
            "Run `androidforge ui observe --session <ID> --json` to inspect the device state.",
        )
        .session(session.id.clone())
        .device(device.serial.clone());
        return Err(receipt_error(
            error,
            &receipt,
            "action executed; postcondition could not be evaluated",
        ));
    }
    let mut value = match action {
        serde_json::Value::Object(mut object) => {
            object.insert("receipt".to_string(), serde_json::to_value(&receipt)?);
            if outcome.requested {
                object.insert("postcondition".to_string(), postcondition_value(&outcome));
            }
            serde_json::Value::Object(object)
        }
        other => serde_json::json!({"action": other, "receipt": receipt}),
    };
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "action_executed".to_string(),
            serde_json::Value::Bool(dispatched),
        );
    }
    let suffix = match receipt.effect {
        receipt::Effect::NoVisibleChange => " No visible semantic change was observed.".to_string(),
        effect => format!(" Effect: {}.", effect.as_str()),
    };
    Ok((value, suffix))
}

fn dispatch_error_receipt(
    builder: receipt::ReceiptBuilder,
    error: &ForgeError,
    target_resolved: bool,
    after: Option<observation::Observation>,
    full: bool,
) -> Result<receipt::ActionReceipt> {
    let dispatch_state = if error_was_not_dispatched(error) {
        receipt::DispatchState::NotDispatched
    } else {
        receipt::DispatchState::Unknown
    };
    Ok(builder.finish(
        dispatch_state,
        target_resolved,
        None,
        after,
        None,
        None,
        Some(error.message().to_string()),
        full,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selector(reference: Option<&str>, token: Option<&str>, text: Option<&str>) -> SelectorArgs {
        SelectorArgs {
            reference: reference.map(str::to_string),
            expect_token: token.map(str::to_string),
            text: text.map(str::to_string),
            ..SelectorArgs::default()
        }
    }

    #[test]
    fn ref_requires_an_observation_token() {
        let selector = selector(Some("el-abcdef"), None, None);
        let error = validate_reference_args(&selector).unwrap_err();
        assert!(error.message().contains("--expect-token"));
    }

    #[test]
    fn ref_rejects_selector_filters() {
        let selector = selector(Some("el-abcdef"), Some("obs-1"), Some("Save"));
        let error = validate_reference_args(&selector).unwrap_err();
        assert!(error.message().contains("cannot be combined"));
    }

    #[test]
    fn expect_token_requires_a_ref() {
        let selector = selector(None, Some("obs-1"), None);
        let error = validate_reference_args(&selector).unwrap_err();
        assert!(error.message().contains("only valid together"));
    }

    #[test]
    fn reference_selector_value_is_explicit() {
        let selector = selector(Some("el-abcdef"), Some("obs-1"), None);
        let value = reference_selector_value(&selector, None)
            .unwrap()
            .expect("reference payload");
        assert_eq!(value["ref"], "el-abcdef");
        assert_eq!(value["expect_token"], "obs-1");
    }

    #[test]
    fn reference_action_context_names_the_resolved_node() {
        let selector = selector(Some("el-abcdef"), Some("obs-1"), None);
        let mut action = serde_json::json!({"ok": true});
        add_reference_action_context(&mut action, &selector, serde_json::json!({"path": "0/1"}));
        assert_eq!(action["element_ref"], "el-abcdef");
        assert_eq!(action["observation_token"], "obs-1");
        assert_eq!(action["resolved_node"]["path"], "0/1");
    }

    #[test]
    fn drag_selector_expressions_are_explicit() {
        let text = parse_drag_selector("Save").unwrap();
        assert_eq!(text.text.as_deref(), Some("Save"));

        let tagged = parse_drag_selector("tag=trash").unwrap();
        assert_eq!(tagged.resource_id.as_deref(), Some("trash"));

        let desc = parse_drag_selector("desc=Drag handle").unwrap();
        assert_eq!(desc.content_desc.as_deref(), Some("Drag handle"));

        assert!(parse_drag_selector("bogus=value").is_err());
        assert!(parse_drag_selector("text=").is_err());
    }
}
