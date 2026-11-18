use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

#[cfg(test)]
use androidforge_core::config::DEFAULT_OBSERVATION_RETENTION;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::Device;
use androidforge_core::output::CommandSuccess;
use androidforge_core::paths::ForgePaths;
use androidforge_core::session::{Session, StateLock};
use androidforge_core::util::{
    ensure_dir, now_rfc3339, read_json, short_id, write_atomic, write_json_atomic,
};
use androidforge_ui::{Rect, UiHierarchy, UiNode};
use serde::{Deserialize, Serialize};

use crate::cli::{UiDiffArgs, UiObserveArgs};
use crate::commands::common::{self, success};
use crate::context::CliContext;
use androidforge_core::retry::ReadOnlyOperation;

pub const OBSERVATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ObservationBounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl From<Rect> for ObservationBounds {
    fn from(rect: Rect) -> Self {
        Self {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ObservationNode {
    /// Stable handles are derived from node identity. They remain valid for
    /// actions only together with the observation token that produced them.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Present on context nodes that can be acted on through a clickable
    /// ancestor. Such refs are accepted by `ui tap` and `ui long-press`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actionable_via: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub path: String,
    pub depth: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_desc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    pub bounds: ObservationBounds,
    /// Always present: agents use this to decide whether a node can be tapped.
    #[serde(default)]
    pub clickable: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub long_clickable: bool,
    /// Always present: agents use this to decide whether an element is usable.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub focused: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub scrollable: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub checked: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub editable: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub password: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub displayed: Option<bool>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn role_for(node: &UiNode) -> String {
    let class = node.class.as_deref().unwrap_or("").to_ascii_lowercase();
    if node.editable_like() {
        "edit_text".to_string()
    } else if class.contains("button") {
        "button".to_string()
    } else if class.contains("checkbox") || class.contains("switch") {
        "toggle".to_string()
    } else if class.contains("tab") {
        "tab".to_string()
    } else if node.scrollable {
        "scrollable".to_string()
    } else if node.clickable || node.long_clickable {
        "actionable".to_string()
    } else if class.contains("image") {
        "image".to_string()
    } else if class.contains("textview") || class.contains("text") {
        "text".to_string()
    } else {
        "other".to_string()
    }
}

fn non_empty(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

impl ObservationNode {
    fn from_node(node: &UiNode) -> Self {
        let password = node.password;
        Self {
            reference: None,
            actionable_via: None,
            role: Some(role_for(node)),
            path: node.path.clone(),
            depth: node.depth,
            text: if password {
                node.text.as_ref().map(|_| "[password]".to_string())
            } else {
                non_empty(&node.text)
            },
            content_desc: non_empty(&node.content_desc),
            hint: non_empty(&node.hint),
            resource_id: non_empty(&node.resource_id),
            class: non_empty(&node.class),
            package: non_empty(&node.package),
            bounds: node.bounds.into(),
            clickable: node.clickable,
            long_clickable: node.long_clickable,
            enabled: node.enabled,
            focused: node.focused,
            selected: node.selected,
            scrollable: node.scrollable,
            checked: node.checked,
            editable: node.editable_like(),
            password,
            displayed: node.displayed,
        }
    }

    fn meaningful(&self) -> bool {
        self.clickable
            || self.long_clickable
            || self.scrollable
            || self.focused
            || self.editable
            || self.text.as_deref().is_some_and(|value| !value.is_empty())
            || self
                .content_desc
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            || self.hint.as_deref().is_some_and(|value| !value.is_empty())
            || self
                .resource_id
                .as_deref()
                .is_some_and(|value| !value.is_empty())
    }

    fn label(&self) -> Option<&str> {
        self.text
            .as_deref()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                self.content_desc
                    .as_deref()
                    .filter(|value| !value.is_empty())
            })
            .or_else(|| self.hint.as_deref().filter(|value| !value.is_empty()))
            .or_else(|| {
                self.resource_id
                    .as_deref()
                    .filter(|value| !value.is_empty())
            })
    }

    /// A label safe to include in receipts (passwords are already masked).
    pub fn label_for_receipt(&self) -> Option<String> {
        self.label().map(str::to_string)
    }

    fn is_layout_container(&self) -> bool {
        let class = self.class.as_deref().unwrap_or("").to_ascii_lowercase();
        matches!(
            class.as_str(),
            "android.widget.framelayout"
                | "android.widget.linearlayout"
                | "android.widget.relativelayout"
                | "android.widget.viewgroup"
                | "androidx.compose.ui.platform.composeview"
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationDisplay {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    pub orientation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationForeground {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub schema_version: u32,
    pub id: String,
    /// Preferred alias for `id`; observation tokens are passed to `ui diff --from`.
    #[serde(default)]
    pub token: String,
    pub session_id: String,
    pub device_serial: String,
    pub observed_at: String,
    pub age_ms: u64,
    pub stale: bool,
    pub display: ObservationDisplay,
    pub foreground: ObservationForeground,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<ObservationNode>,
    #[serde(default)]
    pub actionable: Vec<ObservationNode>,
    #[serde(default)]
    pub actionable_total: usize,
    #[serde(default, skip_serializing_if = "is_false")]
    pub actionable_truncated: bool,
    /// Meaningful non-interactive text/context nodes.
    #[serde(default)]
    pub context: Vec<ObservationNode>,
    #[serde(default)]
    pub context_total: usize,
    #[serde(default, skip_serializing_if = "is_false")]
    pub context_truncated: bool,
    #[serde(default)]
    pub scrollable: Vec<ObservationNode>,
    #[serde(default)]
    pub visible_text: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodes: Option<Vec<ObservationNode>>,
    pub hierarchy: ObservationHierarchyMeta,
    pub raw_xml_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationHierarchyMeta {
    pub package: Option<String>,
    pub rotation: Option<i32>,
    pub node_count: usize,
    pub compact_node_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ObservationRecord {
    observation: Observation,
    boot_id: Option<String>,
    session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DiffNode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_desc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    pub bounds: ObservationBounds,
    #[serde(default)]
    pub clickable: bool,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub focused: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub scrollable: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub checked: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub editable: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub password: bool,
}

impl From<&ObservationNode> for DiffNode {
    fn from(node: &ObservationNode) -> Self {
        Self {
            text: node.text.clone(),
            content_desc: node.content_desc.clone(),
            hint: node.hint.clone(),
            resource_id: node.resource_id.clone(),
            class: node.class.clone(),
            bounds: node.bounds.clone(),
            clickable: node.clickable,
            enabled: node.enabled,
            focused: node.focused,
            scrollable: node.scrollable,
            checked: node.checked,
            editable: node.editable,
            password: node.password,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ObservationChanges {
    #[serde(default)]
    pub added: Vec<DiffNode>,
    #[serde(default)]
    pub removed: Vec<DiffNode>,
    #[serde(default)]
    pub modified: Vec<NodeModification>,
    pub added_total: usize,
    pub removed_total: usize,
    pub modified_total: usize,
    #[serde(default)]
    pub truncated: bool,
    pub focus_changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_package_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_package_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_activity_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_activity_after: Option<String>,
    pub package_changed: bool,
    pub activity_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeModification {
    pub path: String,
    pub changes: Vec<FieldChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldChange {
    pub field: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationDiff {
    pub schema_version: u32,
    pub from: String,
    pub to: String,
    pub session_id: String,
    pub device_serial: String,
    pub changes: ObservationChanges,
}

fn observations_dir(paths: &ForgePaths, session_id: &str) -> PathBuf {
    paths
        .state_dir
        .join("observations")
        .join(sanitize_component(session_id))
}

fn sanitize_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn observation_path(paths: &ForgePaths, session_id: &str, token: &str) -> PathBuf {
    observations_dir(paths, session_id).join(format!("{}.json", sanitize_component(token)))
}

fn allocate_token(paths: &ForgePaths, session_id: &str) -> String {
    loop {
        let candidate = format!("obs-{}", short_id());
        if !observation_path(paths, session_id, &candidate).exists() {
            return candidate;
        }
    }
}

fn xml_path(paths: &ForgePaths, session_id: &str, token: &str) -> PathBuf {
    observations_dir(paths, session_id).join(format!("{}.xml", sanitize_component(token)))
}

/// Evict the oldest observations until at most `target` remain.
///
/// Ordering is by the stored `observed_at` value, then file modification time
/// and finally the token. Random token filenames are never used as the primary
/// order. `keep_token` is always preserved so a just-written observation can
/// never be evicted by its own retention pass.
fn prune_observations(
    paths: &ForgePaths,
    session_id: &str,
    target: usize,
    keep_token: Option<&str>,
) -> Result<()> {
    let dir = observations_dir(paths, session_id);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!(
                    "could not inspect the observation store {}: {error}",
                    dir.display()
                ),
            )
            .with_suggestion(
                "Check that the AndroidForge state directory is writable, then observe again.",
            ));
        }
    };
    let keep_file = keep_token.map(|token| format!("{}.json", sanitize_component(token)));
    let mut json_files = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter(|path| {
            keep_file
                .as_deref()
                .is_none_or(|keep| path.file_name().and_then(|name| name.to_str()) != Some(keep))
        })
        .map(|path| {
            let observed_at = read_json::<ObservationRecord>(&path)
                .ok()
                .map(|record| record.observation.observed_at)
                .unwrap_or_default();
            let modified = std::fs::metadata(&path)
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            let token = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            (observed_at, modified, token, path)
        })
        .collect::<Vec<_>>();
    let total = json_files.len() + usize::from(keep_token.is_some());
    if total <= target {
        return Ok(());
    }
    let remove_count = total - target;
    json_files.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    for (.., path) in json_files.into_iter().take(remove_count) {
        std::fs::remove_file(&path).map_err(|error| {
            ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!(
                    "could not evict old observation {}: {error}",
                    path.display()
                ),
            )
            .with_suggestion(
                "Free space or fix permissions in the AndroidForge state directory, then observe again.",
            )
        })?;
        let _ = std::fs::remove_file(path.with_extension("xml"));
    }
    Ok(())
}

async fn boot_id_async(adb: &androidforge_adb::AdbClient, serial: &str) -> Option<String> {
    adb.shell_args(
        serial,
        &[
            "cat".to_string(),
            "/proc/sys/kernel/random/boot_id".to_string(),
        ],
        Some(Duration::from_secs(5)),
        None,
    )
    .await
    .ok()
    .map(|output| output.stdout_text().trim().to_string())
    .filter(|value| !value.is_empty())
}

fn display_orientation(rotation: Option<i32>) -> String {
    match rotation.unwrap_or(0).rem_euclid(4) {
        1 => "landscape_left".to_string(),
        2 => "reverse_portrait".to_string(),
        3 => "landscape_right".to_string(),
        _ => "portrait".to_string(),
    }
}

fn reference_for(node: &ObservationNode, length: usize) -> String {
    let bounds = format!(
        "{},{},{},{}",
        node.bounds.left, node.bounds.top, node.bounds.right, node.bounds.bottom
    );
    // The observation token is deliberately not part of the ref material:
    // unchanged nodes must keep the same ref across repeated observations.
    let material = format!(
        "v2|{}|{}|{}|{}|{}|{}|{bounds}|{}",
        node.path,
        node.class.as_deref().unwrap_or(""),
        node.package.as_deref().unwrap_or(""),
        node.resource_id.as_deref().unwrap_or(""),
        node.text.as_deref().unwrap_or(""),
        node.content_desc.as_deref().unwrap_or(""),
        node.role.as_deref().unwrap_or("")
    );
    let hash = androidforge_core::util::sha256_hex(material.as_bytes());
    format!("el-{}", &hash[..length.min(hash.len())])
}

fn assign_references(observation: &mut Observation) {
    let mut references = BTreeMap::new();
    let mut used = BTreeSet::new();
    // Full-hierarchy node copies do not carry compact context metadata. Keep
    // the same `actionable_via` marker on every copy of a path so a ref works
    // identically with and without `--full`.
    let actionable_via = observation
        .context
        .iter()
        .filter_map(|node| {
            node.actionable_via
                .as_ref()
                .map(|via| (node.path.clone(), via.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    for node in observation
        .actionable
        .iter()
        .chain(observation.context.iter())
        .chain(observation.scrollable.iter())
        .chain(observation.focused.iter())
        .chain(observation.nodes.iter().flatten())
    {
        if references.contains_key(&node.path) {
            continue;
        }
        let mut length = 12;
        let reference = loop {
            let candidate = reference_for(node, length);
            if used.insert(candidate.clone()) {
                break candidate;
            }
            length += 4;
            if length > 32 {
                break reference_for(node, 64);
            }
        };
        references.insert(node.path.clone(), reference);
    }
    let assign = |node: &mut ObservationNode| {
        if let Some(reference) = references.get(&node.path) {
            node.reference = Some(reference.clone());
        }
    };
    for node in &mut observation.actionable {
        assign(node);
    }
    for node in &mut observation.context {
        assign(node);
    }
    for node in &mut observation.scrollable {
        assign(node);
    }
    if let Some(node) = &mut observation.focused {
        assign(node);
        if let Some(via) = actionable_via.get(&node.path) {
            node.actionable_via = Some(via.clone());
        }
    }
    if let Some(nodes) = &mut observation.nodes {
        for node in nodes {
            assign(node);
            if let Some(via) = actionable_via.get(&node.path) {
                node.actionable_via = Some(via.clone());
            }
        }
    }
}

/// Find a node by its opaque reference inside one observation.
pub fn find_reference<'a>(
    observation: &'a Observation,
    reference: &str,
) -> Option<&'a ObservationNode> {
    observation
        .actionable
        .iter()
        .chain(observation.scrollable.iter())
        .chain(observation.context.iter())
        .chain(observation.focused.iter())
        .chain(observation.nodes.iter().flatten())
        .find(|node| node.reference.as_deref() == Some(reference))
}

pub struct ResolvedElementReference {
    pub node: ObservationNode,
    pub expectation: androidforge_ui::NodeExpectation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceUse {
    Tap,
    LongPress,
    Text,
    Clear,
    Scroll,
    Wait,
    Assert,
}

impl ReferenceUse {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tap => "tap",
            Self::LongPress => "long_press",
            Self::Text => "text",
            Self::Clear => "clear",
            Self::Scroll => "scroll",
            Self::Wait => "wait",
            Self::Assert => "assert",
        }
    }

    fn requires_actionable(self) -> bool {
        !matches!(self, Self::Wait | Self::Assert)
    }

    fn requires_editable(self) -> bool {
        matches!(self, Self::Text | Self::Clear)
    }

    fn requires_scrollable(self) -> bool {
        matches!(self, Self::Scroll)
    }
}

/// Validate a stable element reference against its observation and the current
/// session/device. This does not read the hierarchy; the UI manager performs a
/// final identity check immediately before dispatch.
#[allow(clippy::too_many_arguments)]
pub async fn resolve_element_reference(
    ctx: &CliContext,
    session: &Session,
    device: &Device,
    reference: &str,
    token: &str,
    usage: ReferenceUse,
) -> Result<ResolvedElementReference> {
    if !reference.starts_with("el-") {
        return Err(ForgeError::new(
            ErrorCode::ElementRefUnknown,
            format!("'{reference}' is not an AndroidForge element reference"),
        )
        .with_suggestion("Use the `ref` value returned by `ui observe --json`.")
        .session(session.id.clone())
        .device(device.serial.clone()));
    }
    let record = load_observation(&ctx.paths, &session.id, token)?;
    verify_boot_id(ctx, session, &record).await?;
    let observation = record.observation();
    if observation.device_serial != device.serial {
        return Err(ForgeError::new(
            ErrorCode::ElementDeviceMismatch,
            format!(
                "observation {token} belongs to device {}, not {}",
                observation.device_serial, device.serial
            ),
        )
        .with_suggestion("Re-observe on the current device and choose a new ref.")
        .session(session.id.clone())
        .device(device.serial.clone()));
    }
    let Some(node) = find_reference(&observation, reference) else {
        return Err(ForgeError::new(
            ErrorCode::ElementRefUnknown,
            format!("element reference '{reference}' is not part of observation {token}"),
        )
        .with_context_string("ref", reference)
        .with_context_string("token", token)
        .with_suggestion("References are valid only inside the observation that produced them.")
        .with_suggestion(
            "Run `androidforge ui observe --session <ID> --json` and choose a new ref.",
        )
        .session(session.id.clone())
        .device(device.serial.clone()));
    };
    let self_actionable = observation
        .actionable
        .iter()
        .chain(observation.scrollable.iter())
        .any(|candidate| candidate.reference.as_deref() == Some(reference));
    // A context label inside a clickable row is a legitimate tap target: the
    // UI manager already resolves and dispatches on the nearest clickable
    // ancestor. Keep this fail-closed by requiring an ancestor that was
    // observed as clickable; there is no coordinate fallback.
    let clickable_ancestor = !self_actionable && node.actionable_via.as_deref() == Some("ancestor");
    if clickable_ancestor && !ctx.config.file.ui.tap_clickable_ancestor {
        return Err(ForgeError::new(
            ErrorCode::ElementNotActionable,
            "the referenced context element is actionable only through a clickable ancestor, but ancestor dispatch is disabled",
        )
        .with_context_string("ref", reference)
        .with_context_string("setting", "ui.tap_clickable_ancestor")
        .with_suggestion(
            "Enable ui.tap_clickable_ancestor in the project adapter, or target the clickable ancestor ref.",
        )
        .session(session.id.clone())
        .device(device.serial.clone()));
    }
    if usage.requires_actionable() && !self_actionable && !clickable_ancestor {
        let context_only = observation
            .context
            .iter()
            .any(|candidate| candidate.reference.as_deref() == Some(reference));
        let message = if context_only {
            "the referenced element is context text with no clickable ancestor"
        } else {
            "the referenced element is not in the observation's actionable set"
        };
        return Err(ForgeError::new(ErrorCode::ElementNotActionable, message)
            .with_context_string("ref", reference)
            .with_context_string("use", usage.as_str())
            .with_context(
                "actionable_via",
                observation
                    .context
                    .iter()
                    .find(|candidate| candidate.reference.as_deref() == Some(reference))
                    .and_then(|candidate| candidate.actionable_via.clone())
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null),
            )
            .with_suggestion("Choose a ref from the observation's `actionable` list.")
            .with_suggestion(
                "`ui wait` and `ui assert` may reference context text without an ancestor.",
            )
            .session(session.id.clone())
            .device(device.serial.clone()));
    }
    if usage.requires_editable() && !node.editable {
        return Err(ForgeError::new(
            ErrorCode::ElementNotActionable,
            "the referenced element is not an editable text field",
        )
        .with_context_string("ref", reference)
        .with_context_string("use", usage.as_str())
        .with_suggestion("Choose an EditText/TextField ref from `ui observe --json`.")
        .session(session.id.clone())
        .device(device.serial.clone()));
    }
    if usage.requires_scrollable() && !node.scrollable {
        return Err(ForgeError::new(
            ErrorCode::ElementNotActionable,
            "the referenced element is not scrollable",
        )
        .with_context_string("ref", reference)
        .with_context_string("use", usage.as_str())
        .with_suggestion("Choose a scrollable container ref from `ui observe --json`.")
        .session(session.id.clone())
        .device(device.serial.clone()));
    }
    let expectation = androidforge_ui::NodeExpectation {
        path: node.path.clone(),
        class: node.class.clone(),
        package: node.package.clone(),
        resource_id: node.resource_id.clone(),
        text: if node.password {
            None
        } else {
            node.text.clone()
        },
        content_desc: node.content_desc.clone(),
        bounds: androidforge_ui::Rect {
            left: node.bounds.left,
            top: node.bounds.top,
            right: node.bounds.right,
            bottom: node.bounds.bottom,
        },
        clickable: node.clickable,
        long_clickable: node.long_clickable,
        enabled: node.enabled,
        scrollable: node.scrollable,
        editable: node.editable,
        password: node.password,
        actionable_hint: self_actionable,
    };
    if usage.requires_actionable() && !node.enabled {
        return Err(ForgeError::new(
            ErrorCode::ElementNotActionable,
            "the referenced element was disabled when it was observed",
        )
        .with_context_string("ref", reference)
        .with_suggestion("Wait for it to become enabled, observe again and choose a new ref.")
        .session(session.id.clone())
        .device(device.serial.clone()));
    }
    Ok(ResolvedElementReference {
        node: node.clone(),
        expectation,
    })
}

fn compact_nodes(
    hierarchy: &UiHierarchy,
) -> (
    Vec<ObservationNode>,
    Vec<ObservationNode>,
    Vec<ObservationNode>,
    Option<ObservationNode>,
) {
    let mut actionable = Vec::new();
    let mut context = Vec::new();
    let mut scrollable = Vec::new();
    let mut focused = None;
    for node in &hierarchy.nodes {
        if !node.visible() && !node.focused {
            continue;
        }
        let mut compact = ObservationNode::from_node(node);
        if compact.focused {
            focused = Some(compact.clone());
        }
        // Keep disabled elements only when they still carry useful context.
        if !compact.enabled && !compact.meaningful() {
            continue;
        }
        if compact.scrollable {
            scrollable.push(compact.clone());
        }
        let has_label = compact
            .text
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            || compact
                .content_desc
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            || compact
                .hint
                .as_deref()
                .is_some_and(|value| !value.is_empty());
        let class = compact.class.as_deref().unwrap_or("").to_ascii_lowercase();
        let actionable_class = class.contains("button")
            || class.contains("checkbox")
            || class.contains("switch")
            || class.contains("seekbar")
            || class.contains("spinner")
            || class.contains("tab")
            || class.contains("edittext")
            || class.contains("textfield")
            || class.contains("autocompletetextview")
            || class.contains("searchview");
        // Compose and some system views expose navigation/actions as
        // android.view.View with a content description but clickable=false.
        let content_action = compact.content_desc.is_some()
            && !class.contains("textview")
            && !class.contains("text");
        let interactive = compact.clickable
            || compact.long_clickable
            || compact.scrollable
            || compact.focused
            || compact.editable
            || actionable_class
            || content_action;
        if interactive {
            let has_id = compact
                .resource_id
                .as_deref()
                .is_some_and(|id| !id.is_empty());
            if !has_label && !has_id && !compact.scrollable && !compact.focused && !compact.editable
            {
                continue;
            }
            if compact.is_layout_container()
                && !has_label
                && !compact.scrollable
                && !compact.focused
            {
                continue;
            }
            if compact.meaningful() {
                actionable.push(compact);
            }
        } else if has_label && !compact.is_layout_container() && compact.meaningful() {
            // Context text inside a clickable row is a valid tap target through
            // that ancestor, even when the ancestor itself is a layout
            // container and therefore not listed as actionable.
            if hierarchy.clickable_ancestor(node).is_some() {
                compact.actionable_via = Some("ancestor".to_string());
            }
            context.push(compact);
        }
    }
    actionable.sort_by_key(|node| {
        let rank = if node.focused {
            0
        } else if node.clickable || node.long_clickable || node.editable {
            1
        } else if node.scrollable {
            2
        } else {
            3
        };
        (rank, node.depth, node.bounds.top, node.bounds.left)
    });
    context.sort_by_key(|node| (node.depth, node.bounds.top, node.bounds.left));
    scrollable.sort_by_key(|left| left.depth);
    (actionable, context, scrollable, focused)
}

pub async fn collect_observation(
    ctx: &CliContext,
    session: &Session,
    device: &Device,
    package_filter: Option<&str>,
    include_nodes: bool,
) -> Result<Observation> {
    collect_observation_scoped(
        ctx,
        session,
        device,
        package_filter,
        include_nodes,
        include_nodes,
    )
    .await
}

/// Receipts need the complete actionable/context lists so a text or state
/// change outside the compact cap is still classified correctly. They do not
/// need the raw full hierarchy in their stored observations.
pub async fn collect_receipt_observation(
    ctx: &CliContext,
    session: &Session,
    device: &Device,
    package_filter: Option<&str>,
) -> Result<Observation> {
    collect_observation_scoped(ctx, session, device, package_filter, false, true).await
}

async fn collect_observation_scoped(
    ctx: &CliContext,
    session: &Session,
    device: &Device,
    package_filter: Option<&str>,
    include_nodes: bool,
    include_all_nodes: bool,
) -> Result<Observation> {
    let serial = device.serial.clone();
    let ui = ctx.ui_manager()?;
    let hierarchy = ui.dump(&serial).await?;
    let adb = ctx.adb()?;
    let focus = adb.current_focus(&serial).await;
    let display_size = adb.display_size(&serial).await;
    let boot = boot_id_async(&adb, &serial).await;
    // Serialize token allocation, writes and retention across independent CLI
    // processes. The lock is held only for synchronous state-file work.
    let _state_lock = StateLock::acquire(&ctx.paths)?;
    let retention = ctx.config.file.storage.observation_retention;
    let token = allocate_token(&ctx.paths, &session.id);
    let observed_at = now_rfc3339();
    let (mut actionable, mut context, mut scrollable, focused) = compact_nodes(&hierarchy);
    if let Some(package) = package_filter {
        let belongs = |node: &ObservationNode| {
            node.package.as_deref() == Some(package)
                || node
                    .resource_id
                    .as_deref()
                    .is_some_and(|id| id.starts_with(package))
        };
        actionable.retain(belongs);
        context.retain(belongs);
        scrollable.retain(belongs);
    }
    let mut visible_text = actionable
        .iter()
        .chain(context.iter())
        .filter_map(|node| node.label().map(str::to_string))
        .collect::<Vec<_>>();
    visible_text.sort();
    visible_text.dedup();
    visible_text.truncate(32);
    let actionable_total = actionable.len();
    let context_total = context.len();
    let actionable_truncated = !include_all_nodes && actionable_total > 24;
    if actionable_truncated {
        actionable.truncate(24);
    }
    let context_truncated = !include_all_nodes && context_total > 16;
    if context_truncated {
        context.truncate(16);
    }

    let raw_dir = observations_dir(&ctx.paths, &session.id);
    ensure_dir(&raw_dir)?;
    // Make room before writing so the new record cannot be selected for
    // eviction by this observation's own retention pass.
    prune_observations(&ctx.paths, &session.id, retention.saturating_sub(1), None)?;
    let raw = xml_path(&ctx.paths, &session.id, &token);
    write_atomic(&raw, hierarchy.xml.as_bytes())?;

    let foreground = ObservationForeground {
        package: focus
            .as_ref()
            .map(|focus| focus.package.clone())
            .or_else(|| hierarchy.package_name.clone()),
        activity: focus.as_ref().and_then(|focus| focus.activity.clone()),
        window: focus.and_then(|focus| focus.window),
    };
    let display = ObservationDisplay {
        width: display_size.map(|size| size.0),
        height: display_size.map(|size| size.1),
        orientation: display_orientation(hierarchy.rotation),
        rotation: hierarchy.rotation,
    };
    let mut observation = Observation {
        schema_version: OBSERVATION_SCHEMA_VERSION,
        id: token.clone(),
        token: token.clone(),
        session_id: session.id.clone(),
        device_serial: serial.clone(),
        observed_at,
        age_ms: 0,
        stale: false,
        display,
        foreground,
        focused,
        actionable,
        actionable_total,
        actionable_truncated,
        context,
        context_total,
        context_truncated,
        scrollable,
        visible_text,
        nodes: include_nodes.then(|| {
            hierarchy
                .nodes
                .iter()
                .map(ObservationNode::from_node)
                .collect()
        }),
        hierarchy: ObservationHierarchyMeta {
            package: hierarchy.package_name.clone(),
            rotation: hierarchy.rotation,
            node_count: hierarchy.nodes.len(),
            compact_node_count: actionable_total + context_total,
        },
        raw_xml_path: raw.to_string_lossy().into_owned(),
    };
    assign_references(&mut observation);
    let record = ObservationRecord {
        observation: observation.clone(),
        boot_id: boot,
        session_id: session.id.clone(),
    };
    write_json_atomic(&observation_path(&ctx.paths, &session.id, &token), &record)?;
    prune_observations(&ctx.paths, &session.id, retention, Some(&token))?;
    Ok(observation)
}

pub fn load_observation(
    paths: &ForgePaths,
    session_id: &str,
    token: &str,
) -> Result<ObservationRecordView> {
    let path = observation_path(paths, session_id, token);
    if !path.is_file() {
        if let Some(other_session) = observation_owner(paths, token) {
            return Err(ForgeError::new(
                ErrorCode::ObservationSessionMismatch,
                format!(
                    "observation '{token}' belongs to session {other_session}, not {session_id}"
                ),
            )
            .with_context_string("observation_session", other_session.clone())
            .with_suggestion(format!(
                "Use a token returned for session {session_id}, or pass --session {other_session} if that is the intended session."
            ))
            .session(session_id.to_string()));
        }
        return Err(ForgeError::new(
            ErrorCode::ObservationNotFound,
            format!("observation '{token}' was not found for session {session_id}"),
        )
        .with_suggestion(
            "Run `androidforge ui observe --session <ID> --json` to create a fresh observation.",
        )
        .with_suggestion(
            "Observation tokens are retained per session; create a new one if this token expired.",
        ));
    }
    let record: ObservationRecord = read_json(&path)?;
    if record.session_id != session_id || record.observation.session_id != session_id {
        return Err(ForgeError::new(
            ErrorCode::ObservationSessionMismatch,
            format!("observation '{token}' belongs to a different session"),
        )
        .with_suggestion("Use a token returned for the same session.")
        .session(session_id.to_string()));
    }
    Ok(ObservationRecordView { record })
}

fn observation_owner(paths: &ForgePaths, token: &str) -> Option<String> {
    let root = paths.state_dir.join("observations");
    let entries = std::fs::read_dir(root).ok()?;
    let file = format!("{}.json", sanitize_component(token));
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let candidate = entry.path().join(&file);
        if candidate.is_file() {
            if let Ok(record) = read_json::<ObservationRecord>(&candidate) {
                return Some(record.session_id);
            }
            return entry.file_name().to_str().map(str::to_string);
        }
    }
    None
}

#[derive(Debug)]
pub struct ObservationRecordView {
    record: ObservationRecord,
}

impl ObservationRecordView {
    pub fn observation(&self) -> Observation {
        let mut observation = self.record.observation.clone();
        observation.age_ms = chrono::DateTime::parse_from_rfc3339(&observation.observed_at)
            .ok()
            .map(|observed| {
                chrono::Utc::now()
                    .signed_duration_since(observed.with_timezone(&chrono::Utc))
                    .num_milliseconds()
                    .max(0) as u64
            })
            .unwrap_or(0);
        observation.stale = observation.age_ms > 24 * 60 * 60 * 1000;
        if observation.token.is_empty() {
            observation.token = observation.id.clone();
        }
        observation
    }

    pub fn boot_id(&self) -> Option<&str> {
        self.record.boot_id.as_deref()
    }
}

pub async fn verify_boot_id(
    ctx: &CliContext,
    session: &Session,
    record: &ObservationRecordView,
) -> Result<()> {
    let observation = record.observation();
    if observation.age_ms > 24 * 60 * 60 * 1000 {
        return Err(ForgeError::new(
            ErrorCode::ObservationExpired,
            format!(
                "observation {} is {:.1} hours old; tokens expire after 24 hours",
                observation.id,
                observation.age_ms as f64 / 3_600_000.0
            ),
        )
        .with_context_string("observation_id", observation.id)
        .with_suggestion("Run `androidforge ui observe --session <ID> --json` for a fresh token.")
        .session(session.id.clone()));
    }
    let Some(expected) = record.boot_id() else {
        return Ok(());
    };
    let adb = ctx.adb()?;
    let serial = session.serial()?.to_string();
    if let Some(actual) = boot_id_async(&adb, &serial).await
        && actual != expected
    {
        return Err(ForgeError::new(
            ErrorCode::ObservationExpired,
            "the device rebooted after this observation; the old screen state is not current",
        )
        .with_suggestion("Run `androidforge ui observe --session <ID> --json` for a fresh token.")
        .session(session.id.clone())
        .device(serial));
    }
    Ok(())
}

pub fn latest_observation(paths: &ForgePaths, session_id: &str) -> Option<Observation> {
    let dir = observations_dir(paths, session_id);
    let entries = std::fs::read_dir(dir).ok()?;
    let mut records = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter_map(|path| {
            let record = read_json::<ObservationRecord>(&path).ok()?;
            let modified = std::fs::metadata(&path)
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            Some((record.observation.observed_at.clone(), modified, record))
        })
        .collect::<Vec<_>>();
    records.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let record = records.pop()?.2;
    let mut observation = record.observation;
    observation.age_ms = chrono::DateTime::parse_from_rfc3339(&observation.observed_at)
        .ok()
        .map(|observed| {
            chrono::Utc::now()
                .signed_duration_since(observed.with_timezone(&chrono::Utc))
                .num_milliseconds()
                .max(0) as u64
        })
        .unwrap_or(0);
    observation.stale = observation.age_ms > 24 * 60 * 60 * 1000;
    if observation.token.is_empty() {
        observation.token = observation.id.clone();
    }
    Some(observation)
}

fn semantic_key(node: &ObservationNode) -> String {
    match &node.resource_id {
        Some(id) if !id.is_empty() => format!(
            "id:{}|{}|{}",
            node.package.as_deref().unwrap_or(""),
            id,
            node.class.as_deref().unwrap_or("")
        ),
        _ => format!(
            "path:{}|{}|{}",
            node.path,
            node.class.as_deref().unwrap_or(""),
            node.package.as_deref().unwrap_or("")
        ),
    }
}

fn group_nodes(observation: &Observation) -> BTreeMap<String, Vec<ObservationNode>> {
    let mut groups: BTreeMap<String, Vec<ObservationNode>> = BTreeMap::new();
    for node in observation
        .actionable
        .iter()
        .chain(observation.context.iter())
    {
        groups
            .entry(semantic_key(node))
            .or_default()
            .push(node.clone());
    }
    groups
}

fn field_change(field: &str, before: impl ToString, after: impl ToString) -> FieldChange {
    FieldChange {
        field: field.to_string(),
        before: before.to_string(),
        after: after.to_string(),
    }
}

fn bounds_change_relevant(
    before: &ObservationBounds,
    after: &ObservationBounds,
    display: &ObservationDisplay,
) -> bool {
    let before_area =
        (before.right - before.left).max(0) as i64 * (before.bottom - before.top).max(0) as i64;
    let after_area =
        (after.right - after.left).max(0) as i64 * (after.bottom - after.top).max(0) as i64;
    if before_area == 0 || after_area == 0 {
        return true;
    }
    if (after_area - before_area).unsigned_abs() as f64 / before_area as f64 > 0.25 {
        return true;
    }
    let width = display.width.unwrap_or(1080).max(1) as f64;
    let height = display.height.unwrap_or(2400).max(1) as f64;
    let dx = (after.left - before.left).abs() as f64;
    let dy = (after.top - before.top).abs() as f64;
    dx > width * 0.05 || dy > height * 0.05
}

fn compare_nodes(
    before: &ObservationNode,
    after: &ObservationNode,
    display: &ObservationDisplay,
) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    if before.text != after.text {
        changes.push(field_change(
            "text",
            before.text.clone().unwrap_or_default(),
            after.text.clone().unwrap_or_default(),
        ));
    }
    if before.content_desc != after.content_desc {
        changes.push(field_change(
            "content_desc",
            before.content_desc.clone().unwrap_or_default(),
            after.content_desc.clone().unwrap_or_default(),
        ));
    }
    if before.resource_id != after.resource_id {
        changes.push(field_change(
            "resource_id",
            before.resource_id.clone().unwrap_or_default(),
            after.resource_id.clone().unwrap_or_default(),
        ));
    }
    if before.enabled != after.enabled {
        changes.push(field_change("enabled", before.enabled, after.enabled));
    }
    if before.clickable != after.clickable {
        changes.push(field_change("clickable", before.clickable, after.clickable));
    }
    if before.focused != after.focused {
        changes.push(field_change("focused", before.focused, after.focused));
    }
    if before.selected != after.selected {
        changes.push(field_change("selected", before.selected, after.selected));
    }
    if before.scrollable != after.scrollable {
        changes.push(field_change(
            "scrollable",
            before.scrollable,
            after.scrollable,
        ));
    }
    if before.checked != after.checked {
        changes.push(field_change("checked", before.checked, after.checked));
    }
    // Pixel movement alone is layout noise. Report bounds only when the node's
    // area or position changed enough to be meaningful to a human or selector.
    if before.bounds != after.bounds
        && bounds_change_relevant(&before.bounds, &after.bounds, display)
    {
        changes.push(field_change(
            "bounds",
            format!(
                "[{},{}][{},{}]",
                before.bounds.left, before.bounds.top, before.bounds.right, before.bounds.bottom
            ),
            format!(
                "[{},{}][{},{}]",
                after.bounds.left, after.bounds.top, after.bounds.right, after.bounds.bottom
            ),
        ));
    }
    changes
}

pub fn diff_observations(before: &Observation, after: &Observation, full: bool) -> ObservationDiff {
    let mut before_groups = group_nodes(before);
    let mut changes = ObservationChanges::default();
    for node in after.actionable.iter().chain(after.context.iter()) {
        let key = semantic_key(node);
        let previous = before_groups
            .get_mut(&key)
            .and_then(|group| (!group.is_empty()).then(|| group.remove(0)));
        match previous {
            None => changes.added.push(DiffNode::from(node)),
            Some(previous) => {
                let fields = compare_nodes(&previous, node, &after.display);
                if !fields.is_empty() {
                    changes.modified.push(NodeModification {
                        path: node.path.clone(),
                        changes: fields,
                    });
                }
            }
        }
    }
    for group in before_groups.values() {
        changes.removed.extend(group.iter().map(DiffNode::from));
    }
    changes.added_total = changes.added.len();
    changes.removed_total = changes.removed.len();
    changes.modified_total = changes.modified.len();
    let max_nodes: usize = if full { 100 } else { 12 };
    if changes.added.len() > max_nodes {
        changes.added.truncate(max_nodes);
        changes.truncated = true;
    }
    if changes.removed.len() > max_nodes {
        changes.removed.truncate(max_nodes);
        changes.truncated = true;
    }
    if changes.modified.len() > max_nodes {
        changes.modified.truncate(max_nodes);
        changes.truncated = true;
    }
    changes.focus_changed = before.focused.as_ref().map(|node| &node.path)
        != after.focused.as_ref().map(|node| &node.path);
    changes.focus_before = before
        .focused
        .as_ref()
        .and_then(|node| node.label().map(str::to_string));
    changes.focus_after = after
        .focused
        .as_ref()
        .and_then(|node| node.label().map(str::to_string));
    changes.foreground_package_before = before.foreground.package.clone();
    changes.foreground_package_after = after.foreground.package.clone();
    changes.foreground_activity_before = before.foreground.activity.clone();
    changes.foreground_activity_after = after.foreground.activity.clone();
    changes.package_changed = before.foreground.package != after.foreground.package;
    changes.activity_changed = before.foreground.activity != after.foreground.activity;
    ObservationDiff {
        schema_version: OBSERVATION_SCHEMA_VERSION,
        from: before.id.clone(),
        to: after.id.clone(),
        session_id: after.session_id.clone(),
        device_serial: after.device_serial.clone(),
        changes,
    }
}

pub async fn observe_command(
    ctx: &CliContext,
    session: &Session,
    device: &Device,
    args: &UiObserveArgs,
) -> Result<CommandSuccess> {
    if ctx.dry_run {
        return Ok(success(
            "ui.observe",
            serde_json::json!({
                "dry_run": true,
                "session_id": session.id,
                "device_serial": device.serial,
                "package": args.package,
                "since": args.since,
                "full": args.full,
                "would_capture": "UI hierarchy, foreground focus, display size and boot id",
                "would_write": "a compact observation token and raw hierarchy artifact",
            }),
            format!(
                "Would observe {} on {} without writing an observation token.\n",
                device.serial, session.id
            ),
        )
        .with_session(session.id.clone())
        .with_device(device.serial.clone()));
    }
    let (observation, retry_report) =
        common::retry_read_only(ReadOnlyOperation::UiObserve, args.retries, || {
            collect_observation(ctx, session, device, args.package.as_deref(), args.full)
        })
        .await?;
    let since = match &args.since {
        Some(token) => {
            let record = load_observation(&ctx.paths, &session.id, token)?;
            verify_boot_id(ctx, session, &record).await?;
            let diff = diff_observations(&record.observation(), &observation, args.full);
            Some(diff)
        }
        None => None,
    };
    let mut result = if let Some(diff) = since {
        serde_json::json!({"observation": observation, "changes": diff.changes})
    } else {
        serde_json::to_value(&observation)?
    };
    if args.full
        && let Some(object) = result.as_object_mut()
    {
        object.insert("full".to_string(), serde_json::Value::Bool(true));
    }
    if args.retries > 0
        && let Some(object) = result.as_object_mut()
    {
        object.insert(
            "retry".to_string(),
            serde_json::to_value(&retry_report).unwrap_or(serde_json::Value::Null),
        );
    }
    let retry_note = if retry_report.retried() {
        format!(" after {} retry attempt(s)", retry_report.retries_used)
    } else {
        String::new()
    };
    let human = format!(
        "Observed {} node(s) from {}{} ({}).{retry_note}\n",
        observation.hierarchy.node_count,
        observation.foreground.package.as_deref().unwrap_or("-"),
        observation
            .foreground
            .activity
            .as_deref()
            .map(|activity| format!(" {activity}"))
            .unwrap_or_default(),
        observation.id
    );
    Ok(success("ui.observe", result, human)
        .with_optional_session(Some(session.id.clone()))
        .with_device(device.serial.clone()))
}

pub async fn diff_command(
    ctx: &CliContext,
    session: &Session,
    device: &Device,
    args: &UiDiffArgs,
) -> Result<CommandSuccess> {
    if ctx.dry_run {
        return Ok(success(
            "ui.diff",
            serde_json::json!({
                "dry_run": true,
                "session_id": session.id,
                "device_serial": device.serial,
                "from": args.from,
                "to": args.to,
                "would": "compare the stored observation with the stored or freshly captured target",
                "would_capture": args.to.is_none(),
            }),
            format!(
                "Would compare observation {} without writing a new token.\n",
                args.from
            ),
        )
        .with_session(session.id.clone())
        .with_device(device.serial.clone()));
    }
    let before_record = load_observation(&ctx.paths, &session.id, &args.from)?;
    verify_boot_id(ctx, session, &before_record).await?;
    let before = before_record.observation();
    let mut retry_report = None;
    let after = match &args.to {
        Some(token) => {
            let record = load_observation(&ctx.paths, &session.id, token)?;
            verify_boot_id(ctx, session, &record).await?;
            record.observation()
        }
        None => {
            let (after, report) =
                common::retry_read_only(ReadOnlyOperation::UiDiffCapture, args.retries, || {
                    collect_observation(ctx, session, device, None, false)
                })
                .await?;
            retry_report = Some(report);
            after
        }
    };
    let diff = diff_observations(&before, &after, args.full);
    let human = format!(
        "Compared {} with {}: {} added, {} removed, {} modified{}.\\n",
        diff.from,
        diff.to,
        diff.changes.added_total,
        diff.changes.removed_total,
        diff.changes.modified_total,
        if diff.changes.truncated {
            " (lists truncated; see totals)"
        } else {
            ""
        }
    );
    let mut result = serde_json::to_value(&diff)?;
    if args.retries > 0
        && let Some(object) = result.as_object_mut()
    {
        if let Some(report) = retry_report {
            object.insert(
                "retry".to_string(),
                serde_json::to_value(&report).unwrap_or(serde_json::Value::Null),
            );
        } else {
            object.insert(
                "retry".to_string(),
                serde_json::json!({
                    "operation": "ui.diff_stored",
                    "requested_retries": args.retries,
                    "attempts": [],
                    "final_result": "not_applicable",
                    "message": "stored observation comparison is local and does not read the device"
                }),
            );
        }
    }
    Ok(success("ui.diff", result, human)
        .with_optional_session(Some(session.id.clone()))
        .with_device(device.serial.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_ancestor_path(ancestor: &str, path: &str) -> bool {
        path.len() > ancestor.len()
            && path.starts_with(ancestor)
            && path.as_bytes().get(ancestor.len()) == Some(&b'.')
    }

    fn bounds(left: i32, top: i32, right: i32, bottom: i32) -> ObservationBounds {
        ObservationBounds {
            left,
            top,
            right,
            bottom,
        }
    }

    fn node(path: &str, text: &str, rect: ObservationBounds) -> ObservationNode {
        ObservationNode {
            reference: None,
            actionable_via: None,
            role: Some("button".to_string()),
            path: path.to_string(),
            depth: 2,
            text: Some(text.to_string()),
            content_desc: None,
            hint: None,
            resource_id: Some("com.example:id/button".to_string()),
            class: Some("android.widget.Button".to_string()),
            package: Some("com.example".to_string()),
            bounds: rect,
            clickable: true,
            long_clickable: false,
            enabled: true,
            focused: false,
            selected: false,
            scrollable: false,
            checked: false,
            editable: false,
            password: false,
            displayed: Some(true),
        }
    }

    fn observation(id: &str, nodes: Vec<ObservationNode>) -> Observation {
        let actionable_total = nodes.len();
        Observation {
            schema_version: OBSERVATION_SCHEMA_VERSION,
            id: id.to_string(),
            token: id.to_string(),
            session_id: "ses-test".to_string(),
            device_serial: "emulator-5554".to_string(),
            observed_at: now_rfc3339(),
            age_ms: 0,
            stale: false,
            display: ObservationDisplay {
                width: Some(1080),
                height: Some(2400),
                orientation: "portrait".to_string(),
                rotation: Some(0),
            },
            foreground: ObservationForeground {
                package: Some("com.example".to_string()),
                activity: Some(".MainActivity".to_string()),
                window: None,
            },
            focused: None,
            actionable: nodes,
            actionable_total,
            actionable_truncated: false,
            context: Vec::new(),
            context_total: 0,
            context_truncated: false,
            scrollable: Vec::new(),
            visible_text: Vec::new(),
            nodes: None,
            hierarchy: ObservationHierarchyMeta {
                package: Some("com.example".to_string()),
                rotation: Some(0),
                node_count: 1,
                compact_node_count: 1,
            },
            raw_xml_path: "ui/observation.xml".to_string(),
        }
    }

    fn test_paths(root: &std::path::Path) -> ForgePaths {
        ForgePaths {
            cwd: root.to_path_buf(),
            project_root: root.to_path_buf(),
            project_config_path: None,
            global_config_path: root.join("global.toml"),
            global_dir: root.join("global"),
            state_dir: root.join("state"),
            artifacts_dir: root.join("artifacts"),
        }
    }

    #[test]
    fn diff_reports_semantic_changes_without_pixel_noise() {
        let before = observation("obs-1", vec![node("root/0", "Save", bounds(0, 0, 100, 50))]);
        let mut after_node = node("root/0", "Saved", bounds(1, 1, 101, 51));
        after_node.enabled = false;
        let after = observation("obs-2", vec![after_node]);
        let diff = diff_observations(&before, &after, false);
        assert_eq!(diff.changes.added_total, 0);
        assert_eq!(diff.changes.removed_total, 0);
        assert_eq!(diff.changes.modified_total, 1);
        let fields = &diff.changes.modified[0].changes;
        assert!(fields.iter().any(|change| change.field == "text"));
        assert!(fields.iter().any(|change| change.field == "enabled"));
        assert!(!fields.iter().any(|change| change.field == "bounds"));
    }

    #[test]
    fn diff_ignores_pure_layout_movement() {
        let before = observation(
            "obs-1",
            vec![node("root/0", "Settings", bounds(0, 0, 100, 50))],
        );
        let after = observation(
            "obs-2",
            vec![node("root/0", "Settings", bounds(1, 1, 101, 51))],
        );
        let diff = diff_observations(&before, &after, false);
        assert_eq!(diff.changes.modified_total, 0);
        assert_eq!(diff.changes.added_total, 0);
        assert_eq!(diff.changes.removed_total, 0);
    }

    #[test]
    fn diff_reports_meaningful_size_changes() {
        let before = observation(
            "obs-1",
            vec![node("root/0", "Settings", bounds(0, 0, 100, 50))],
        );
        let after = observation(
            "obs-2",
            vec![node("root/0", "Settings", bounds(0, 0, 300, 200))],
        );
        let diff = diff_observations(&before, &after, false);
        assert_eq!(diff.changes.modified_total, 1);
        assert!(
            diff.changes.modified[0]
                .changes
                .iter()
                .any(|change| change.field == "bounds")
        );
    }

    #[test]
    fn diff_pairs_duplicate_resource_ids_in_order() {
        let before = observation(
            "obs-1",
            vec![
                node("root/0", "One", bounds(0, 0, 100, 50)),
                node("root/1", "Two", bounds(0, 60, 100, 110)),
            ],
        );
        let after = observation(
            "obs-2",
            vec![
                node("root/0", "Uno", bounds(0, 0, 100, 50)),
                node("root/1", "Dos", bounds(0, 60, 100, 110)),
            ],
        );
        let diff = diff_observations(&before, &after, false);
        assert_eq!(diff.changes.added_total, 0);
        assert_eq!(diff.changes.removed_total, 0);
        assert_eq!(diff.changes.modified_total, 2);
    }

    #[test]
    fn diff_limits_large_changes_and_keeps_counts() {
        let before = observation("obs-1", Vec::new());
        let after = observation(
            "obs-2",
            (0..150)
                .map(|index| {
                    node(
                        &format!("root/{index}"),
                        &format!("Button {index}"),
                        bounds(0, index, 100, index + 40),
                    )
                })
                .collect(),
        );
        let compact = diff_observations(&before, &after, false);
        assert_eq!(compact.changes.added_total, 150);
        assert_eq!(compact.changes.added.len(), 12);
        assert!(compact.changes.truncated);
        let full = diff_observations(&before, &after, true);
        assert_eq!(full.changes.added.len(), 100);
        assert!(full.changes.truncated);
    }

    #[test]
    fn compact_nodes_separates_interactive_targets_from_context_text() {
        let xml = r#"<hierarchy rotation="0">
  <node index="0" text="Save" resource-id="com.example:id/save" class="android.widget.Button" package="com.example" content-desc="" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[100,200][300,300]" />
  <node index="1" text="Footer note" resource-id="" class="android.widget.TextView" package="com.example" content-desc="" clickable="false" enabled="true" focused="false" scrollable="false" bounds="[100,400][700,450]" />
</hierarchy>"#;
        let hierarchy = UiHierarchy::parse(xml).unwrap();
        let (actionable, context, _, _) = compact_nodes(&hierarchy);
        assert_eq!(actionable.len(), 1);
        assert_eq!(actionable[0].text.as_deref(), Some("Save"));
        assert!(actionable[0].clickable);
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].text.as_deref(), Some("Footer note"));
        assert!(!context[0].clickable);
        let value = serde_json::to_value(&context[0]).unwrap();
        assert_eq!(value["clickable"], false);
        assert_eq!(value["enabled"], true);
        assert!(value.get("resource_id").is_none());
    }

    #[test]
    fn password_selector_never_contains_the_secret() {
        let node = UiNode {
            id: 1,
            parent: None,
            children: Vec::new(),
            depth: 1,
            text: Some("hunter2".to_string()),
            content_desc: None,
            hint: None,
            resource_id: Some("com.example:id/password".to_string()),
            class: Some("android.widget.EditText".to_string()),
            package: Some("com.example".to_string()),
            bounds: Rect {
                left: 0,
                top: 0,
                right: 100,
                bottom: 50,
            },
            bounds_raw: "[0,0][100,50]".to_string(),
            clickable: true,
            long_clickable: false,
            enabled: true,
            focused: true,
            selected: false,
            scrollable: false,
            checked: false,
            password: true,
            displayed: Some(true),
            index_attr: None,
            path: "root/0".to_string(),
        };
        let compact = ObservationNode::from_node(&node);
        assert_eq!(compact.text.as_deref(), Some("[password]"));
        assert!(compact.password);
        let serialized = serde_json::to_string(&compact).unwrap();
        assert!(!serialized.contains("hunter2"));
    }

    #[test]
    fn observation_tokens_are_session_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let record = ObservationRecord {
            observation: observation("obs-1", Vec::new()),
            boot_id: Some("boot-1".to_string()),
            session_id: "ses-a".to_string(),
        };
        let mut record = record;
        record.observation.session_id = "ses-a".to_string();
        let path = observation_path(&paths, "ses-a", "obs-1");
        write_json_atomic(&path, &record).unwrap();
        let loaded = load_observation(&paths, "ses-a", "obs-1").expect("same session");
        assert_eq!(loaded.boot_id(), Some("boot-1"));
        let error = load_observation(&paths, "ses-b", "obs-1").unwrap_err();
        assert_eq!(error.code(), ErrorCode::ObservationSessionMismatch);
        let error = load_observation(&paths, "ses-a", "obs-missing").unwrap_err();
        assert_eq!(error.code(), ErrorCode::ObservationNotFound);
    }

    #[test]
    fn element_references_are_unique_and_scoped_to_the_observation_token() {
        let mut first = observation(
            "obs-ref-1",
            vec![
                node("root/0", "Save", bounds(0, 0, 100, 50)),
                node("root/1", "Save", bounds(0, 60, 100, 110)),
            ],
        );
        assign_references(&mut first);
        let refs = first
            .actionable
            .iter()
            .filter_map(|node| node.reference.clone())
            .collect::<Vec<_>>();
        assert_eq!(refs.len(), 2);
        assert_ne!(refs[0], refs[1]);
        assert!(refs.iter().all(|value| value.starts_with("el-")));
        assert!(refs.iter().all(|value| value.len() == 15));

        let mut second = first.clone();
        second.id = "obs-ref-2".to_string();
        second.token = "obs-ref-2".to_string();
        assign_references(&mut second);
        assert_eq!(
            first.actionable[0].reference, second.actionable[0].reference,
            "an unchanged node keeps the same ref across observations"
        );

        // A changed identity is a different target and must not inherit the
        // previous ref.
        let mut relabelled = first.clone();
        relabelled.id = "obs-ref-3".to_string();
        relabelled.token = "obs-ref-3".to_string();
        relabelled.actionable[0].text = Some("Delete".to_string());
        assign_references(&mut relabelled);
        assert_ne!(
            first.actionable[0].reference,
            relabelled.actionable[0].reference
        );
    }

    #[test]
    fn retention_evicts_oldest_and_never_the_token_just_written() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let session = "ses-retention";
        let base = chrono::Utc::now() - chrono::Duration::minutes(30);
        let mut tokens = Vec::new();
        for index in 0..DEFAULT_OBSERVATION_RETENTION {
            let token = format!("obs-retention-{index:02}");
            let mut record = ObservationRecord {
                observation: observation(&token, Vec::new()),
                boot_id: None,
                session_id: session.to_string(),
            };
            record.observation.session_id = session.to_string();
            record.observation.observed_at =
                (base + chrono::Duration::minutes(index as i64)).to_rfc3339();
            write_json_atomic(&observation_path(&paths, session, &token), &record).unwrap();
            tokens.push(token);
        }
        // Pre-write eviction leaves room for exactly one new observation.
        prune_observations(
            &paths,
            session,
            DEFAULT_OBSERVATION_RETENTION.saturating_sub(1),
            None,
        )
        .unwrap();
        let newest = "obs-newest";
        let mut record = ObservationRecord {
            observation: observation(newest, Vec::new()),
            boot_id: None,
            session_id: session.to_string(),
        };
        record.observation.session_id = session.to_string();
        record.observation.observed_at = chrono::Utc::now().to_rfc3339();
        write_json_atomic(&observation_path(&paths, session, newest), &record).unwrap();
        prune_observations(&paths, session, DEFAULT_OBSERVATION_RETENTION, Some(newest)).unwrap();

        assert!(observation_path(&paths, session, newest).is_file());
        assert!(!observation_path(&paths, session, &tokens[0]).is_file());
        assert!(observation_path(&paths, session, &tokens[1]).is_file());
        let remaining = std::fs::read_dir(observations_dir(&paths, session))
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
            .count();
        assert_eq!(remaining, DEFAULT_OBSERVATION_RETENTION);
    }

    #[test]
    fn context_text_inside_a_clickable_layout_is_marked_actionable_via_ancestor() {
        let hierarchy = UiHierarchy::parse(
            r#"<hierarchy rotation="0">
  <node index="0" text="" resource-id="com.example:id/key_row" class="android.widget.LinearLayout" package="com.example" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[0,0][100,100]">
    <node index="0" text="7" resource-id="" class="android.widget.TextView" package="com.example" clickable="false" enabled="true" focused="false" scrollable="false" bounds="[10,10][90,90]" />
  </node>
</hierarchy>"#,
        )
        .unwrap();
        let (actionable, context, _, _) = compact_nodes(&hierarchy);
        assert!(
            actionable.is_empty(),
            "the clickable layout is filtered as a layout container"
        );
        let key = context
            .iter()
            .find(|node| node.text.as_deref() == Some("7"))
            .expect("key label");
        assert_eq!(key.actionable_via.as_deref(), Some("ancestor"));
        assert!(is_ancestor_path("0", &key.path));
    }

    #[test]
    fn new_observation_tokens_are_short_session_scoped_and_collision_checked() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let first = allocate_token(&paths, "ses-a");
        let second = allocate_token(&paths, "ses-a");
        assert!(first.starts_with("obs-"));
        assert_eq!(first.len(), 16);
        assert_ne!(first, second);
        assert!(!observation_path(&paths, "ses-a", &first).exists());
        // Old longer tokens remain readable by the same path scheme.
        assert!(
            observation_path(&paths, "ses-a", "obs-20260101000000-abcdefabcdef")
                .to_string_lossy()
                .contains("obs-20260101000000-abcdefabcdef")
        );
    }

    #[test]
    fn full_observation_ref_keeps_context_ancestor_metadata() {
        let mut observation = observation("obs-full-context", Vec::new());
        let mut context = node("0.0", "7", bounds(10, 10, 90, 90));
        context.clickable = false;
        context.actionable_via = Some("ancestor".to_string());
        let mut full = context.clone();
        full.actionable_via = None;
        observation.context.push(context);
        observation.context_total = 1;
        observation.nodes = Some(vec![full]);
        assign_references(&mut observation);
        let reference = observation.context[0]
            .reference
            .clone()
            .expect("context reference");
        let found = find_reference(&observation, &reference).expect("found");
        assert_eq!(found.actionable_via.as_deref(), Some("ancestor"));
        assert_eq!(
            observation.nodes.as_ref().unwrap()[0]
                .actionable_via
                .as_deref(),
            Some("ancestor"),
            "full hierarchy copies must carry the same readiness metadata"
        );
    }

    #[test]
    fn references_can_be_found_and_mapped_to_an_expectation() {
        let mut observation = observation(
            "obs-ref-3",
            vec![node("root/0", "Save", bounds(0, 0, 100, 50))],
        );
        assign_references(&mut observation);
        let reference = observation.actionable[0]
            .reference
            .clone()
            .expect("reference");
        let found = find_reference(&observation, &reference).expect("found");
        assert_eq!(found.path, "root/0");
        assert_eq!(found.text.as_deref(), Some("Save"));
    }
}
