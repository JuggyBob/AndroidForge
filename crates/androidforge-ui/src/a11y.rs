//! Read-only accessibility and contrast audit primitives.
//!
//! Everything in this module is pure analysis of data the caller already has:
//! a [`UiHierarchy`] (usually from `uiautomator dump`) and/or a decoded
//! [`Image`] (usually from `screencap`). Nothing here touches a device, and
//! the same inputs always produce the same [`A11yReport`].
//!
//! # Rules and thresholds
//!
//! | Rule | Trigger | Default threshold | Severity |
//! |---|---|---|---|
//! | [`A11yRule::MissingContentDescription`] | Actionable (clickable/long-clickable) node or image node has neither a content description nor visible text, and has no labelled clickable ancestor | none | error for actionable, warning for image |
//! | [`A11yRule::TinyTouchTarget`] | Actionable node is smaller than the required size on either axis | 48 dp (Material / WCAG 2.5.5 target size), converted with `density` px-per-dp (`1.0` by default) | warning |
//! | [`A11yRule::DuplicateLabel`] | Two or more actionable nodes share the same normalised accessible label | none (whitespace/case normalised) | warning |
//! | [`A11yRule::LowContrast`] | Text/background WCAG ratio is below the configured threshold | 4.5:1 for normal text, 3.0:1 for large text | error |
//!
//! Visible text is accepted as an accessible name because TalkBack reads
//! `text` when `content-desc` is absent; a labelled clickable ancestor makes
//! a nested image decorative, so it is not flagged. An image node is detected
//! by an `image` substring in its class (covers `ImageView`/`ImageButton`).
//!
//! **Large-text exception.** WCAG 2.x allows 3:1 for "large text": at least
//! 18 pt (approximately 24 sp at 1 sp = 1 px) or at least 14 pt bold
//! (approximately 18.66 sp). UIAutomator does not expose font size or weight,
//! so hierarchy-only audits cannot infer it. Callers that know the text is
//! large should pass `large_text = true` to [`check_contrast`] or use
//! [`A11yOptions::contrast_threshold_for_sp`] with a known size; the default
//! path deliberately applies the stricter 4.5:1 threshold. The 14 pt bold
//! exception must be requested explicitly because weight is unavailable.
//!
//! **Touch-target units.** UIAutomator bounds are device pixels, while the
//! guideline is 48 dp. [`A11yOptions::min_touch_target_px`] multiplies the dp
//! threshold by the configured density. Density defaults to `1.0` (treat px
//! as dp); a real device should supply its density to avoid false positives.
//!
//! Output is bounded by [`A11yOptions::max_findings`] and
//! [`A11yOptions::max_nodes`], and findings are emitted in hierarchy order
//! with a fixed per-node rule order, so reports are stable and diffable.

use std::collections::BTreeMap;
use std::fmt;

use androidforge_core::error::Result;
use serde::{Deserialize, Serialize};

use crate::color::{self, Region, Rgb};
use crate::hierarchy::{Rect, UiHierarchy, UiNode};
use crate::visual::Image;

/// Minimum recommended touch-target size in dp (Material design and WCAG
/// 2.5.5 AAA; WCAG 2.5.8 AA accepts 24 dp, which can be configured).
pub const DEFAULT_MIN_TOUCH_TARGET_DP: f64 = 48.0;
/// Pixels per dp. `1.0` means "treat hierarchy pixels as dp".
pub const DEFAULT_DENSITY: f64 = 1.0;
/// WCAG AA contrast ratio for normal text.
pub const DEFAULT_MIN_CONTRAST_RATIO: f64 = 4.5;
/// WCAG AA contrast ratio for large text (see module docs).
pub const DEFAULT_LARGE_TEXT_CONTRAST_RATIO: f64 = 3.0;
/// Text size in sp at or above which the large-text ratio is used.
pub const DEFAULT_LARGE_TEXT_MIN_SP: f64 = 24.0;
/// Default cap on emitted findings.
pub const DEFAULT_MAX_FINDINGS: usize = 200;
/// Default cap on visible nodes examined.
pub const DEFAULT_MAX_NODES: usize = 5000;

