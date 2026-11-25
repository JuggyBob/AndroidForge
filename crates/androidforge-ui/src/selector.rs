use androidforge_core::error::{ErrorCode, ForgeError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hierarchy::{Rect, UiHierarchy, UiNode};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Selector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_desc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_desc_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clickable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrollable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_regex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub above: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct SelectorContext {
    text_regex: Option<regex::Regex>,
    region: Option<Rect>,
    within: Vec<(usize, Rect)>,
    above: Vec<(usize, Rect)>,
    below: Vec<(usize, Rect)>,
    left_of: Vec<(usize, Rect)>,
    right_of: Vec<(usize, Rect)>,
}

impl SelectorContext {
    fn build(hierarchy: &UiHierarchy, selector: &Selector) -> Result<Self> {
        let text_regex = selector
            .text_regex
            .as_deref()
            .map(regex::Regex::new)
            .transpose()
            .map_err(|error| {
                ForgeError::invalid_argument(format!("invalid --text-regex: {error}"))
            })?;
        let region = selector.region.as_deref().map(parse_region).transpose()?;
        Ok(Self {
            text_regex,
            region,
            within: reference_rects(hierarchy, selector.within.as_deref())?,
            above: reference_rects(hierarchy, selector.above.as_deref())?,
            below: reference_rects(hierarchy, selector.below.as_deref())?,
            left_of: reference_rects(hierarchy, selector.left_of.as_deref())?,
            right_of: reference_rects(hierarchy, selector.right_of.as_deref())?,
        })
    }
}

fn parse_region(value: &str) -> Result<Rect> {
    let numbers = value
        .split(',')
        .map(str::trim)
        .map(str::parse::<i32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| {
            ForgeError::invalid_argument(format!(
                "--region {value:?} must be four integers: x1,y1,x2,y2 ({error})"
            ))
        })?;
    if numbers.len() != 4 || numbers[2] <= numbers[0] || numbers[3] <= numbers[1] {
        return Err(ForgeError::invalid_argument(format!(
            "--region {value:?} must be x1,y1,x2,y2 with x2>x1 and y2>y1"
        )));
    }
    Ok(Rect {
        left: numbers[0],
        top: numbers[1],
        right: numbers[2],
        bottom: numbers[3],
    })
}

fn rect_contains(rect: Rect, point: (i32, i32)) -> bool {
    rect.contains(point.0, point.1)
}

fn label_matches(node: &UiNode, label: &str) -> bool {
    node.text.as_deref() == Some(label)
        || node.content_desc.as_deref() == Some(label)
        || node.resource_id.as_deref() == Some(label)
}

fn reference_rects(hierarchy: &UiHierarchy, label: Option<&str>) -> Result<Vec<(usize, Rect)>> {
    let Some(label) = label else {
        return Ok(Vec::new());
    };
    let mut matches = hierarchy
        .nodes
        .iter()
        .filter(|node| node.visible() && label_matches(node, label))
        .map(|node| (node.id, node.bounds))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        matches = hierarchy
            .nodes
            .iter()
            .filter(|node| {
                node.visible()
                    && (node
                        .text
                        .as_deref()
                        .is_some_and(|text| text.contains(label))
                        || node
                            .content_desc
                            .as_deref()
                            .is_some_and(|text| text.contains(label)))
            })
            .map(|node| (node.id, node.bounds))
            .collect();
    }
    if matches.is_empty() {
        return Err(ForgeError::new(
            ErrorCode::SelectorNotFound,
            format!("reference selector {label:?} did not match any visible node"),
        )
        .with_suggestion("Use --text, --content-desc or --resource-id for the reference node.")
        .with_suggestion("Run `androidforge ui find --contains <label>` to inspect candidates."));
    }
    Ok(matches)
}

