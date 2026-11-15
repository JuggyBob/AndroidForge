use androidforge_core::error::{ForgeError, Result};
use androidforge_core::model::Device;
use androidforge_core::output::CommandSuccess;
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
    if let Some(owner) = owner {
        if let Some(object) = value.as_object_mut() {
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