/// The deterministic rule that produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum A11yRule {
    MissingContentDescription,
    TinyTouchTarget,
    DuplicateLabel,
    LowContrast,
}

impl A11yRule {
    /// Stable machine-readable slug (matches the serde representation).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingContentDescription => "missing_content_description",
            Self::TinyTouchTarget => "tiny_touch_target",
            Self::DuplicateLabel => "duplicate_label",
            Self::LowContrast => "low_contrast",
        }
    }
}

impl fmt::Display for A11yRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How seriously a finding should be treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The measured value behind a finding, kept structured for machine consumers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum A11yMeasurement {
    MissingContentDescription,
    TinyTouchTarget {
        width_px: i32,
        height_px: i32,
        required_px: f64,
    },
    DuplicateLabel {
        count: usize,
    },
    LowContrast {
        ratio: f64,
        threshold: f64,
        foreground_hex: String,
        background_hex: String,
    },
}

impl A11yMeasurement {
    /// A single scalar for sorting/reporting; `None` when no number applies.
    pub fn numeric_value(&self) -> Option<f64> {
        match self {
            Self::MissingContentDescription => None,
            Self::TinyTouchTarget {
                width_px,
                height_px,
                ..
            } => Some((*width_px).min(*height_px) as f64),
            Self::DuplicateLabel { count } => Some(*count as f64),
            Self::LowContrast { ratio, .. } => Some(*ratio),
        }
    }
}

/// One actionable accessibility finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct A11yFinding {
    pub rule: A11yRule,
    pub severity: Severity,
    pub node_path: String,
    pub node_class: Option<String>,
    pub bounds: Rect,
    /// Accessible name of the node (content description, else visible text).
    pub label: Option<String>,
    pub measured: A11yMeasurement,
    /// Actionable, human-readable remediation guidance.
    pub message: String,
}

/// Configuration for [`audit_hierarchy`] and the contrast helpers.
///
/// All fields are public; [`A11yOptions::sanitized`] clamps non-finite or
/// nonsensical values so the audit functions stay total and deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct A11yOptions {
    /// Minimum touch target in dp. Default 48.
    pub min_touch_target_dp: f64,
    /// Device density in px-per-dp. Default 1.0 (px treated as dp).
    pub density: f64,
    /// WCAG ratio required for normal text. Default 4.5.
    pub min_contrast_ratio: f64,
    /// WCAG ratio required for large text. Default 3.0.
    pub large_text_contrast_ratio: f64,
    /// Text size in sp at which the large-text ratio applies. Default 24.
    pub large_text_min_sp: f64,
    pub check_content_descriptions: bool,
    pub check_touch_targets: bool,
    pub check_duplicate_labels: bool,
    /// Maximum findings emitted. Default 200.
    pub max_findings: usize,
    /// Maximum visible nodes examined. Default 5000.
    pub max_nodes: usize,
}

impl Default for A11yOptions {
    fn default() -> Self {
        Self {
            min_touch_target_dp: DEFAULT_MIN_TOUCH_TARGET_DP,
            density: DEFAULT_DENSITY,
            min_contrast_ratio: DEFAULT_MIN_CONTRAST_RATIO,
            large_text_contrast_ratio: DEFAULT_LARGE_TEXT_CONTRAST_RATIO,
            large_text_min_sp: DEFAULT_LARGE_TEXT_MIN_SP,
            check_content_descriptions: true,
            check_touch_targets: true,
            check_duplicate_labels: true,
            max_findings: DEFAULT_MAX_FINDINGS,
            max_nodes: DEFAULT_MAX_NODES,
        }
    }
}