impl Selector {
    pub fn has_conditions(&self) -> bool {
        self.text.is_some()
            || self.text_contains.is_some()
            || self.content_desc.is_some()
            || self.content_desc_contains.is_some()
            || self.resource_id.is_some()
            || self.class.is_some()
            || self.clickable.is_some()
            || self.enabled.is_some()
            || self.focused.is_some()
            || self.scrollable.is_some()
            || self.checked.is_some()
            || self.visible.is_some()
            || self.role.is_some()
            || self.text_regex.is_some()
            || self.within.is_some()
            || self.above.is_some()
            || self.below.is_some()
            || self.left_of.is_some()
            || self.right_of.is_some()
            || self.region.is_some()
    }

    pub fn validate(&self) -> Result<()> {
        if !self.has_conditions() {
            return Err(ForgeError::invalid_argument(
                "a UI selector needs at least one condition (for example --text, --content-desc, --resource-id, --role or --text-regex)",
            ));
        }
        if let Some(pattern) = &self.text_regex {
            regex::Regex::new(pattern).map_err(|error| {
                ForgeError::invalid_argument(format!(
                    "--text-regex {pattern:?} is invalid: {error}"
                ))
            })?;
        }
        if let Some(region) = &self.region {
            parse_region(region)?;
        }
        Ok(())
    }

    pub(crate) fn matches(
        &self,
        hierarchy: &UiHierarchy,
        node: &UiNode,
        context: &SelectorContext,
    ) -> bool {
        if let Some(text) = &self.text
            && node.text.as_deref() != Some(text.as_str())
        {
            return false;
        }
        if let Some(text) = &self.text_contains
            && !node
                .text
                .as_deref()
                .map(|value| value.contains(text))
                .unwrap_or(false)
        {
            return false;
        }
        if let Some(desc) = &self.content_desc
            && node.content_desc.as_deref() != Some(desc.as_str())
        {
            return false;
        }
        if let Some(desc) = &self.content_desc_contains
            && !node
                .content_desc
                .as_deref()
                .map(|value| value.contains(desc))
                .unwrap_or(false)
        {
            return false;
        }
        if let Some(id) = &self.resource_id
            && node.resource_id.as_deref() != Some(id.as_str())
        {
            return false;
        }
        if let Some(class) = &self.class
            && node.class.as_deref() != Some(class.as_str())
        {
            return false;
        }
        if let Some(role) = &self.role {
            let requested = role.to_ascii_lowercase();
            let requested = match requested.as_str() {
                "text_input" | "textfield" | "edit" => "edit_text",
                "checkbox" | "switch" | "toggle_button" => "toggle",
                "image_view" => "image",
                other => other,
            };
            if node.semantic_role() != requested {
                return false;
            }
        }
        if let Some(regex) = &context.text_regex {
            let matched = node
                .text
                .as_deref()
                .or(node.content_desc.as_deref())
                .is_some_and(|text| regex.is_match(text));
            if !matched {
                return false;
            }
        }
        if let Some(clickable) = self.clickable {
            // A clickable ancestor makes a label actionable for tap purposes.
            // This is the same rule the UI manager applies when dispatching, so
            // `--clickable` must not reject an element it would otherwise tap.
            let is_clickable =
                node.clickable || (clickable && hierarchy.clickable_ancestor(node).is_some());
            if is_clickable != clickable {
                return false;
            }
        }
        if let Some(enabled) = self.enabled
            && node.enabled != enabled
        {
            return false;
        }
        if let Some(focused) = self.focused
            && node.focused != focused
        {
            return false;
        }
        if let Some(scrollable) = self.scrollable
            && node.scrollable != scrollable
        {
            return false;
        }
        if let Some(checked) = self.checked
            && node.checked != checked
        {
            return false;
        }
        if let Some(visible) = self.visible
            && node.visible() != visible
        {
            return false;
        }
        if let Some(region) = context.region
            && !rect_contains(region, node.bounds.center())
        {
            return false;
        }
        if !context.within.is_empty()
            && !context
                .within
                .iter()
                .any(|(id, rect)| *id != node.id && rect_contains(*rect, node.bounds.center()))
        {
            return false;
        }
        if !context.above.is_empty()
            && !context
                .above
                .iter()
                .any(|(id, rect)| *id != node.id && node.bounds.bottom <= rect.top)
        {
            return false;
        }
        if !context.below.is_empty()
            && !context
                .below
                .iter()
                .any(|(id, rect)| *id != node.id && node.bounds.top >= rect.bottom)
        {
            return false;
        }
        if !context.left_of.is_empty()
            && !context
                .left_of
                .iter()
                .any(|(id, rect)| *id != node.id && node.bounds.right <= rect.left)
        {
            return false;
        }
        if !context.right_of.is_empty()
            && !context
                .right_of
                .iter()
                .any(|(id, rect)| *id != node.id && node.bounds.left >= rect.right)
        {
            return false;
        }
        true
    }

