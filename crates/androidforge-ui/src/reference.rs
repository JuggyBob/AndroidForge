//! Stable element-reference validation.
//!
//! A reference is only a handle into one observation. Before an action the CLI
//! re-reads the hierarchy and proves that the same node still exists with the
//! recorded identity. If the node moved, changed, disappeared or is disabled,
//! the action is refused. There is never a text/coordinate fallback.

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use serde::{Deserialize, Serialize};

use crate::hierarchy::{Rect, UiHierarchy, UiNode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeExpectation {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_desc: Option<String>,
    pub bounds: Rect,
    pub clickable: bool,
    pub long_clickable: bool,
    pub enabled: bool,
    pub scrollable: bool,
    pub editable: bool,
    #[serde(default)]
    pub password: bool,
    /// True when the node belonged to an observation's actionable sequence.
    #[serde(default)]
    pub actionable_hint: bool,
}

impl NodeExpectation {
    pub fn from_node(node: &UiNode) -> Self {
        Self {
            path: node.path.clone(),
            class: node.class.clone(),
            package: node.package.clone(),
            resource_id: node.resource_id.clone(),
            text: node.text.clone(),
            content_desc: node.content_desc.clone(),
            bounds: node.bounds,
            clickable: node.clickable,
            long_clickable: node.long_clickable,
            enabled: node.enabled,
            scrollable: node.scrollable,
            editable: node.editable_like(),
            password: node.password,
            actionable_hint: node.clickable
                || node.long_clickable
                || node.editable_like()
                || node.scrollable
                || (node.content_desc.is_some()
                    && !node
                        .class
                        .as_deref()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .contains("text")),
        }
    }

    /// Find the node and require that it is safe to act on.
    pub fn find<'a>(&self, hierarchy: &'a UiHierarchy) -> Result<&'a UiNode> {
        self.find_with(hierarchy, ExpectationMode::Action)
    }

    /// Find the node and prove its identity, without requiring actionability.
    /// This is used by wait/assert, which may reference context text.
    pub fn find_presence<'a>(&self, hierarchy: &'a UiHierarchy) -> Result<&'a UiNode> {
        self.find_with(hierarchy, ExpectationMode::Presence)
    }

    fn find_with<'a>(
        &self,
        hierarchy: &'a UiHierarchy,
        mode: ExpectationMode,
    ) -> Result<&'a UiNode> {
        let Some(node) = hierarchy
            .nodes
            .iter()
            .find(|candidate| candidate.path == self.path)
        else {
            return Err(ForgeError::new(
                ErrorCode::ElementRefStale,
                format!(
                    "element reference target path '{}' no longer exists",
                    self.path
                ),
            )
            .with_suggestion(
                "Run `androidforge ui observe --session <ID> --json` and choose a new ref.",
            )
            .with_suggestion(
                "A stale ref is never retried with text, resource-id or coordinates.",
            ));
        };
        self.validate(node, hierarchy, mode)?;
        Ok(node)
    }

    fn validate(
        &self,
        node: &UiNode,
        hierarchy: &UiHierarchy,
        mode: ExpectationMode,
    ) -> Result<()> {
        if !self.matches_identity(node) {
            return Err(ForgeError::new(
                ErrorCode::ElementIdentityChanged,
                "the element at this path no longer matches the observed identity",
            )
            .with_context_string("path", self.path.clone())
            .with_context(
                "observed",
                serde_json::to_value(self).unwrap_or(serde_json::Value::Null),
            )
            .with_context(
                "current",
                serde_json::to_value(crate::selector::UiMatch::from_node(hierarchy, 0, node))
                    .unwrap_or(serde_json::Value::Null),
            )
            .with_suggestion(
                "Run `androidforge ui observe --session <ID> --json` and choose a new ref.",
            )
            .with_suggestion(
                "AndroidForge never falls back to text or coordinates for a changed node.",
            ));
        }
        if mode == ExpectationMode::Presence {
            return Ok(());
        }
        if !node.enabled {
            return Err(ForgeError::new(
                ErrorCode::ElementNotActionable,
                "the referenced element is disabled",
            )
            .with_context_string("path", self.path.clone())
            .with_suggestion("Wait for the element to become enabled, then observe again."));
        }
        if !node.visible() {
            return Err(ForgeError::new(
                ErrorCode::ElementNotActionable,
                "the referenced element is not visible",
            )
            .with_context_string("path", self.path.clone())
            .with_suggestion("Scroll the element into view, then observe again."));
        }
        if !(self.actionable_hint
            || node.clickable
            || node.long_clickable
            || node.editable_like()
            || node.scrollable
            || hierarchy.clickable_ancestor(node).is_some())
        {
            return Err(ForgeError::new(
                ErrorCode::ElementNotActionable,
                "the referenced element has no actionable semantics",
            )
            .with_context_string("path", self.path.clone())
            .with_suggestion("Choose an element from the observation's `actionable` list."));
        }
        Ok(())
    }

    /// Full identity except the fields that legitimately change after focus or
    /// input. Used to prove that a tap focused the referenced field before any
    /// text is sent.
    pub fn matches_core(&self, node: &UiNode) -> bool {
        node.class == self.class
            && node.package == self.package
            && node.resource_id == self.resource_id
            && (self.password || node.text == self.text)
            && node.content_desc == self.content_desc
            && node.editable_like() == self.editable
    }

    /// Complete identity recorded by the observation, including bounds and
    /// actionability flags.
    pub fn matches_identity(&self, node: &UiNode) -> bool {
        self.matches_core(node)
            && node.bounds == self.bounds
            && node.clickable == self.clickable
            && node.long_clickable == self.long_clickable
            && node.enabled == self.enabled
            && node.scrollable == self.scrollable
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectationMode {
    /// The node must exist with the recorded identity and be actionable.
    Action,
    /// The node must exist with the recorded identity; actionability is not
    /// required. Used by wait and assert.
    Presence,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(path: &str) -> UiNode {
        UiNode {
            id: 1,
            parent: None,
            children: Vec::new(),
            depth: 1,
            text: Some("Save".to_string()),
            content_desc: None,
            hint: None,
            resource_id: Some("com.example:id/save".to_string()),
            class: Some("android.widget.Button".to_string()),
            package: Some("com.example".to_string()),
            bounds: Rect {
                left: 10,
                top: 10,
                right: 110,
                bottom: 60,
            },
            bounds_raw: "[10,10][110,60]".to_string(),
            clickable: true,
            long_clickable: false,
            enabled: true,
            focused: false,
            selected: false,
            scrollable: false,
            checked: false,
            password: false,
            displayed: Some(true),
            index_attr: None,
            path: path.to_string(),
        }
    }

    fn hierarchy(nodes: Vec<UiNode>) -> UiHierarchy {
        UiHierarchy {
            xml: "<hierarchy/>".to_string(),
            rotation: Some(0),
            root: None,
            package_name: Some("com.example".to_string()),
            nodes,
        }
    }

    #[test]
    fn matching_identity_is_accepted() {
        let node = node("0/1");
        let expectation = NodeExpectation::from_node(&node);
        assert!(expectation.find(&hierarchy(vec![node])).is_ok());
    }

    #[test]
    fn removed_node_is_stale() {
        let expectation = NodeExpectation::from_node(&node("0/1"));
        let error = expectation.find(&hierarchy(Vec::new())).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ElementRefStale);
    }

    #[test]
    fn changed_identity_is_rejected() {
        let expectation = NodeExpectation::from_node(&node("0/1"));
        let mut changed = node("0/1");
        changed.text = Some("Delete".to_string());
        let error = expectation.find(&hierarchy(vec![changed])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ElementIdentityChanged);
    }

    #[test]
    fn disabled_node_is_not_actionable() {
        let mut node = node("0/1");
        let expectation = NodeExpectation::from_node(&node);
        node.enabled = false;
        // The identity changed first because enabled is part of identity; both
        // outcomes refuse dispatch, which is the safety property that matters.
        let error = expectation.find(&hierarchy(vec![node])).unwrap_err();
        assert!(
            matches!(
                error.code(),
                ErrorCode::ElementIdentityChanged | ErrorCode::ElementNotActionable
            ),
            "{:?}",
            error.code()
        );
    }

    #[test]
    fn plain_text_is_not_actionable() {
        let mut node = node("0/1");
        node.clickable = false;
        node.class = Some("android.widget.TextView".to_string());
        let expectation = NodeExpectation::from_node(&node);
        let error = expectation.find(&hierarchy(vec![node])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ElementNotActionable);
    }

    #[test]
    fn presence_accepts_non_actionable_context_text() {
        let mut node = node("0/1");
        node.clickable = false;
        node.class = Some("android.widget.TextView".to_string());
        let expectation = NodeExpectation::from_node(&node);
        assert!(
            expectation
                .find_presence(&hierarchy(vec![node.clone()]))
                .is_ok()
        );
        assert!(expectation.find(&hierarchy(vec![node])).is_err());
    }

    #[test]
    fn core_identity_ignores_bounds_but_rejects_changed_text() {
        let node = node("0/1");
        let expectation = NodeExpectation::from_node(&node);
        let mut moved = node.clone();
        moved.bounds.left += 10;
        assert!(expectation.matches_core(&moved));
        assert!(!expectation.matches_identity(&moved));
        let mut relabelled = node;
        relabelled.text = Some("Delete".to_string());
        assert!(!expectation.matches_core(&relabelled));
    }
}