impl A11yOptions {
    /// Clamp values to documented, finite ranges. Called automatically by the
    /// audit functions; exposed so callers can record the effective config.
    pub fn sanitized(self) -> Self {
        let finite_or = |value: f64, default: f64| {
            if value.is_finite() { value } else { default }
        };
        Self {
            min_touch_target_dp: finite_or(self.min_touch_target_dp, DEFAULT_MIN_TOUCH_TARGET_DP)
                .clamp(1.0, 256.0),
            density: finite_or(self.density, DEFAULT_DENSITY).clamp(0.25, 4.0),
            min_contrast_ratio: finite_or(self.min_contrast_ratio, DEFAULT_MIN_CONTRAST_RATIO)
                .clamp(1.0, 21.0),
            large_text_contrast_ratio: finite_or(
                self.large_text_contrast_ratio,
                DEFAULT_LARGE_TEXT_CONTRAST_RATIO,
            )
            .clamp(1.0, 21.0),
            large_text_min_sp: finite_or(self.large_text_min_sp, DEFAULT_LARGE_TEXT_MIN_SP)
                .clamp(1.0, 200.0),
            check_content_descriptions: self.check_content_descriptions,
            check_touch_targets: self.check_touch_targets,
            check_duplicate_labels: self.check_duplicate_labels,
            max_findings: self.max_findings,
            max_nodes: self.max_nodes,
        }
    }

    /// Effective minimum touch-target size in device pixels.
    pub fn min_touch_target_px(&self) -> f64 {
        self.min_touch_target_dp * self.density
    }

    /// Ratio to use when the caller knows whether the text is large.
    pub fn contrast_threshold(&self, large_text: bool) -> f64 {
        if large_text {
            self.large_text_contrast_ratio
        } else {
            self.min_contrast_ratio
        }
    }

    /// Ratio to use from a known text size in sp. Callers with bold text that
    /// is 14 pt (about 18.66 sp) should pass `large_text = true` explicitly,
    /// since font weight is not available in the hierarchy.
    pub fn contrast_threshold_for_sp(&self, text_sp: f64) -> f64 {
        self.contrast_threshold(text_sp.is_finite() && text_sp >= self.large_text_min_sp)
    }
}

/// Bounded result of [`audit_hierarchy`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct A11yReport {
    pub findings: Vec<A11yFinding>,
    /// Number of visible nodes actually examined (after `max_nodes`).
    pub nodes_examined: usize,
    /// Effective (sanitised) options used for this report.
    pub options: A11yOptions,
    /// True when a configured cap truncated the scan or the findings list.
    pub truncated: bool,
}

impl A11yReport {
    pub fn new(options: A11yOptions) -> Self {
        Self {
            findings: Vec::new(),
            nodes_examined: 0,
            options,
            truncated: false,
        }
    }

    pub fn count(&self, rule: A11yRule) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.rule == rule)
            .count()
    }

    pub fn count_by_severity(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .count()
    }

    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Accessible name of a node: content description, else visible text.
///
/// Both candidates are trimmed and empty strings are ignored. TalkBack uses
/// `text` as the name when `content-desc` is absent, so this mirrors the
/// runtime behaviour rather than requiring a content description everywhere.
pub fn accessible_label(node: &UiNode) -> Option<&str> {
    node.content_desc
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .or_else(|| {
            node.text
                .as_deref()
                .map(str::trim)
                .filter(|label| !label.is_empty())
        })
}

fn has_text(node: &UiNode) -> bool {
    node.text
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
}

fn has_content_description(node: &UiNode) -> bool {
    node.content_desc
        .as_deref()
        .is_some_and(|desc| !desc.trim().is_empty())
}

fn is_actionable(node: &UiNode) -> bool {
    node.clickable || node.long_clickable
}

fn is_image(node: &UiNode) -> bool {
    node.class
        .as_deref()
        .is_some_and(|class| class.to_ascii_lowercase().contains("image"))
}