    pub fn describe(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiMatch {
    /// Zero-based position in the selector's match list (the value accepted by --index).
    pub index: usize,
    /// Internal node id in the parsed hierarchy, useful for diagnostics.
    pub node_id: usize,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_desc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Stable semantic role, using the same vocabulary as observation nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Observation-style element reference (`el-…`) derived from node identity.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "ref")]
    pub reference: Option<String>,
    pub bounds: Rect,
    pub clickable: bool,
    pub enabled: bool,
    pub focused: bool,
    pub scrollable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_clickable: Option<bool>,
    #[serde(default = "default_match_source")]
    pub source: String,
}

fn default_match_source() -> String {
    "uiautomator".to_string()
}

/// Deterministic observation-style element reference for a hierarchy node.
///
/// This is the canonical v2 reference material shared with the CLI observation
/// store (`el-<12 hex chars>`), so a selector candidate can be correlated with
/// a `ui observe --json` ref without loading an observation. Password text is
/// masked exactly as it is in an observation. The returned value is a
/// diagnostics hint only: a ref is valid together with the observation that
/// produced it, never as a standalone handle.
pub fn observation_ref(node: &UiNode) -> String {
    observation_ref_with_length(node, 12)
}

/// The same canonical v2 material as [`observation_ref`], truncated to
/// `length` hex characters.
///
/// The CLI observation store lengthens refs when two distinct nodes share a
/// short hash prefix. Keeping the length parameter here means the ref formula
/// has exactly one definition, shared by selectors and observations.
pub fn observation_ref_with_length(node: &UiNode, length: usize) -> String {
    let text = if node.password && node.text.is_some() {
        "[password]"
    } else {
        node.text.as_deref().unwrap_or("")
    };
    let bounds = format!(
        "{},{},{},{}",
        node.bounds.left, node.bounds.top, node.bounds.right, node.bounds.bottom
    );
    let material = format!(
        "v2|{}|{}|{}|{}|{}|{}|{bounds}|{}",
        node.path,
        node.class.as_deref().unwrap_or(""),
        node.package.as_deref().unwrap_or(""),
        node.resource_id.as_deref().unwrap_or(""),
        text,
        node.content_desc.as_deref().unwrap_or(""),
        node.semantic_role(),
    );
    let hash = androidforge_core::util::sha256_hex(material.as_bytes());
    format!("el-{}", &hash[..length.min(hash.len())])
}

impl UiMatch {
    pub fn short_label(&self) -> String {
        format!(
            "{} {} [{},{}][{},{}]",
            self.text
                .as_deref()
                .or(self.content_desc.as_deref())
                .or(self.resource_id.as_deref())
                .unwrap_or("(no label)"),
            self.class.as_deref().unwrap_or(""),
            self.bounds.left,
            self.bounds.top,
            self.bounds.right,
            self.bounds.bottom
        )
    }

