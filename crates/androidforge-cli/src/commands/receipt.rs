//! Action receipts: one compact, evidence-backed answer to "what did my action
//! actually do?".
//!
//! Receipts reuse the same observations and semantic diffs as `ui observe` and
//! `ui diff`. They never infer an effect from raw coordinates and never retry a
//! state-changing action when a postcondition is not observed.

use std::time::Instant;

use androidforge_core::model::Device;
use androidforge_core::session::Session;
use androidforge_core::util::{now_rfc3339, short_id};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::observation::{Observation, ObservationChanges, ObservationNode};

pub const RECEIPT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    NoVisibleChange,
    ContentChanged,
    Navigated,
    DialogAppeared,
    DialogDisappeared,
    ElementDisappeared,
    PackageChanged,
    Unknown,
}

impl Effect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoVisibleChange => "no_visible_change",
            Self::ContentChanged => "content_changed",
            Self::Navigated => "navigated",
            Self::DialogAppeared => "dialog_appeared",
            Self::DialogDisappeared => "dialog_disappeared",
            Self::ElementDisappeared => "element_disappeared",
            Self::PackageChanged => "package_changed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchState {
    NotDispatched,
    Dispatched,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptCompleteness {
    Complete,
    BeforeMissing,
    AfterMissing,
    Incomplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptElementReference {
    #[serde(rename = "ref")]
    pub reference: String,
    pub expected_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_node: Option<Value>,
    pub validation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptObservationRef {
    pub token: String,
    pub observed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<String>,
}

impl ReceiptObservationRef {
    fn from_observation(observation: &Observation) -> Self {
        Self {
            token: observation.token.clone(),
            observed_at: observation.observed_at.clone(),
            foreground_package: observation.foreground.package.clone(),
            foreground_activity: observation.foreground.activity.clone(),
            focused: observation
                .focused
                .as_ref()
                .and_then(ObservationNode::label_for_receipt),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostconditionResult {
    NotRequested,
    Satisfied,
    TimedOut,
    NotEvaluated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostconditionCondition {
    #[serde(rename = "type")]
    pub kind: String,
    pub expected: String,
    pub satisfied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostconditionReceipt {
    pub result: PostconditionResult,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<PostconditionCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_activity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub schema_version: u32,
    pub receipt_id: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<Value>,
    pub session_id: String,
    pub device_serial: String,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    pub duration_ms: u64,
    pub target_resolved: bool,
    pub dispatched: bool,
    pub dispatch_state: DispatchState,
    pub target: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<ReceiptElementReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<ReceiptObservationRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<ReceiptObservationRef>,
    pub effect: Effect,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effect_evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<ObservationChanges>,
    pub completeness: ReceiptCompleteness,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postcondition: Option<PostconditionReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_result: Option<Value>,
}

pub struct ReceiptBuilder {
    receipt_id: String,
    action: String,
    selector: Option<Value>,
    session_id: String,
    device_serial: String,
    started_at: String,
    started: Instant,
    warnings: Vec<String>,
    before: Option<Observation>,
    reference: Option<ReceiptElementReference>,
}

impl ReceiptBuilder {
    pub fn new(
        action: impl Into<String>,
        selector: Option<Value>,
        session: &Session,
        device: &Device,
    ) -> Self {
        Self {
            receipt_id: format!("rcpt-{}", short_id()),
            action: action.into(),
            selector,
            session_id: session.id.clone(),
            device_serial: device.serial.clone(),
            started_at: now_rfc3339(),
            started: Instant::now(),
            warnings: Vec::new(),
            before: None,
            reference: None,
        }
    }

    pub fn set_before(&mut self, observation: Observation) {
        self.before = Some(observation);
    }

    pub fn set_reference(&mut self, reference: ReceiptElementReference) {
        self.reference = Some(reference);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish(
        self,
        dispatch_state: DispatchState,
        target_resolved: bool,
        target: Option<Value>,
        after: Option<Observation>,
        action_result: Option<Value>,
        postcondition: Option<PostconditionReceipt>,
        dispatch_error: Option<String>,
        full: bool,
    ) -> ActionReceipt {
        let mut warnings = self.warnings;
        let before_ref = self
            .before
            .as_ref()
            .map(ReceiptObservationRef::from_observation);
        let after_ref = after.as_ref().map(ReceiptObservationRef::from_observation);
        let mut changes = None;
        let mut effect = Effect::Unknown;
        let mut evidence = Vec::new();
        let dispatched = dispatch_state == DispatchState::Dispatched;
        let mut completeness = if dispatched {
            ReceiptCompleteness::Complete
        } else {
            ReceiptCompleteness::Incomplete
        };
        match (&self.before, &after) {
            (Some(before), Some(after)) => {
                let diff = crate::commands::observation::diff_observations(before, after, full);
                evidence.extend(effect_evidence(before, after, &diff.changes, &mut warnings));
                effect = classify(before, after, &diff.changes, &mut warnings);
                changes = Some(diff.changes);
            }
            (None, _) => {
                completeness = ReceiptCompleteness::BeforeMissing;
                warnings.push(
                    "The before-observation was unavailable, so the action effect cannot be classified."
                        .to_string(),
                );
            }
            (Some(_), None) => {
                completeness = if completeness == ReceiptCompleteness::Complete {
                    ReceiptCompleteness::AfterMissing
                } else {
                    completeness
                };
                warnings.push(
                    "The after-observation was unavailable, so the action effect cannot be classified."
                        .to_string(),
                );
            }
        }
        if dispatch_error.is_some() {
            completeness = ReceiptCompleteness::Incomplete;
        }
        if let Some(postcondition) = &postcondition {
            match postcondition.result {
                PostconditionResult::TimedOut => {
                    completeness = ReceiptCompleteness::Incomplete;
                }
                PostconditionResult::NotEvaluated
                    if completeness == ReceiptCompleteness::Complete =>
                {
                    completeness = ReceiptCompleteness::Incomplete;
                }
                _ => {}
            }
        }
        if changes.as_ref().is_some_and(|changes| changes.truncated)
            && !warnings.iter().any(|warning| warning.contains("truncated"))
        {
            warnings.push(
                "Semantic diff lists were truncated; added_total, removed_total and modified_total remain exact."
                    .to_string(),
            );
        }
        ActionReceipt {
            schema_version: RECEIPT_SCHEMA_VERSION,
            receipt_id: self.receipt_id,
            action: self.action,
            selector: self.selector,
            session_id: self.session_id,
            device_serial: self.device_serial,
            started_at: self.started_at,
            ended_at: Some(now_rfc3339()),
            duration_ms: self.started.elapsed().as_millis() as u64,
            target_resolved,
            dispatched,
            dispatch_state,
            target,
            reference: self.reference,
            before: before_ref,
            after: after_ref,
            effect,
            effect_evidence: evidence,
            changes,
            completeness,
            warnings,
            postcondition,
            dispatch_error,
            action_result,
        }
    }
}

/// Deterministic effect classification.
///
/// Precedence, highest first:
/// 1. `unknown` when observations are missing or clearly inconsistent.
/// 2. `package_changed` when the foreground package changed.
/// 3. `dialog_appeared` / `dialog_disappeared` when dialog-like nodes appeared
///    or disappeared within the same package.
/// 4. `navigated` when the foreground activity changed within the same package.
/// 5. `element_disappeared` when only removals were observed.
/// 6. `content_changed` for any semantic addition, removal or modification.
/// 7. `no_visible_change` when the observable semantic state is unchanged.
pub fn classify(
    before: &Observation,
    after: &Observation,
    changes: &ObservationChanges,
    warnings: &mut Vec<String>,
) -> Effect {
    if before.session_id != after.session_id || before.device_serial != after.device_serial {
        warnings.push(
            "The before and after observations belong to different sessions or devices."
                .to_string(),
        );
        return Effect::Unknown;
    }
    if before.token == after.token {
        warnings.push(
            "The same observation token was used for before and after; the screen was not re-read."
                .to_string(),
        );
        return Effect::Unknown;
    }
    if changes.package_changed {
        if changes.foreground_package_after.is_none() {
            warnings.push(
                "The foreground package became unavailable; the package change cannot be named."
                    .to_string(),
            );
        }
        return Effect::PackageChanged;
    }
    let added_dialog = dialog_evidence(&changes.added);
    let removed_dialog = dialog_evidence(&changes.removed);
    if added_dialog && !removed_dialog {
        return Effect::DialogAppeared;
    }
    if removed_dialog && !added_dialog {
        return Effect::DialogDisappeared;
    }
    if changes.activity_changed
        && changes.foreground_activity_before.is_some()
        && changes.foreground_activity_after.is_some()
    {
        return Effect::Navigated;
    }
    if changes.removed_total > 0 && changes.added_total == 0 && changes.modified_total == 0 {
        return Effect::ElementDisappeared;
    }
    if changes.added_total > 0 || changes.removed_total > 0 || changes.modified_total > 0 {
        return Effect::ContentChanged;
    }
    if changes.focus_changed {
        // Focus state is part of `modified` when the focused node remains in the
        // hierarchy. A pure focus change without nodes is still observable state.
        return Effect::ContentChanged;
    }
    Effect::NoVisibleChange
}

fn dialog_evidence(nodes: &[crate::commands::observation::DiffNode]) -> bool {
    let mut action_labels = 0usize;
    let mut has_dialog_class = false;
    for node in nodes {
        let class = node.class.as_deref().unwrap_or("").to_ascii_lowercase();
        if class.contains("dialog") || class.contains("alertdialog") {
            has_dialog_class = true;
        }
        let label = node
            .text
            .as_deref()
            .or(node.content_desc.as_deref())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if node.clickable
            && matches!(
                label.as_str(),
                "ok" | "cancel"
                    | "allow"
                    | "deny"
                    | "got it"
                    | "dismiss"
                    | "close"
                    | "while using the app"
                    | "only this time"
                    | "don't allow"
                    | "don’t allow"
            )
        {
            action_labels += 1;
        }
    }
    has_dialog_class || action_labels >= 2
}

fn effect_evidence(
    before: &Observation,
    after: &Observation,
    changes: &ObservationChanges,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut evidence = vec![format!(
        "semantic diff: {} added, {} removed, {} modified",
        changes.added_total, changes.removed_total, changes.modified_total
    )];
    if changes.package_changed {
        evidence.push(format!(
            "foreground package: {} -> {}",
            changes.foreground_package_before.as_deref().unwrap_or("-"),
            changes.foreground_package_after.as_deref().unwrap_or("-")
        ));
    }
    if changes.activity_changed {
        evidence.push(format!(
            "foreground activity: {} -> {}",
            changes.foreground_activity_before.as_deref().unwrap_or("-"),
            changes.foreground_activity_after.as_deref().unwrap_or("-")
        ));
    }
    if changes.focus_changed {
        evidence.push(format!(
            "focus: {} -> {}",
            changes.focus_before.as_deref().unwrap_or("-"),
            changes.focus_after.as_deref().unwrap_or("-")
        ));
    }
    if before.foreground.package.is_none() && after.foreground.package.is_none() {
        warnings.push(
            "Neither observation could read the foreground package; package-change classification is unavailable."
                .to_string(),
        );
    }
    evidence
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::observation::{
        ObservationBounds, ObservationDisplay, ObservationForeground, ObservationHierarchyMeta,
    };

    fn node(
        path: &str,
        text: &str,
        class: &str,
        clickable: bool,
        bounds: (i32, i32, i32, i32),
    ) -> ObservationNode {
        ObservationNode {
            reference: None,
            role: Some("button".to_string()),
            path: path.to_string(),
            depth: 2,
            text: Some(text.to_string()),
            content_desc: None,
            hint: None,
            resource_id: Some("com.example:id/control".to_string()),
            class: Some(class.to_string()),
            package: Some("com.example".to_string()),
            bounds: ObservationBounds {
                left: bounds.0,
                top: bounds.1,
                right: bounds.2,
                bottom: bounds.3,
            },
            clickable,
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

    fn observation(
        token: &str,
        package: &str,
        activity: &str,
        actionable: Vec<ObservationNode>,
    ) -> Observation {
        Observation {
            schema_version: 1,
            id: token.to_string(),
            token: token.to_string(),
            session_id: "ses-1".to_string(),
            device_serial: "emulator-5554".to_string(),
            observed_at: "2026-01-01T00:00:00Z".to_string(),
            age_ms: 0,
            stale: false,
            display: ObservationDisplay {
                width: Some(1080),
                height: Some(2400),
                orientation: "portrait".to_string(),
                rotation: Some(0),
            },
            foreground: ObservationForeground {
                package: Some(package.to_string()),
                activity: Some(activity.to_string()),
                window: None,
            },
            focused: None,
            actionable_total: actionable.len(),
            actionable,
            actionable_truncated: false,
            context: Vec::new(),
            context_total: 0,
            context_truncated: false,
            scrollable: Vec::new(),
            visible_text: Vec::new(),
            nodes: None,
            hierarchy: ObservationHierarchyMeta {
                package: Some(package.to_string()),
                rotation: Some(0),
                node_count: 1,
                compact_node_count: 1,
            },
            raw_xml_path: "obs.xml".to_string(),
        }
    }

    fn classify_between(before: Observation, after: Observation) -> (Effect, Vec<String>) {
        let diff = crate::commands::observation::diff_observations(&before, &after, false);
        let mut warnings = Vec::new();
        let effect = classify(&before, &after, &diff.changes, &mut warnings);
        (effect, warnings)
    }

    #[test]
    fn no_change_is_reported() {
        let before = observation(
            "obs-1",
            "com.example",
            ".Main",
            vec![node(
                "0",
                "Save",
                "android.widget.Button",
                true,
                (0, 0, 10, 10),
            )],
        );
        let after = before.clone_with_token("obs-2");
        let (effect, _) = classify_between(before, after);
        assert_eq!(effect, Effect::NoVisibleChange);
    }

    #[test]
    fn package_change_wins_over_content_change() {
        let before = observation(
            "obs-1",
            "com.example",
            ".Main",
            vec![node(
                "0",
                "Save",
                "android.widget.Button",
                true,
                (0, 0, 10, 10),
            )],
        );
        let after = observation(
            "obs-2",
            "com.android.settings",
            ".Settings",
            vec![node(
                "0",
                "Other",
                "android.widget.Button",
                true,
                (0, 0, 10, 10),
            )],
        );
        let (effect, _) = classify_between(before, after);
        assert_eq!(effect, Effect::PackageChanged);
    }

    #[test]
    fn activity_change_is_navigation() {
        let before = observation("obs-1", "com.example", ".Main", Vec::new());
        let after = observation("obs-2", "com.example", ".Details", Vec::new());
        let (effect, _) = classify_between(before, after);
        assert_eq!(effect, Effect::Navigated);
    }

    #[test]
    fn dialog_appearance_is_detected() {
        let before = observation("obs-1", "com.example", ".Main", Vec::new());
        let after = observation(
            "obs-2",
            "com.example",
            ".Main",
            vec![
                node(
                    "0",
                    "Discard changes?",
                    "android.app.AlertDialog",
                    false,
                    (0, 0, 900, 600),
                ),
                node(
                    "1",
                    "Cancel",
                    "android.widget.Button",
                    true,
                    (0, 0, 100, 50),
                ),
                node("2", "OK", "android.widget.Button", true, (0, 0, 100, 50)),
            ],
        );
        let (effect, _) = classify_between(before, after);
        assert_eq!(effect, Effect::DialogAppeared);
    }

    #[test]
    fn dialog_disappearance_is_detected() {
        let before = observation(
            "obs-1",
            "com.example",
            ".Main",
            vec![
                node(
                    "0",
                    "Discard changes?",
                    "android.app.AlertDialog",
                    false,
                    (0, 0, 900, 600),
                ),
                node(
                    "1",
                    "Cancel",
                    "android.widget.Button",
                    true,
                    (0, 0, 100, 50),
                ),
                node("2", "OK", "android.widget.Button", true, (0, 0, 100, 50)),
            ],
        );
        let after = observation("obs-2", "com.example", ".Main", Vec::new());
        let (effect, _) = classify_between(before, after);
        assert_eq!(effect, Effect::DialogDisappeared);
    }

    #[test]
    fn removed_only_content_is_element_disappeared() {
        let before = observation(
            "obs-1",
            "com.example",
            ".Main",
            vec![node(
                "0",
                "Delete",
                "android.widget.Button",
                true,
                (0, 0, 10, 10),
            )],
        );
        let after = observation("obs-2", "com.example", ".Main", Vec::new());
        let (effect, _) = classify_between(before, after);
        assert_eq!(effect, Effect::ElementDisappeared);
    }

    #[test]
    fn same_token_is_unknown() {
        let before = observation("obs-1", "com.example", ".Main", Vec::new());
        let after = before.clone_with_token("obs-1");
        let (effect, warnings) = classify_between(before, after);
        assert_eq!(effect, Effect::Unknown);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("same observation"))
        );
    }

    impl Observation {
        fn clone_with_token(&self, token: &str) -> Self {
            let mut clone = self.clone();
            clone.id = token.to_string();
            clone.token = token.to_string();
            clone
        }
    }
}