/// Normalise a label for duplicate detection: collapse whitespace and case.
fn normalize_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn make_finding(
    rule: A11yRule,
    severity: Severity,
    node: &UiNode,
    measured: A11yMeasurement,
    message: String,
) -> A11yFinding {
    A11yFinding {
        rule,
        severity,
        node_path: node.path.clone(),
        node_class: node.class.clone(),
        bounds: node.bounds,
        label: accessible_label(node).map(str::to_string),
        measured,
        message,
    }
}

/// Audit a UI hierarchy for structural accessibility issues.
///
/// Only visible nodes are considered, and at most `options.max_nodes` of them.
/// Findings are produced in hierarchy order with rule order
/// missing-description, tiny-target, duplicate-label. The scan stops once
/// `options.max_findings` findings have been emitted.
pub fn audit_hierarchy(hierarchy: &UiHierarchy, options: A11yOptions) -> A11yReport {
    let options = options.sanitized();
    let mut report = A11yReport::new(options);

    let visible: Vec<&UiNode> = hierarchy
        .nodes
        .iter()
        .filter(|node| node.visible())
        .take(options.max_nodes)
        .collect();
    report.nodes_examined = visible.len();
    report.truncated = visible.len() < hierarchy.nodes.iter().filter(|node| node.visible()).count();

    let mut label_counts: BTreeMap<String, usize> = BTreeMap::new();
    if options.check_duplicate_labels {
        for node in &visible {
            if !is_actionable(node) {
                continue;
            }
            if let Some(label) = accessible_label(node) {
                *label_counts.entry(normalize_label(label)).or_insert(0) += 1;
            }
        }
    }

    let required_px = options.min_touch_target_px();
    for node in visible {
        if report.findings.len() >= options.max_findings {
            report.truncated = true;
            break;
        }

        let actionable = is_actionable(node);
        let image = is_image(node);

        if options.check_content_descriptions
            && (actionable || image)
            && !has_content_description(node)
        {
            let labelled_ancestor = image
                && hierarchy
                    .clickable_ancestor(node)
                    .is_some_and(|ancestor| accessible_label(ancestor).is_some());
            if !has_text(node) && !labelled_ancestor {
                let severity = if actionable {
                    Severity::Error
                } else {
                    Severity::Warning
                };
                let message = if actionable {
                    format!(
                        "Actionable node {} has no accessible name; set content-desc (or visible text) so TalkBack can announce it.",
                        node.path
                    )
                } else {
                    format!(
                        "Image node {} has no content-desc and no labelled clickable ancestor; mark it importantForAccessibility or add content-desc.",
                        node.path
                    )
                };
                report.findings.push(make_finding(
                    A11yRule::MissingContentDescription,
                    severity,
                    node,
                    A11yMeasurement::MissingContentDescription,
                    message,
                ));
            }
        }

        if options.check_touch_targets && actionable {
            let width = node.bounds.width();
            let height = node.bounds.height();
            if (width as f64) < required_px || (height as f64) < required_px {
                report.findings.push(make_finding(
                    A11yRule::TinyTouchTarget,
                    Severity::Warning,
                    node,
                    A11yMeasurement::TinyTouchTarget {
                        width_px: width,
                        height_px: height,
                        required_px,
                    },
                    format!(
                        "Touch target is {width}x{height} px, below the required {required_px} px ({} dp at density {}); add padding or increase the view size.",
                        options.min_touch_target_dp, options.density
                    ),
                ));
            }
        }

        if options.check_duplicate_labels
            && actionable
            && let Some(label) = accessible_label(node)
        {
            let count = label_counts
                .get(&normalize_label(label))
                .copied()
                .unwrap_or(0);
            if count > 1 {
                report.findings.push(make_finding(
                    A11yRule::DuplicateLabel,
                    Severity::Warning,
                    node,
                    A11yMeasurement::DuplicateLabel { count },
                    format!(
                        "Accessible label {label:?} is shared by {count} actionable nodes; give each control a distinct content description."
                    ),
                ));
            }
        }
    }

    report
}

