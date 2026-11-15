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
    }

    pub fn validate(&self) -> Result<()> {
        if !self.has_conditions() {
            return Err(ForgeError::invalid_argument(
                "a UI selector needs at least one condition (for example --text, --content-desc or --resource-id)",
            ));
        }
        Ok(())
    }

    pub fn matches(&self, node: &UiNode) -> bool {
        if let Some(text) = &self.text {
            if node.text.as_deref() != Some(text.as_str()) {
                return false;
            }
        }
        if let Some(text) = &self.text_contains {
            if !node
                .text
                .as_deref()
                .map(|value| value.contains(text))
                .unwrap_or(false)
            {
                return false;
            }
        }
        if let Some(desc) = &self.content_desc {
            if node.content_desc.as_deref() != Some(desc.as_str()) {
                return false;
            }
        }
        if let Some(desc) = &self.content_desc_contains {
            if !node
                .content_desc
                .as_deref()
                .map(|value| value.contains(desc))
                .unwrap_or(false)
            {
                return false;
            }
        }
        if let Some(id) = &self.resource_id {
            if node.resource_id.as_deref() != Some(id.as_str()) {
                return false;
            }
        }
        if let Some(class) = &self.class {
            if node.class.as_deref() != Some(class.as_str()) {
                return false;
            }
        }
        if let Some(clickable) = self.clickable {
            if node.clickable != clickable {
                return false;
            }
        }
        if let Some(enabled) = self.enabled {
            if node.enabled != enabled {
                return false;
            }
        }
        if let Some(focused) = self.focused {
            if node.focused != focused {
                return false;
            }
        }
        if let Some(scrollable) = self.scrollable {
            if node.scrollable != scrollable {
                return false;
            }
        }
        if let Some(checked) = self.checked {
            if node.checked != checked {
                return false;
            }
        }
        if let Some(visible) = self.visible {
            if node.visible() != visible {
                return false;
            }
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
    pub bounds: Rect,
    pub clickable: bool,
    pub enabled: bool,
    pub focused: bool,
    pub scrollable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_clickable: Option<bool>,
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
            bounds: node.bounds,
            clickable: node.clickable,
            enabled: node.enabled,
            focused: node.focused,
            scrollable: node.scrollable,
            parent_clickable: hierarchy
                .clickable_ancestor(node)
                .map(|ancestor| ancestor.clickable),
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
    let indices = hierarchy
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| selector.matches(node))
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
    let indices = hierarchy
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| selector.matches(node))
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
        );
        for item in &matches {
            error = error.with_suggestion(format!(
                "--index {}: {}",
                item.index,
                item.text
                    .as_deref()
                    .or(item.content_desc.as_deref())
                    .or(item.resource_id.as_deref())
                    .unwrap_or("(no label)")
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
    labels.truncate(40);
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
    .with_suggestion("Run `androidforge ui dump` and inspect the current hierarchy.")
    .with_suggestion("Use --contains for partial text matching if the label is dynamic.")
    .with_suggestion("Use `androidforge ui wait` if the screen may still be loading.")
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
    fn ambiguous_selector_is_an_error_with_candidates() {
        let hierarchy = UiHierarchy::parse(XML).unwrap();
        let selector = Selector {
            text: Some("Save".to_string()),
            ..Selector::default()
        };
        let error = select(hierarchy, &selector).unwrap_err();
        assert_eq!(error.code(), ErrorCode::SelectorAmbiguous);
        assert!(error.body().context.contains_key("matches"));
        assert!(
            error
                .body()
                .suggestions
                .iter()
                .any(|suggestion| suggestion.contains("--index"))
        );
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
}
