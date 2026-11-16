use androidforge_core::error::{ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::run::RunContext;
use androidforge_core::session::Session;
use androidforge_core::util::write_atomic;

use crate::cli::{
    UiCommand, UiDumpArgs, UiInspectArgs, UiKeyEventArgs, UiScrollArgs, UiScrollToArgs,
    UiSettleArgs, UiSwipeArgs, UiTextArgs, UiWaitArgs,
};
use crate::commands::common::{require_selector, success};
use crate::context::CliContext;

pub async fn ui(ctx: &CliContext, command: &UiCommand) -> Result<CommandSuccess> {
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
        UiCommand::Find(args) => {
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
                execute_in_run!(
                    ctx,
                    "ui.tap",
                    Some("ui tap coordinates"),
                    session.as_ref(),
                    Some(&device),
                    true,
                    |_run| async {
                        let ui = ctx.ui_manager()?;
                        let display = ui.tap_xy(&serial, x, y).await?;
                        Ok(success(
                            "ui.tap",
                            serde_json::json!({"x": x, "y": y, "display_size": display}),
                            format!("Tapped ({x}, {y}) on {serial}\n"),
                        )
                        .with_optional_session(session_id(session.as_ref()))
                        .with_device(serial.clone()))
                    },
                )
                .await
            } else {
                let selector = require_selector(&args.selector)?;
                execute_in_run!(
                    ctx,
                    "ui.tap",
                    Some("ui tap"),
                    session.as_ref(),
                    Some(&device),
                    true,
                    |_run| async {
                        let ui = ctx.ui_manager()?;
                        let result = ui.tap(&serial, &selector).await?;
                        Ok(success(
                            "ui.tap",
                            serde_json::to_value(&result)?,
                            format!(
                                "Tapped {} at ({}, {}){}\n",
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
                                }
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
            let selector = require_selector(&args.selector)?;
            execute_in_run!(
                ctx,
                "ui.long-press",
                Some("ui long press"),
                session.as_ref(),
                Some(&device),
                true,
                |_run| async {
                    let ui = ctx.ui_manager()?;
                    let result = ui.long_press(&serial, &selector).await?;
                    Ok(success(
                        "ui.long-press",
                        serde_json::to_value(&result)?,
                        format!("Long-pressed at ({}, {})\n", result.tap_x, result.tap_y),
                    )
                    .with_optional_session(session_id(session.as_ref()))
                    .with_device(serial.clone()))
                },
            )
            .await
        }
        UiCommand::Text(args) => text(ctx, session.as_ref(), &device, args).await,
        UiCommand::Clear(args) => {
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
    let selector = if args.selector.has_selector() {
        Some(require_selector(&args.selector)?)
    } else {
        None
    };
    let serial = device.serial.clone();
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
            let result = ui
                .text(
                    &serial,
                    &args.value,
                    selector.as_ref(),
                    args.clear,
                    args.hide_keyboard,
                )
                .await?;
            let human = format!(
                "Text entry {} for {} character(s){}.\n",
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
                }
            );
            Ok(success("ui.text", serde_json::to_value(&result)?, human)
                .with_optional_session(session.map(|session| session.id.clone()))
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
    let selector = if args.selector.has_selector() {
        Some(require_selector(&args.selector)?)
    } else {
        None
    };
    let serial = device.serial.clone();
    execute_in_run!(
        ctx,
        "ui.scroll",
        Some("ui scroll"),
        session,
        Some(device),
        true,
        |_run| async {
            let ui = ctx.ui_manager()?;
            let result = ui
                .scroll(&serial, &args.direction, selector.as_ref())
                .await?;
            Ok(success(
                "ui.scroll",
                serde_json::to_value(&result)?,
                format!(
                    "Scrolled {} within [{},{}][{},{}].\n",
                    args.direction,
                    result.target_bounds.left,
                    result.target_bounds.top,
                    result.target_bounds.right,
                    result.target_bounds.bottom
                ),
            )
            .with_optional_session(session.map(|session| session.id.clone()))
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

fn session_id(session: Option<&Session>) -> Option<String> {
    session.map(|session| session.id.clone())
}