/// Result of a WCAG text/background contrast check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContrastCheck {
    /// Foreground (text) colour as `#RRGGBB`.
    pub foreground_hex: String,
    /// Background colour as `#RRGGBB`.
    pub background_hex: String,
    pub ratio: f64,
    pub threshold: f64,
    pub passed: bool,
}

/// Compare text and background colours with the documented WCAG ratio.
///
/// `threshold` is caller-selected via [`A11yOptions::contrast_threshold`];
/// the comparison is inclusive (`ratio >= threshold` passes).
pub fn check_contrast(foreground: Rgb, background: Rgb, threshold: f64) -> ContrastCheck {
    let ratio = color::contrast_ratio(foreground, background);
    ContrastCheck {
        foreground_hex: foreground.hex(),
        background_hex: background.hex(),
        ratio,
        threshold,
        passed: ratio >= threshold,
    }
}

/// Sample the dominant background colour in a screenshot region and check it
/// against a known text colour.
///
/// The region's majority colour is used because antialiased glyph pixels would
/// otherwise dominate a naive average. Out-of-bounds or empty regions return
/// the explicit error from [`color::majority_color`].
pub fn audit_region_contrast(
    image: &Image,
    region: Region,
    foreground: Rgb,
    threshold: f64,
) -> Result<ContrastCheck> {
    let background = color::majority_color(image, region)?;
    Ok(check_contrast(foreground, background, threshold))
}

/// Clip a hierarchy bounds rectangle to a screenshot, returning `None` when
/// there is no positive-area intersection.
pub fn region_from_bounds(bounds: Rect, image: &Image) -> Option<Region> {
    let low = |value: i32| if value <= 0 { 0u32 } else { value as u32 };
    let high = |value: i32, max: u32| -> u32 {
        if value <= 0 {
            0
        } else if (value as u64) >= u64::from(max) {
            max
        } else {
            value as u32
        }
    };
    let x = low(bounds.left);
    let y = low(bounds.top);
    let right = high(bounds.right, image.width);
    let bottom = high(bounds.bottom, image.height);
    if right <= x || bottom <= y {
        return None;
    }
    Region::new(x, y, right - x, bottom - y).ok()
}