    pub fn from_node(hierarchy: &UiHierarchy, index: usize, node: &UiNode) -> Self {
        Self {
            index,
            node_id: node.id,
            path: node.path.clone(),
            text: node.text.clone(),
            content_desc: node.content_desc.clone(),
            resource_id: node.resource_id.clone(),
            class: node.class.clone(),
            package: node.package.clone(),
            role: Some(node.semantic_role().to_string()),
            reference: Some(observation_ref(node)),
            bounds: node.bounds,
            clickable: node.clickable,
            enabled: node.enabled,
            focused: node.focused,
            scrollable: node.scrollable,
            parent_clickable: hierarchy
                .clickable_ancestor(node)
                .map(|ancestor| ancestor.clickable),
            source: default_match_source(),
        }
    }

    pub fn ocr(index: usize, text: String, bounds: Rect) -> Self {
        Self {
            index,
            node_id: 0,
            path: format!("ocr:{index}"),
            text: Some(text),
            content_desc: None,
            resource_id: None,
            class: None,
            package: None,
            role: None,
            reference: None,
            bounds,
            clickable: true,
            enabled: true,
            focused: false,
            scrollable: false,
            parent_clickable: None,
            source: "ocr".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Selection {
    pub hierarchy: UiHierarchy,
    pub indices: Vec<usize>,
    pub selected: usize,
}

impl Selection {
    pub fn selected_node(&self) -> &UiNode {
        &self.hierarchy.nodes[self.selected]
    }

    pub fn selected_match(&self) -> UiMatch {
        let match_index = self
            .indices
            .iter()
            .position(|index| *index == self.selected)
            .unwrap_or(0);
        UiMatch::from_node(
            &self.hierarchy,
            match_index,
            &self.hierarchy.nodes[self.selected],
        )
    }

    pub fn matches(&self) -> Vec<UiMatch> {
        self.indices
            .iter()
            .enumerate()
            .map(|(match_index, index)| {
                UiMatch::from_node(&self.hierarchy, match_index, &self.hierarchy.nodes[*index])
            })
            .collect()
    }
}

pub fn find(hierarchy: &UiHierarchy, selector: &Selector) -> Result<Vec<UiMatch>> {
    selector.validate()?;
    let context = SelectorContext::build(hierarchy, selector)?;
    let indices = hierarchy
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| selector.matches(hierarchy, node, &context))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    Ok(indices
        .iter()
        .enumerate()
        .map(|(match_index, index)| {
            UiMatch::from_node(hierarchy, match_index, &hierarchy.nodes[*index])
        })
        .collect())
}

pub fn select(hierarchy: UiHierarchy, selector: &Selector) -> Result<Selection> {
    selector.validate()?;
    let context = SelectorContext::build(&hierarchy, selector)?;
    let indices = hierarchy
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| selector.matches(&hierarchy, node, &context))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if indices.is_empty() {
        return Err(selector_not_found(&hierarchy, selector));
    }
    if let Some(requested) = selector.index {
        if requested >= indices.len() {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                format!(
                    "selector matched {} element(s), but index {} was requested",
                    indices.len(),
                    requested
                ),
            )
            .with_context("selector", selector.describe())
            .with_context(
                "matches",
                serde_json::to_value(
                    indices
                        .iter()
                        .enumerate()
                        .map(|(match_index, index)| {
                            UiMatch::from_node(&hierarchy, match_index, &hierarchy.nodes[*index])
                        })
                        .collect::<Vec<_>>(),
                )
                .unwrap_or(Value::Null),
            )
            .with_suggestion(format!(
                "Use --index 0 through --index {}.",
                indices.len().saturating_sub(1)
            )));
        }
        return Ok(Selection {
            selected: indices[requested],
            indices,
            hierarchy,
        });
    }
    if indices.len() > 1 {
        let matches = indices
            .iter()
            .enumerate()
            .map(|(match_index, index)| {
                UiMatch::from_node(&hierarchy, match_index, &hierarchy.nodes[*index])
            })
            .collect::<Vec<_>>();
        let mut error = ForgeError::new(
            ErrorCode::SelectorAmbiguous,
            format!(
                "selector matched {} UI elements; refusing to choose one arbitrarily",
                indices.len()
            ),
        )
        .with_context("selector", selector.describe())
        .with_context(
            "matches",
            serde_json::to_value(&matches).unwrap_or(Value::Null),
        )
        .with_suggestion(
            "Add a more specific condition, or pass --index N to choose one of the listed matches.",
        )
        .with_suggestion(
            "Narrow by semantic role with --role (for example --role button), or add --resource-id or --content-desc.",
        );
        if matches.iter().any(|item| item.reference.is_some()) {
            error = error.with_suggestion(
                "Run `androidforge ui observe --session <ID> --json`, then pass --ref <ref> to target the exact observed element.",
            );
        }
        if matches.iter().any(|item| item.clickable) && matches.iter().any(|item| !item.clickable) {
            error = error.with_suggestion(
                "If only one candidate is interactive, add --clickable to exclude non-clickable text.",
            );
        }
        for item in &matches {
            let mut details = Vec::new();
            if let Some(role) = &item.role {
                details.push(format!("role={role}"));
            }
            if let Some(reference) = &item.reference {
                details.push(format!("ref={reference}"));
            }
            let suffix = if details.is_empty() {
                String::new()
            } else {
                format!(" [{}]", details.join(", "))
            };
            error = error.with_suggestion(format!(
                "--index {}: {}{}",
                item.index,
                item.text
                    .as_deref()
                    .or(item.content_desc.as_deref())
                    .or(item.resource_id.as_deref())
                    .unwrap_or("(no label)"),
                suffix
            ));
        }
        return Err(error);
    }
    Ok(Selection {
        selected: indices[0],
        indices,
        hierarchy,
    })
}

