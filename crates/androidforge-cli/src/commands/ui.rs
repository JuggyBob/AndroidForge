use androidforge_core::error::{ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::run::RunContext;
use androidforge_core::session::Session;
use androidforge_core::util::write_atomic;
use androidforge_ui::{Selector, UiHierarchy};
use std::time::Instant;

use crate::cli::{
    PostconditionArgs, SelectorArgs, UiCommand, UiDumpArgs, UiInspectArgs, UiKeyEventArgs,
    UiScrollArgs, UiScrollToArgs, UiSettleArgs, UiSwipeArgs, UiTextArgs, UiWaitArgs,
};
use crate::commands::common::{require_selector, success};
use crate::commands::observation;
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
                    let result = ui.find(&serial, &selector).await?;
                    let human = if result.matches.is_empty() {
                        "No matching UI elements.\n".to_string()
                    } else {
                        format!(
                            "{} matching element(s){}\n{}\n",
                            result.matches.len(),
                            if result.ambiguous { " (ambiguous)" } else { "" },
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
                                    format!("  --index {} {label} {bounds}", item.index)
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
                            ctx, session, &device, &reference, &token,
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
                            let before = match observation::collect_observation(
                                ctx, session, &device, None, false,
                            )
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
                            let action = serde_json::to_value(&result)?;
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
                        let result = ui.tap_expected(&serial, &resolved.expectation).await?;
                        let mut action = serde_json::to_value(&result)?;
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
                            let before = match observation::collect_observation(
                                ctx, session, &device, None, false,
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
                            let action =
                                serde_json::json!({"x": x, "y": y, "display_size": display});
                            let (value, suffix) = complete_receipt(
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
                            return Ok(success(
                                "ui.tap",
                                value,
                                format!("Tapped ({x}, {y}) on {serial}{suffix}\n"),
                            )
                            .with_session(session.id.clone())
                            .with_device(serial.clone()));
                        }
                        let display = ui.tap_xy(&serial, x, y).await?;
                        let action = serde_json::json!({"x": x, "y": y, "display_size": display});
                        let (value, suffix) = apply_postconditions(
                            ctx,
                            session.as_ref(),
                            &device,
                            None,
                            &args.postcondition,
                            action,
                        )
                        .await?;
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
                            let before = match observation::collect_observation(
                                ctx, session, &device, None, false,
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
                            let result = match ui.tap(&serial, &selector).await {
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
                            let action = serde_json::to_value(&result)?;
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
                        let result = ui.tap(&serial, &selector).await?;
                        let action = serde_json::to_value(&result)?;
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
                            ctx, session, &device, &reference, &token,
                        )
                        .await?;
                        let node_value = serde_json::to_value(&resolved.node)?;
                        let ui = ctx.ui_manager()?;
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
                            let before = match observation::collect_observation(
                                ctx, session, &device, None, false,
                            )
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
                            let result = match ui
                                .long_press_expected(&serial, &resolved.expectation)
                                .await
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
                            let action = serde_json::to_value(&result)?;
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
                        let result = ui
                            .long_press_expected(&serial, &resolved.expectation)
                            .await?;
                        let mut action = serde_json::to_value(&result)?;
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
                            "long_press",
                            Some(selector_value.clone()),
                            session,
                            &device,
                        );
                        let before = match observation::collect_observation(
                            ctx, session, &device, None, false,
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
                        let result = match ui.long_press(&serial, &selector).await {
                            Ok(result) => result,
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
                        let action = serde_json::to_value(&result)?;
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
                    let result = ui.long_press(&serial, &selector).await?;
                    let action = serde_json::to_value(&result)?;
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
        UiCommand::Text(args) => text(ctx, session.as_ref(), &device, args).await,
        UiCommand::Clear(args) => {
            reject_ref_for("ui clear", &args.selector)?;
            let selector = if args.selector.has_selector() {
                Some(require_selector(&args.selector)?)
            } else {
                None
            };
            execute_in_run!(
                ctx,
                "ui.clear",
                Some("ui clear"),
                session.as_ref(),
                Some(&device),
                true,
                |_run| async {
                    let ui = ctx.ui_manager()?;
                    let result = ui.clear(&serial, selector.as_ref()).await?;
                    Ok(success(
                        "ui.clear",
                        serde_json::to_value(&result)?,
                        "Cleared the focused text field.\n".to_string(),
                    )
                    .with_optional_session(session_id(session.as_ref()))
                    .with_device(serial.clone()))
                },
            )
            .await
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

async fn text(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiTextArgs,
) -> Result<CommandSuccess> {
    reject_ref_for("ui text", &args.selector)?;
    let selector = if args.selector.has_selector() {
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
                let selector_value = selector.as_ref().map(serde_json::to_value).transpose()?;
                let mut builder =
                    receipt::ReceiptBuilder::new("text", selector_value, session, device);
                let before =
                    match observation::collect_observation(ctx, session, device, None, false).await
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
                let result = match ui
                    .text(
                        &serial,
                        &args.value,
                        selector.as_ref(),
                        args.clear,
                        args.hide_keyboard,
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
                    None,
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
            let result = ui
                .text(
                    &serial,
                    &args.value,
                    selector.as_ref(),
                    args.clear,
                    args.hide_keyboard,
                )
                .await?;
            let action = serde_json::to_value(&result)?;
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

async fn wait_or_assert(
    ctx: &CliContext,
    session: Option<&Session>,
    device: &androidforge_core::model::Device,
    args: &UiWaitArgs,
    assertion: bool,
) -> Result<CommandSuccess> {
    let selector = require_selector(&args.selector)?;
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
            let result = ui
                .wait(&serial, &selector, timeout, args.not_exists)
                .await?;
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
                serde_json::to_value(&result)?,
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
    reject_ref_for("ui scroll", &args.selector)?;
    let selector = if args.selector.has_selector() {
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
                let selector_value = selector.as_ref().map(serde_json::to_value).transpose()?;
                let mut builder =
                    receipt::ReceiptBuilder::new("scroll", selector_value, session, device);
                let before =
                    match observation::collect_observation(ctx, session, device, None, false).await
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
                let result = match ui.scroll(&serial, &args.direction, selector.as_ref()).await {
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
                    None,
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
            let result = ui
                .scroll(&serial, &args.direction, selector.as_ref())
                .await?;
            let action = serde_json::to_value(&result)?;
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
            "Use `ui tap --ref ...` or `ui long-press --ref ...`; for other actions observe again and use a selector.",
        ));
    }
    Ok(())
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
                    observation::collect_observation(ctx, session, device, None, false)
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
                    observation::collect_observation(ctx, session, device, None, false)
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
        None => observation::collect_observation(ctx, session, device, None, false)
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