/// Convert a failed contrast check into an actionable finding for a node.
///
/// Returns `None` when the check passed. The `large_text` flag is recorded in
/// the message so a report makes the applied exception explicit.
pub fn contrast_finding(
    node: &UiNode,
    check: &ContrastCheck,
    large_text: bool,
) -> Option<A11yFinding> {
    if check.passed {
        return None;
    }
    let exception = if large_text {
        " (large-text 3:1 exception applied)"
    } else {
        ""
    };
    let message = format!(
        "Text {} on background {} has contrast {:.2}:1, below the required {:.2}:1{}; darken the text or lighten the background.",
        check.foreground_hex, check.background_hex, check.ratio, check.threshold, exception
    );
    Some(make_finding(
        A11yRule::LowContrast,
        Severity::Error,
        node,
        A11yMeasurement::LowContrast {
            ratio: check.ratio,
            threshold: check.threshold,
            foreground_hex: check.foreground_hex.clone(),
            background_hex: check.background_hex.clone(),
        },
        message,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: usize) -> UiNode {
        UiNode {
            id,
            parent: None,
            children: Vec::new(),
            depth: 0,
            text: None,
            content_desc: None,
            hint: None,
            resource_id: None,
            class: Some("android.widget.View".to_string()),
            package: None,
            bounds: Rect {
                left: 0,
                top: 0,
                right: 100,
                bottom: 100,
            },
            bounds_raw: "[0,0][100,100]".to_string(),
            clickable: false,
            long_clickable: false,
            enabled: true,
            focused: false,
            selected: false,
            scrollable: false,
            checked: false,
            password: false,
            displayed: Some(true),
            index_attr: None,
            path: id.to_string(),
        }
    }

    fn hierarchy(nodes: Vec<UiNode>) -> UiHierarchy {
        UiHierarchy {
            xml: String::new(),
            rotation: None,
            nodes,
            root: None,
            package_name: None,
        }
    }

    fn rules(report: &A11yReport, rule: A11yRule) -> Vec<&A11yFinding> {
        report
            .findings
            .iter()
            .filter(|finding| finding.rule == rule)
            .collect()
    }

    fn test_image(pixels: &[[u8; 4]], width: u32, height: u32) -> Image {
        Image::from_rgba(width, height, pixels.iter().flatten().copied().collect()).unwrap()
    }

    #[test]
    fn accessible_labels_follow_talkback_precedence() {
        let mut n = node(0);
        assert_eq!(accessible_label(&n), None);
        n.text = Some("  Save  ".to_string());
        assert_eq!(accessible_label(&n), Some("Save"));
        n.content_desc = Some("  Save changes  ".to_string());
        assert_eq!(accessible_label(&n), Some("Save changes"));
        n.content_desc = Some(String::new());
        assert_eq!(accessible_label(&n), Some("Save"));
    }

    #[test]
    fn clean_hierarchy_produces_no_findings() {
        let mut save = node(1);
        save.class = Some("android.widget.Button".to_string());
        save.content_desc = Some("Save changes".to_string());
        save.clickable = true;
        let report = audit_hierarchy(&hierarchy(vec![save]), A11yOptions::default());
        assert!(report.is_clean(), "{:?}", report.findings);
        assert_eq!(report.nodes_examined, 1);
        assert!(!report.truncated);
    }

    #[test]
    fn missing_content_description_flags_unlabelled_actionable_and_image() {
        let mut button = node(1);
        button.class = Some("android.widget.Button".to_string());
        button.clickable = true;
        let mut image = node(2);
        image.class = Some("android.widget.ImageView".to_string());
        let mut plain = node(3);
        plain.class = Some("android.widget.TextView".to_string());
        plain.text = Some("Heading".to_string());

        let report = audit_hierarchy(
            &hierarchy(vec![button, image, plain]),
            A11yOptions::default(),
        );
        let findings = rules(&report, A11yRule::MissingContentDescription);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].node_path, "1");
        assert_eq!(findings[0].severity, Severity::Error);
        assert_eq!(findings[1].node_path, "2");
        assert_eq!(findings[1].severity, Severity::Warning);
        assert!(findings[0].message.contains("content-desc"));
        assert_eq!(
            findings[0].measured,
            A11yMeasurement::MissingContentDescription
        );
    }

    #[test]
    fn visible_text_satisfies_the_name_requirement() {
        let mut button = node(1);
        button.class = Some("android.widget.Button".to_string());
        button.clickable = true;
        button.text = Some("  Save  ".to_string());
        let report = audit_hierarchy(&hierarchy(vec![button]), A11yOptions::default());
        assert_eq!(report.count(A11yRule::MissingContentDescription), 0);
    }

    #[test]
    fn labelled_clickable_ancestor_makes_nested_image_decorative() {
        let mut parent = node(0);
        parent.clickable = true;
        parent.content_desc = Some("Open profile".to_string());
        let mut image = node(1);
        image.parent = Some(0);
        image.class = Some("android.widget.ImageView".to_string());

        let report = audit_hierarchy(&hierarchy(vec![parent, image]), A11yOptions::default());
        assert_eq!(report.count(A11yRule::MissingContentDescription), 0);
    }

    #[test]
    fn tiny_touch_target_uses_configurable_density() {
        let mut button = node(1);
        button.clickable = true;
        button.bounds.right = 40;
        button.bounds.bottom = 32;
        let h = hierarchy(vec![button]);

        let strict = audit_hierarchy(&h, A11yOptions::default());
        let findings = rules(&strict, A11yRule::TinyTouchTarget);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].measured,
            A11yMeasurement::TinyTouchTarget {
                width_px: 40,
                height_px: 32,
                required_px: 48.0,
            }
        );
        assert_eq!(findings[0].severity, Severity::Warning);
        assert!(findings[0].message.contains("48 px"));

        let relaxed = audit_hierarchy(
            &h,
            A11yOptions {
                density: 0.5,
                ..A11yOptions::default()
            },
        );
        assert_eq!(relaxed.count(A11yRule::TinyTouchTarget), 0);
    }

    #[test]
    fn duplicate_labels_are_normalised_and_reported_per_node() {
        let mut first = node(1);
        first.clickable = true;
        first.text = Some("  Save  ".to_string());
        let mut second = node(2);
        second.clickable = true;
        second.content_desc = Some("save".to_string());
        let mut third = node(3);
        third.clickable = true;
        third.content_desc = Some("Cancel".to_string());

        let report = audit_hierarchy(
            &hierarchy(vec![first, second, third]),
            A11yOptions::default(),
        );
        let findings = rules(&report, A11yRule::DuplicateLabel);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].node_path, "1");
        assert_eq!(findings[1].node_path, "2");
        assert_eq!(
            findings[0].measured,
            A11yMeasurement::DuplicateLabel { count: 2 }
        );
    }

    #[test]
    fn hidden_and_zero_area_nodes_are_ignored() {
        let mut hidden = node(1);
        hidden.clickable = true;
        hidden.displayed = Some(false);
        let mut empty = node(2);
        empty.clickable = true;
        empty.bounds.right = 0;
        empty.bounds.bottom = 0;
        let report = audit_hierarchy(&hierarchy(vec![hidden, empty]), A11yOptions::default());
        assert_eq!(report.nodes_examined, 0);
        assert!(report.is_clean());
    }

    #[test]
    fn max_findings_and_max_nodes_bound_output() {
        let mut nodes = Vec::new();
        for id in 0..10 {
            let mut n = node(id);
            n.clickable = true;
            nodes.push(n);
        }
        let h = hierarchy(nodes);
        let capped = audit_hierarchy(
            &h,
            A11yOptions {
                max_findings: 2,
                ..A11yOptions::default()
            },
        );
        assert_eq!(capped.findings.len(), 2);
        assert!(capped.truncated);
        assert_eq!(capped.count(A11yRule::DuplicateLabel), 0);

        let node_limited = audit_hierarchy(
            &h,
            A11yOptions {
                max_nodes: 3,
                ..A11yOptions::default()
            },
        );
        assert_eq!(node_limited.nodes_examined, 3);
        assert!(node_limited.truncated);
    }

    #[test]
    fn reports_are_deterministic_and_options_are_sanitised() {
        let mut button = node(1);
        button.clickable = true;
        let h = hierarchy(vec![button]);
        let options = A11yOptions {
            min_touch_target_dp: f64::NAN,
            density: -3.0,
            min_contrast_ratio: 99.0,
            large_text_contrast_ratio: 0.0,
            large_text_min_sp: f64::NAN,
            ..A11yOptions::default()
        };
        let first = audit_hierarchy(&h, options);
        let second = audit_hierarchy(&h, options);
        assert_eq!(first, second);
        assert_eq!(first.options.density, 0.25);
        assert_eq!(first.options.min_touch_target_px(), 48.0 * 0.25);
        assert_eq!(first.options.min_contrast_ratio, 21.0);
        assert_eq!(first.options.large_text_contrast_ratio, 1.0);
        assert_eq!(first.options.large_text_min_sp, DEFAULT_LARGE_TEXT_MIN_SP);
    }

    #[test]
    fn contrast_thresholds_follow_large_text_rules() {
        let options = A11yOptions::default();
        assert_eq!(options.contrast_threshold(false), 4.5);
        assert_eq!(options.contrast_threshold(true), 3.0);
        assert_eq!(options.contrast_threshold_for_sp(18.0), 4.5);
        assert_eq!(options.contrast_threshold_for_sp(24.0), 3.0);
        assert_eq!(options.contrast_threshold_for_sp(f64::NAN), 4.5);
    }

    #[test]
    fn check_contrast_uses_inclusive_wcag_threshold() {
        let black = Rgb::new(0, 0, 0);
        let white = Rgb::new(255, 255, 255);
        let check = check_contrast(black, white, 21.0);
        assert!(check.passed);
        assert!((check.ratio - 21.0).abs() < 1e-9);
        assert_eq!(check.foreground_hex, "#000000");
        assert_eq!(check.background_hex, "#FFFFFF");
        assert!(!check_contrast(black, white, 21.0001).passed);
    }

    #[test]
    fn region_contrast_samples_the_majority_background() {
        // 3x3 region: eight white pixels and one black pixel. The white
        // majority must be chosen as the background.
        let image = test_image(
            &[
                [255, 255, 255, 255],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
                [0, 0, 0, 255],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
            ],
            3,
            3,
        );
        let region = Region::new(0, 0, 3, 3).unwrap();
        let check = audit_region_contrast(&image, region, Rgb::new(128, 128, 128), 4.5).unwrap();
        assert_eq!(check.background_hex, "#FFFFFF");
        assert!(!check.passed, "grey on white should fail 4.5:1");
        let passed = audit_region_contrast(&image, region, Rgb::new(0, 0, 0), 4.5).unwrap();
        assert!(passed.passed);
    }

    #[test]
    fn region_contrast_rejects_invalid_regions() {
        let image = test_image(&[[0, 0, 0, 255]], 1, 1);
        let error = audit_region_contrast(
            &image,
            Region::new(1, 0, 1, 1).unwrap(),
            Rgb::new(255, 255, 255),
            4.5,
        )
        .unwrap_err();
        assert_eq!(
            error.code(),
            androidforge_core::error::ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn region_from_bounds_clips_and_rejects_no_intersection() {
        let image = test_image(&[[0, 0, 0, 255]; 16], 4, 4);
        let bounds = Rect {
            left: -2,
            top: -1,
            right: 3,
            bottom: 9,
        };
        assert_eq!(
            region_from_bounds(bounds, &image),
            Some(Region {
                x: 0,
                y: 0,
                width: 3,
                height: 4,
            })
        );
        let outside = Rect {
            left: -5,
            top: -5,
            right: -1,
            bottom: -1,
        };
        assert_eq!(region_from_bounds(outside, &image), None);
    }

    #[test]
    fn contrast_finding_carries_measurement_and_message() {
        let mut node = node(1);
        node.class = Some("android.widget.TextView".to_string());
        node.text = Some("Low contrast".to_string());
        let check = check_contrast(Rgb::new(128, 128, 128), Rgb::new(255, 255, 255), 4.5);
        let finding = contrast_finding(&node, &check, false).unwrap();
        assert_eq!(finding.rule, A11yRule::LowContrast);
        assert_eq!(finding.severity, Severity::Error);
        assert_eq!(finding.bounds, node.bounds);
        assert_eq!(finding.label.as_deref(), Some("Low contrast"));
        assert_eq!(
            finding.measured,
            A11yMeasurement::LowContrast {
                ratio: check.ratio,
                threshold: 4.5,
                foreground_hex: "#808080".to_string(),
                background_hex: "#FFFFFF".to_string(),
            }
        );
        assert!(finding.message.contains("darken the text"));

        let large_check = check_contrast(Rgb::new(190, 190, 190), Rgb::new(255, 255, 255), 3.0);
        let large_finding = contrast_finding(&node, &large_check, true).unwrap();
        assert!(large_finding.message.contains("large-text"));

        let passing = check_contrast(Rgb::new(0, 0, 0), Rgb::new(255, 255, 255), 4.5);
        assert!(contrast_finding(&node, &passing, false).is_none());
    }
}