pub fn selector_not_found(hierarchy: &UiHierarchy, selector: &Selector) -> ForgeError {
    let focused = hierarchy
        .focused()
        .into_iter()
        .map(|node| {
            serde_json::json!({
                "path": node.path,
                "text": node.text,
                "content_desc": node.content_desc,
                "resource_id": node.resource_id,
                "class": node.class,
                "bounds": node.bounds,
                "password": node.password,
            })
        })
        .collect::<Vec<_>>();
    let mut labels = hierarchy
        .nodes
        .iter()
        .filter(|node| node.visible())
        .filter_map(|node| {
            node.text
                .as_ref()
                .or(node.content_desc.as_ref())
                .or(node.resource_id.as_ref())
                .cloned()
        })
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();
    // Put the requested label (or labels containing it) first so a bounded
    // diagnostic list cannot hide the very label the caller asked about.
    let requested = selector
        .text
        .as_deref()
        .or(selector.content_desc.as_deref())
        .or(selector.resource_id.as_deref())
        .or(selector.text_contains.as_deref())
        .or(selector.content_desc_contains.as_deref())
        .filter(|value| !value.is_empty());
    if let Some(requested) = requested {
        labels.sort_by_key(|label| {
            if label == requested {
                0
            } else if label.contains(requested) {
                1
            } else {
                2
            }
        });
    }
    let label_total = labels.len();
    let label_limit = 40;
    labels.truncate(label_limit);
    let labels_truncated = label_total > label_limit;
    ForgeError::new(
        ErrorCode::SelectorNotFound,
        "no UI element matched the requested selector",
    )
    .with_context("selector", selector.describe())
    .with_context(
        "focused_nodes",
        serde_json::to_value(focused).unwrap_or(Value::Null),
    )
    .with_context(
        "visible_labels",
        serde_json::to_value(labels).unwrap_or(Value::Null),
    )
    .with_context("visible_labels_total", label_total)
    .with_context("visible_labels_truncated", labels_truncated)
    .with_suggestion("Run `androidforge ui dump` and inspect the current hierarchy.")
    .with_suggestion("Use --contains for partial text matching if the label is dynamic.")
    .with_suggestion("Use `androidforge ui wait` if the screen may still be loading.")
    .with_suggestion(if let Some(text) = &selector.text {
        format!(
            "If the target is below the fold, run `androidforge ui scroll-to --text \"{text}\" --json`."
        )
    } else if let Some(desc) = &selector.content_desc {
        format!(
            "If the target is below the fold, run `androidforge ui scroll-to --content-desc \"{desc}\" --json`."
        )
    } else {
        "If the target is below the fold, use `androidforge ui scroll-to` with the same selector.".to_string()
    })
    .with_suggestion(if selector.resource_id.is_some() {
        "Compose testTag values are visible only when the debug app sets testTagsAsResourceId = true in its semantics.".to_string()
    } else {
        "For Compose screens, prefer visible text/content descriptions or explicit bounds when semantic tags are not exposed.".to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hierarchy::UiHierarchy;

    const XML: &str = r#"<hierarchy rotation="0">
  <node index="0" text="Save" resource-id="com.example:id/save" class="android.widget.Button" package="com.example" content-desc="Save changes" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[100,200][300,300]" />
  <node index="1" text="Save" resource-id="com.example:id/save2" class="android.widget.Button" package="com.example" content-desc="Save draft" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[100,400][300,500]" />
  <node index="2" text="" resource-id="com.example:id/name" class="android.widget.EditText" package="com.example" content-desc="Name" clickable="true" enabled="true" focused="true" scrollable="false" bounds="[100,600][900,700]" />
</hierarchy>"#;

    #[test]
    fn exact_and_contains_matching() {
        let hierarchy = UiHierarchy::parse(XML).unwrap();
        let selector = Selector {
            text: Some("Save".to_string()),
            ..Selector::default()
        };
        assert_eq!(find(&hierarchy, &selector).unwrap().len(), 2);
        let selector = Selector {
            text_contains: Some("Sav".to_string()),
            ..Selector::default()
        };
        assert_eq!(find(&hierarchy, &selector).unwrap().len(), 2);
        let selector = Selector {
            content_desc: Some("Save draft".to_string()),
            ..Selector::default()
        };
        assert_eq!(find(&hierarchy, &selector).unwrap().len(), 1);
    }

    #[test]
    fn regex_role_region_and_relative_selectors_match() {
        let hierarchy = UiHierarchy::parse(XML).unwrap();
        let regex = Selector {
            text_regex: Some("^Save$".to_string()),
            ..Selector::default()
        };
        assert_eq!(find(&hierarchy, &regex).unwrap().len(), 2);

        let role = Selector {
            role: Some("button".to_string()),
            ..Selector::default()
        };
        assert_eq!(find(&hierarchy, &role).unwrap().len(), 2);

        let region = Selector {
            text: Some("Save".to_string()),
            region: Some("150,240,250,260".to_string()),
            ..Selector::default()
        };
        assert_eq!(find(&hierarchy, &region).unwrap().len(), 1);

        let above = Selector {
            resource_id: Some("com.example:id/save".to_string()),
            above: Some("Save draft".to_string()),
            ..Selector::default()
        };
        assert_eq!(find(&hierarchy, &above).unwrap().len(), 1);

        let below = Selector {
            resource_id: Some("com.example:id/save2".to_string()),
            below: Some("Save".to_string()),
            ..Selector::default()
        };
        assert_eq!(find(&hierarchy, &below).unwrap().len(), 1);
    }

    #[test]
    fn within_and_left_right_selectors_use_reference_bounds() {
        let hierarchy = UiHierarchy::parse(
            r#"<hierarchy rotation="0">
  <node index="0" text="Form" resource-id="com.example:id/form" class="android.widget.FrameLayout" package="com.example" clickable="false" enabled="true" focused="false" scrollable="false" bounds="[0,0][1000,800]">
    <node index="0" text="Left" resource-id="com.example:id/left" class="android.widget.Button" package="com.example" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[10,10][200,100]" />
    <node index="1" text="Right" resource-id="com.example:id/right" class="android.widget.Button" package="com.example" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[800,10][990,100]" />
  </node>
  <node index="1" text="Outside" resource-id="com.example:id/outside" class="android.widget.Button" package="com.example" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[10,900][200,1000]" />
</hierarchy>"#,
        )
        .unwrap();
        let within = Selector {
            within: Some("Form".to_string()),
            ..Selector::default()
        };
        assert_eq!(find(&hierarchy, &within).unwrap().len(), 2);

        let left_of = Selector {
            text: Some("Left".to_string()),
            left_of: Some("Right".to_string()),
            ..Selector::default()
        };
        assert_eq!(find(&hierarchy, &left_of).unwrap().len(), 1);

        let right_of = Selector {
            text: Some("Right".to_string()),
            right_of: Some("Left".to_string()),
            ..Selector::default()
        };
        assert_eq!(find(&hierarchy, &right_of).unwrap().len(), 1);
    }

    #[test]
    fn find_matches_expose_role_and_observation_ref() {
        let hierarchy = UiHierarchy::parse(XML).unwrap();
        let selector = Selector {
            text: Some("Save".to_string()),
            ..Selector::default()
        };
        let matches = find(&hierarchy, &selector).unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].role.as_deref(), Some("button"));
        // Locked to the v2 material used by the CLI observation store.
        assert_eq!(matches[0].reference.as_deref(), Some("el-b94dd4c28015"));
        assert!(
            matches[1]
                .reference
                .as_deref()
                .is_some_and(|reference| reference.starts_with("el-"))
        );
        assert_ne!(matches[0].reference, matches[1].reference);

        let value = serde_json::to_value(&matches).unwrap();
        assert_eq!(value[0]["role"], "button");
        assert_eq!(value[0]["ref"], "el-b94dd4c28015");
        assert!(value[1]["ref"].as_str().is_some());

        // Payloads written before role/ref existed still deserialize.
        let legacy = serde_json::json!({
            "index": 0,
            "node_id": 0,
            "path": "0",
            "bounds": {"left": 0, "top": 0, "right": 10, "bottom": 10},
            "clickable": true,
            "enabled": true,
            "focused": false,
            "scrollable": false
        });
        let legacy: UiMatch = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.role, None);
        assert_eq!(legacy.reference, None);
        assert_eq!(legacy.source, "uiautomator");

        // OCR matches have no hierarchy node, so both keys are omitted.
        let ocr = UiMatch::ocr(0, "Save".to_string(), Rect::default());
        let value = serde_json::to_value(&ocr).unwrap();
        assert!(value.get("role").is_none());
        assert!(value.get("ref").is_none());
    }

    #[test]
    fn observation_ref_masks_password_text_like_observations() {
        let hierarchy = UiHierarchy::parse(
            r#"<hierarchy rotation="0">
  <node index="0" text="secret" resource-id="com.example:id/password" class="android.widget.EditText" package="com.example" clickable="true" enabled="true" focused="false" scrollable="false" password="true" bounds="[0,0][100,50]" />
</hierarchy>"#,
        )
        .unwrap();
        let node = &hierarchy.nodes[0];
        let reference = observation_ref(node);
        let mut masked = node.clone();
        masked.text = Some("[password]".to_string());
        masked.password = false;
        assert_eq!(reference, observation_ref(&masked));
    }

    #[test]
    fn ambiguous_selector_is_an_error_with_candidates() {
        let hierarchy = UiHierarchy::parse(XML).unwrap();
        let selector = Selector {
            text: Some("Save".to_string()),
            ..Selector::default()
        };
        let error = select(hierarchy, &selector).unwrap_err();
        assert_eq!(error.code(), ErrorCode::SelectorAmbiguous);
        let body = error.body();
        let matches = body
            .context
            .get("matches")
            .and_then(serde_json::Value::as_array)
            .unwrap();
        assert_eq!(matches.len(), 2);
        for item in matches {
            assert_eq!(item["role"], "button");
            assert!(
                item["ref"]
                    .as_str()
                    .is_some_and(|reference| reference.starts_with("el-"))
            );
        }
        let suggestions = &body.suggestions;
        assert!(suggestions.iter().any(|text| text.contains("--index")));
        assert!(suggestions.iter().any(|text| text.contains("--role")));
        assert!(suggestions.iter().any(|text| text.contains("--ref")));
        assert!(suggestions.iter().any(|text| text.contains("ref=el-")));
    }

    #[test]
    fn index_selects_match() {
        let hierarchy = UiHierarchy::parse(XML).unwrap();
        let selector = Selector {
            text: Some("Save".to_string()),
            index: Some(1),
            ..Selector::default()
        };
        let selection = select(hierarchy, &selector).unwrap();
        assert_eq!(
            selection.selected_node().resource_id.as_deref(),
            Some("com.example:id/save2")
        );
    }

    #[test]
    fn missing_selector_returns_context() {
        let hierarchy = UiHierarchy::parse(XML).unwrap();
        let selector = Selector {
            text: Some("Nope".to_string()),
            ..Selector::default()
        };
        let error = select(hierarchy, &selector).unwrap_err();
        assert_eq!(error.code(), ErrorCode::SelectorNotFound);
        assert!(error.body().context.contains_key("focused_nodes"));
    }

    #[test]
    fn clickable_flag_accepts_a_clickable_ancestor() {
        let hierarchy = UiHierarchy::parse(
            r#"<hierarchy rotation="0">
  <node index="0" text="" resource-id="com.example:id/nav_settings" class="android.view.View" package="com.example" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[0,0][500,200]">
    <node index="0" text="Settings" resource-id="" class="android.widget.TextView" package="com.example" clickable="false" enabled="true" focused="false" scrollable="false" bounds="[10,10][200,100]" />
  </node>
</hierarchy>"#,
        )
        .unwrap();
        let selector = Selector {
            text: Some("Settings".to_string()),
            clickable: Some(true),
            ..Selector::default()
        };
        let matches = find(&hierarchy, &selector).unwrap();
        assert_eq!(matches.len(), 1);
        assert!(!matches[0].clickable);
        assert_eq!(matches[0].parent_clickable, Some(true));
        assert!(select(hierarchy, &selector).is_ok());
    }

    #[test]
    fn missing_selector_prioritizes_the_requested_label_and_reports_truncation() {
        let mut xml = String::from("<hierarchy rotation=\"0\">\n");
        for index in 0..50 {
            xml.push_str(&format!(
                "<node index=\"{index}\" text=\"Item {index:02}\" resource-id=\"\" class=\"android.widget.TextView\" package=\"com.example\" clickable=\"false\" enabled=\"true\" focused=\"false\" scrollable=\"false\" bounds=\"[0,{index}][100,{}]\" />\n",
                index + 1
            ));
        }
        xml.push_str("<node index=\"50\" text=\"Settings\" resource-id=\"\" class=\"android.widget.TextView\" package=\"com.example\" clickable=\"false\" enabled=\"true\" focused=\"false\" scrollable=\"false\" bounds=\"[0,60][100,70]\" />\n</hierarchy>");
        let hierarchy = UiHierarchy::parse(&xml).unwrap();
        let selector = Selector {
            text: Some("Settings".to_string()),
            ..Selector::default()
        };
        let error = selector_not_found(&hierarchy, &selector);
        assert_eq!(error.code(), ErrorCode::SelectorNotFound);
        let labels = error.body().context["visible_labels"].as_array().unwrap();
        assert_eq!(labels[0], "Settings");
        assert_eq!(labels.len(), 40);
        assert_eq!(error.body().context["visible_labels_total"], 51);
        assert_eq!(error.body().context["visible_labels_truncated"], true);
    }
}
