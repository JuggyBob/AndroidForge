use std::time::{Duration, Instant};

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::util::short_id;
use androidforge_process::escape_input_text;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hierarchy::{Rect, UiHierarchy, UiNode};
use crate::selector::{Selection, Selector, UiMatch, find, select, selector_not_found};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindResult {
    pub matches: Vec<UiMatch>,
    pub ambiguous: bool,
    #[serde(default)]
    pub ocr_used: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TapResult {
    pub matched: UiMatch,
    pub tap_x: i32,
    pub tap_y: i32,
    pub used_clickable_ancestor: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ancestor: Option<UiMatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_size: Option<(u32, u32)>,
    pub rotation: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextResult {
    pub text: String,
    pub cleared: bool,
    pub verified: bool,
    pub focused: UiMatch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_was_visible: Option<bool>,
    pub keyboard_hidden: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitResult {
    pub present: bool,
    pub matched: bool,
    pub attempts: u32,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_element: Option<UiMatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hierarchy_package: Option<String>,
}

/// Result of waiting for a set of user-visible strings to disappear.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextAbsenceResult {
    pub absent: bool,
    pub attempts: u32,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lingering: Vec<UiMatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hierarchy_package: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettleResult {
    pub stable: bool,
    pub attempts: u32,
    pub duration_ms: u64,
    pub xml_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollResult {
    pub direction: String,
    pub start_x: i32,
    pub start_y: i32,
    pub end_x: i32,
    pub end_y: i32,
    pub target_bounds: Rect,
    pub used_scrollable_node: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollToResult {
    pub found: bool,
    pub swipes: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_element: Option<UiMatch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackResult {
    pub keyboard_was_visible: Option<bool>,
    pub keyboard_hidden: bool,
    pub back_sent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyResult {
    pub keycode: String,
}

/// A device-independent snapshot of a UI element's bounds.
///
/// Produced by exactly one read-only selector resolution. All values are
/// integer device pixels in the current display orientation; `width`,
/// `height`, `center_x` and `center_y` are derived from `Rect` so every caller
/// sees the same numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub width: i32,
    pub height: i32,
    pub center_x: i32,
    pub center_y: i32,
}

impl Bounds {
    /// The bottom edge coordinate asserted by `ui assert-bounds`.
    pub fn bottom_y(&self) -> i32 {
        self.bottom
    }
}

impl From<Rect> for Bounds {
    fn from(rect: Rect) -> Self {
        let (center_x, center_y) = rect.center();
        Self {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
            width: rect.width(),
            height: rect.height(),
            center_x,
            center_y,
        }
    }
}

/// The relation evaluated by a bounds comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoundsRelation {
    #[serde(rename = "eq")]
    Equal,
    #[serde(rename = "gt")]
    GreaterThan,
    #[serde(rename = "lt")]
    LessThan,
}

impl BoundsRelation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Equal => "eq",
            Self::GreaterThan => "gt",
            Self::LessThan => "lt",
        }
    }
}

/// Structured result of a `bottom_y` assertion.
///
/// `eq` passes when `abs(actual - expected) <= tolerance`. `gt` passes when
/// `actual > expected + tolerance`, and `lt` passes when
/// `actual < expected - tolerance`; the tolerance therefore widens the dead
/// band that must be exceeded before a strict comparison passes. Negative
/// tolerances are clamped to zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundsComparison {
    pub relation: BoundsRelation,
    pub expected: i32,
    pub actual: i32,
    pub tolerance: i32,
    pub passed: bool,
}

impl BoundsComparison {
    /// Turn a comparison into an assertion result.
    ///
    /// A passing comparison is returned unchanged. A failing comparison
    /// becomes `E_UI_ASSERTION_FAILED` and carries the structured comparison
    /// in its error context, so callers never have to reinterpret raw numbers.
    pub fn require_passed(self) -> Result<Self> {
        if self.passed {
            return Ok(self);
        }
        Err(ForgeError::new(
            ErrorCode::UiAssertionFailed,
            format!(
                "bottom_y {} failed: expected {} (tolerance {}), actual {}",
                self.relation.as_str(),
                self.expected,
                self.tolerance,
                self.actual
            ),
        )
        .with_context(
            "comparison",
            serde_json::to_value(self).unwrap_or(Value::Null),
        )
        .with_context("relation", self.relation.as_str())
        .with_context("expected", self.expected)
        .with_context("actual", self.actual)
        .with_context("tolerance", self.tolerance)
        .with_context("passed", self.passed))
    }
}

/// Result of [`UiManager::wait_for_bounds_stable`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundsStabilityResult {
    pub stable: bool,
    pub bounds: Bounds,
    pub attempts: u32,
    pub duration_ms: u64,
    pub stable_for_ms: u64,
}

#[derive(Debug, Clone)]
pub struct UiManager {
    pub adb: AdbClient,
    pub default_timeout: Duration,
    pub poll_interval: Duration,
    pub settle_interval: Duration,
    pub tap_clickable_ancestor: bool,
    pub text_verify: bool,
}

impl UiManager {
    pub fn new(
        adb: AdbClient,
        default_timeout: Duration,
        poll_interval: Duration,
        settle_interval: Duration,
        tap_clickable_ancestor: bool,
        text_verify: bool,
    ) -> Self {
        Self {
            adb,
            default_timeout,
            poll_interval,
            settle_interval,
            tap_clickable_ancestor,
            text_verify,
        }
    }

    pub async fn dump(&self, serial: &str) -> Result<UiHierarchy> {
        let remote = format!("/data/local/tmp/androidforge-ui-{}.xml", short_id());
        let mut last_error = None;
        let mut recovery_attempted = false;
        for attempt in 0..3 {
            let dump_output = self
                .adb
                .shell_args(
                    serial,
                    &[
                        "uiautomator".to_string(),
                        "dump".to_string(),
                        remote.clone(),
                    ],
                    Some(Duration::from_secs(30)),
                    Some(2 * 1024 * 1024),
                )
                .await;
            match dump_output {
                Ok(output) => {
                    let text = output.combined_text();
                    if text.contains("ERROR: could not get idle state")
                        || text.contains("ERROR: null root node")
                        || text.contains("ERROR: could not parse")
                    {
                        last_error = Some(text.trim().to_string());
                    } else {
                        match self
                            .adb
                            .exec_out_args(
                                serial,
                                &["cat".to_string(), remote.clone()],
                                Some(Duration::from_secs(30)),
                                Some(16 * 1024 * 1024),
                            )
                            .await
                        {
                            Ok(xml_output) => {
                                if xml_output.stdout_truncated {
                                    last_error = Some(
                                        "UI hierarchy exceeded the 16 MiB capture limit"
                                            .to_string(),
                                    );
                                } else {
                                    let xml = xml_output.stdout_text();
                                    if xml.contains("<hierarchy") {
                                        match UiHierarchy::parse(&xml) {
                                            Ok(hierarchy) => {
                                                let _ = self
                                                    .adb
                                                    .shell_args(
                                                        serial,
                                                        &[
                                                            "rm".to_string(),
                                                            "-f".to_string(),
                                                            remote.clone(),
                                                        ],
                                                        Some(Duration::from_secs(10)),
                                                        None,
                                                    )
                                                    .await;
                                                if hierarchy.nodes.is_empty() {
                                                    // A rapid screen transition can
                                                    // produce a valid but empty
                                                    // hierarchy. Treat it as
                                                    // transient rather than
                                                    // reporting a blank screen.
                                                    last_error = Some(
                                                        "uiautomator returned an empty hierarchy"
                                                            .to_string(),
                                                    );
                                                } else {
                                                    return Ok(hierarchy);
                                                }
                                            }
                                            Err(error) => {
                                                last_error = Some(error.message().to_string());
                                            }
                                        }
                                    } else {
                                        last_error = Some(
                                            "uiautomator produced empty or partial XML".to_string(),
                                        );
                                    }
                                }
                            }
                            Err(error) => {
                                last_error = Some(error.message().to_string());
                            }
                        }
                    }
                }
                Err(error) => {
                    last_error = Some(error.message().to_string());
                    if error.code() == ErrorCode::UnsupportedOperation {
                        break;
                    }
                }
            }
            if attempt < 2 {
                let transient = last_error
                    .as_deref()
                    .is_some_and(is_transient_uiautomator_failure);
                if transient {
                    recovery_attempted = true;
                    recover_uiautomator(&self.adb, serial).await;
                }
                tokio::time::sleep(Duration::from_millis(if transient { 750 } else { 450 })).await;
            }
        }
        let _ = self
            .adb
            .shell_args(
                serial,
                &["rm".to_string(), "-f".to_string(), remote],
                Some(Duration::from_secs(10)),
                None,
            )
            .await;
        Err(ForgeError::new(
            ErrorCode::UiAutomatorUnavailable,
            "could not obtain a usable UIAutomator hierarchy",
        )
        .with_context_string(
            "last_error",
            last_error.unwrap_or_else(|| "unknown UIAutomator failure".to_string()),
        )
        .with_context("attempts", serde_json::json!(3))
        .with_context("recovery_attempted", serde_json::json!(recovery_attempted))
        .with_suggestion(
            "Retry; AndroidForge already attempted to clear a stuck UIAutomator helper.",
        )
        .with_suggestion(
            "If it persists, run `androidforge adb shell pkill -9 -f uiautomator` and retry.",
        )
        .with_suggestion("Inspect the device with `androidforge diagnostics collect`.")
        .device(serial.to_string()))
    }

    pub async fn find(&self, serial: &str, selector: &Selector) -> Result<FindResult> {
        let hierarchy = self.dump(serial).await?;
        let matches = find(&hierarchy, selector)?;
        Ok(FindResult {
            ambiguous: matches.len() > 1,
            matches,
            ocr_used: false,
        })
    }

    pub async fn select(&self, serial: &str, selector: &Selector) -> Result<Selection> {
        let hierarchy = self.dump(serial).await?;
        select(hierarchy, selector)
    }

    /// Resolve `selector` once and return a stable bounds snapshot.
    ///
    /// This is read-only: one hierarchy dump, one [`select`] call and no
    /// coordinate or tap fallback. Ambiguous selectors fail closed with
    /// `E_SELECTOR_AMBIGUOUS` through the existing selector logic.
    pub async fn bounds(&self, serial: &str, selector: &Selector) -> Result<Bounds> {
        let hierarchy = self.dump(serial).await?;
        bounds_from_hierarchy(&hierarchy, selector)
    }

    /// Compare the resolved element's `bottom_y` without asserting the result.
    pub async fn check_bottom_y(
        &self,
        serial: &str,
        selector: &Selector,
        expected: i32,
        tolerance: i32,
    ) -> Result<BoundsComparison> {
        let bounds = self.bounds(serial, selector).await?;
        Ok(compare_bottom_y(bounds.bottom_y(), expected, tolerance))
    }

    /// Assert equality of `bottom_y` within `tolerance`.
    pub async fn assert_bottom_y(
        &self,
        serial: &str,
        selector: &Selector,
        expected: i32,
        tolerance: i32,
    ) -> Result<BoundsComparison> {
        let comparison = self
            .check_bottom_y(serial, selector, expected, tolerance)
            .await?;
        comparison
            .require_passed()
            .map_err(|error| error.device(serial.to_string()))
    }

    /// Compare whether `bottom_y` exceeds `expected` by more than `tolerance`.
    pub async fn check_bottom_y_gt(
        &self,
        serial: &str,
        selector: &Selector,
        expected: i32,
        tolerance: i32,
    ) -> Result<BoundsComparison> {
        let bounds = self.bounds(serial, selector).await?;
        Ok(compare_bottom_y_gt(bounds.bottom_y(), expected, tolerance))
    }

    /// Assert that `bottom_y` is greater than `expected + tolerance`.
    pub async fn assert_bottom_y_gt(
        &self,
        serial: &str,
        selector: &Selector,
        expected: i32,
        tolerance: i32,
    ) -> Result<BoundsComparison> {
        let comparison = self
            .check_bottom_y_gt(serial, selector, expected, tolerance)
            .await?;
        comparison
            .require_passed()
            .map_err(|error| error.device(serial.to_string()))
    }

    /// Compare whether `bottom_y` falls below `expected` by more than `tolerance`.
    pub async fn check_bottom_y_lt(
        &self,
        serial: &str,
        selector: &Selector,
        expected: i32,
        tolerance: i32,
    ) -> Result<BoundsComparison> {
        let bounds = self.bounds(serial, selector).await?;
        Ok(compare_bottom_y_lt(bounds.bottom_y(), expected, tolerance))
    }

    /// Assert that `bottom_y` is less than `expected - tolerance`.
    pub async fn assert_bottom_y_lt(
        &self,
        serial: &str,
        selector: &Selector,
        expected: i32,
        tolerance: i32,
    ) -> Result<BoundsComparison> {
        let comparison = self
            .check_bottom_y_lt(serial, selector, expected, tolerance)
            .await?;
        comparison
            .require_passed()
            .map_err(|error| error.device(serial.to_string()))
    }

    pub async fn tap(&self, serial: &str, selector: &Selector) -> Result<TapResult> {
        let selection = self.select(serial, selector).await?;
        self.tap_selection(serial, selection).await
    }

    /// Tap a node from a previous observation after proving that the node still
    /// has the recorded identity. No selector or coordinate fallback is used.
    pub async fn tap_expected(
        &self,
        serial: &str,
        expected: &crate::reference::NodeExpectation,
    ) -> Result<TapResult> {
        let hierarchy = self.dump(serial).await?;
        let node_id = expected.find(&hierarchy)?.id;
        self.tap_selection(
            serial,
            Selection {
                hierarchy,
                indices: vec![node_id],
                selected: node_id,
            },
        )
        .await
    }

    pub async fn tap_xy(&self, serial: &str, x: i32, y: i32) -> Result<Option<(u32, u32)>> {
        let display = self.display_size(serial, None).await;
        if x < 0 || y < 0 {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                format!("tap coordinate ({x}, {y}) must be non-negative"),
            )
            .with_suggestion("Prefer semantic selectors such as --text or --resource-id.")
            .device(serial.to_string()));
        }
        if let Some((width, height)) = display
            && (x >= width as i32 || y >= height as i32)
        {
            return Err(ForgeError::new(
                    ErrorCode::InvalidArgument,
                    format!("tap coordinate ({x}, {y}) is outside the current display {width}x{height}"),
                )
                .with_suggestion(
                    "Coordinates are real device pixels in the current display orientation. Do not scale them from an unrelated screenshot.",
                )
                .with_suggestion("Prefer semantic selectors such as --text or --resource-id.")
                .device(serial.to_string()));
        }
        self.adb.input_tap(serial, x as u32, y as u32).await?;
        Ok(display)
    }

    pub async fn long_press(&self, serial: &str, selector: &Selector) -> Result<TapResult> {
        self.long_press_with_duration(serial, selector, 900).await
    }

    pub async fn long_press_with_duration(
        &self,
        serial: &str,
        selector: &Selector,
        duration_ms: u64,
    ) -> Result<TapResult> {
        let selection = self.select(serial, selector).await?;
        self.long_press_selection(serial, selection, duration_ms)
            .await
    }

    /// Long-press a node from a previous observation after identity validation.
    pub async fn long_press_expected(
        &self,
        serial: &str,
        expected: &crate::reference::NodeExpectation,
        duration_ms: u64,
    ) -> Result<TapResult> {
        let hierarchy = self.dump(serial).await?;
        let node_id = expected.find(&hierarchy)?.id;
        self.long_press_selection(
            serial,
            Selection {
                hierarchy,
                indices: vec![node_id],
                selected: node_id,
            },
            duration_ms,
        )
        .await
    }

    async fn long_press_selection(
        &self,
        serial: &str,
        selection: Selection,
        duration_ms: u64,
    ) -> Result<TapResult> {
        let hierarchy = &selection.hierarchy;
        let node = selection.selected_node();
        let (target, ancestor) = self.tap_target(hierarchy, node)?;
        let (x, y) = target.bounds.center();
        let display = self.display_size(serial, hierarchy.rotation).await;
        self.validate_point(serial, x, y, display)?;
        if !target.enabled {
            return Err(ForgeError::new(
                ErrorCode::UiAssertionFailed,
                "the matched UI element is disabled",
            )
            .with_context(
                "element",
                serde_json::to_value(UiMatch::from_node(hierarchy, 0, target))
                    .unwrap_or(Value::Null),
            )
            .with_suggestion("Wait until the control is enabled or inspect the screen state.")
            .device(serial.to_string()));
        }
        self.adb
            .input_swipe(
                serial,
                x as u32,
                y as u32,
                x as u32,
                y as u32,
                duration_ms.max(1) as u32,
            )
            .await?;
        Ok(TapResult {
            matched: UiMatch::from_node(hierarchy, 0, node),
            tap_x: x,
            tap_y: y,
            used_clickable_ancestor: ancestor.is_some(),
            ancestor: ancestor.map(|node| UiMatch::from_node(hierarchy, 0, node)),
            display_size: display,
            rotation: hierarchy.rotation,
        })
    }

    /// Inject a raw motion event. Used by drag/long-press orchestration that
    /// needs holds and mid-gesture screenshots.
    pub async fn input_motion(&self, serial: &str, action: &str, x: i32, y: i32) -> Result<()> {
        self.adb
            .shell_args(
                serial,
                &[
                    "input".to_string(),
                    "motionevent".to_string(),
                    action.to_string(),
                    x.to_string(),
                    y.to_string(),
                ],
                Some(Duration::from_secs(15)),
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn swipe(
        &self,
        serial: &str,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        duration_ms: u32,
    ) -> Result<Option<(u32, u32)>> {
        let display = self.display_size(serial, None).await;
        self.validate_point(serial, x1, y1, display)?;
        self.validate_point(serial, x2, y2, display)?;
        self.adb
            .input_swipe(
                serial,
                x1 as u32,
                y1 as u32,
                x2 as u32,
                y2 as u32,
                duration_ms,
            )
            .await?;
        Ok(display)
    }

    /// Clear the focused field, or the field matched by `selector`.
    ///
    /// Unlike text entry, clearing with no selector should fail with a
    /// selector/focus explanation rather than a generic text-verification
    /// error: there is nothing to verify if no editable field is focused.
    pub async fn clear(&self, serial: &str, selector: Option<&Selector>) -> Result<TextResult> {
        if selector.is_none() {
            let hierarchy = self.dump(serial).await?;
            let focused = hierarchy
                .nodes
                .iter()
                .filter(|node| node.focused)
                .map(|node| UiMatch::from_node(&hierarchy, 0, node))
                .collect::<Vec<_>>();
            if !hierarchy
                .nodes
                .iter()
                .any(|node| node.focused && node.editable_like())
            {
                return Err(ForgeError::new(
                    ErrorCode::SelectorNotFound,
                    "no editable field is focused; refusing to clear an unknown target",
                )
                .with_context(
                    "focused_nodes",
                    serde_json::to_value(focused).unwrap_or(Value::Null),
                )
                .with_suggestion("Tap the field first, or pass --text, --resource-id, --content-desc or --class.")
                .with_suggestion("Run `androidforge ui focused` to inspect the focused node.")
                .device(serial.to_string()));
            }
        }
        self.text(serial, "", selector, true, false).await
    }

    pub async fn text(
        &self,
        serial: &str,
        text: &str,
        selector: Option<&Selector>,
        clear: bool,
        hide_keyboard: bool,
    ) -> Result<TextResult> {
        if let Some(selector) = selector {
            self.tap(serial, selector).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        self.text_impl(serial, text, clear, hide_keyboard, selector.is_some(), None)
            .await
    }

    /// Enter text into the node identified by a previous observation. The tap
    /// is dispatched only after identity validation, and the focused field's
    /// path and core identity must still match before any text is sent.
    pub async fn text_expected(
        &self,
        serial: &str,
        expected: &crate::reference::NodeExpectation,
        text: &str,
        clear: bool,
        hide_keyboard: bool,
    ) -> Result<TextResult> {
        let hierarchy = self.dump(serial).await?;
        let node = expected.find(&hierarchy)?;
        if !node.editable_like() {
            return Err(ForgeError::new(
                ErrorCode::ElementNotActionable,
                "the referenced element is not an editable text field",
            )
            .with_context_string("path", expected.path.clone())
            .with_suggestion("Choose an EditText/TextField ref from `ui observe --json`.")
            .device(serial.to_string()));
        }
        let selection = Selection {
            selected: node.id,
            indices: vec![node.id],
            hierarchy,
        };
        self.tap_selection(serial, selection).await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.text_impl(serial, text, clear, hide_keyboard, true, Some(expected))
            .await
    }

    /// Clear the field identified by a previous observation. Identity and
    /// editability are validated before the field is tapped.
    pub async fn clear_expected(
        &self,
        serial: &str,
        expected: &crate::reference::NodeExpectation,
    ) -> Result<TextResult> {
        self.text_expected(serial, expected, "", true, false).await
    }

    async fn text_impl(
        &self,
        serial: &str,
        text: &str,
        clear: bool,
        hide_keyboard: bool,
        targeted: bool,
        expected: Option<&crate::reference::NodeExpectation>,
    ) -> Result<TextResult> {
        let mut notes = Vec::new();
        let (hierarchy, focused_id, keyboard_visible) =
            self.wait_for_focused_field(serial, targeted).await?;
        let focused_node = hierarchy.nodes.get(focused_id).cloned().ok_or_else(|| {
            ForgeError::new(
                ErrorCode::TextVerificationFailed,
                "focused field disappeared",
            )
        })?;
        if let Some(expected) = expected
            && (focused_node.path != expected.path || !expected.matches_core(&focused_node))
        {
            return Err(ForgeError::new(
                ErrorCode::ElementIdentityChanged,
                "text input would go to a different field than the referenced element",
            )
            .with_context_string("expected_path", expected.path.clone())
            .with_context_string("focused_path", focused_node.path.clone())
            .with_context(
                "focused_node",
                serde_json::to_value(UiMatch::from_node(&hierarchy, 0, &focused_node))
                    .unwrap_or(Value::Null),
            )
            .with_suggestion(
                "The referenced field did not take focus. Observe again and choose a new ref.",
            )
            .with_suggestion("AndroidForge will not send text to an unverified field.")
            .device(serial.to_string()));
        }
        let mut before_text = focused_node.text.clone();
        if focused_node.text_is_hint() {
            if !before_text.as_deref().unwrap_or("").is_empty() {
                notes.push(
                    "the field exposes its hint/placeholder as text; treating it as empty before input"
                        .to_string(),
                );
            }
            before_text = None;
        }
        let mut cleared = false;
        let mut placeholder_unchanged = false;
        if clear {
            (cleared, placeholder_unchanged) = self
                .clear_focused_field(serial, &focused_node, &mut notes)
                .await?;
        }

        let encoded = escape_input_text(text).map_err(|message| {
            ForgeError::new(ErrorCode::InvalidArgument, message)
                .with_suggestion("Use `androidforge adb shell input ...` for text that the standard input command cannot represent.")
                .device(serial.to_string())
        })?;
        let mut output_notes = Vec::new();
        if !text.is_empty() {
            let output = self.adb.input_text(serial, &encoded).await?;
            let stderr = output.stderr_text();
            if !stderr.trim().is_empty() {
                output_notes.push(format!("input text stderr: {}", stderr.trim()));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        } else {
            cleared = true;
            notes.push("no text was sent because the requested text was empty".to_string());
        }

        let hierarchy_after = self.dump(serial).await?;
        let focused_after = hierarchy_after
            .nodes
            .iter()
            .filter(|node| node.focused)
            .collect::<Vec<_>>();
        let Some(focused_after) = focused_after.first() else {
            return Err(ForgeError::new(
                ErrorCode::TextVerificationFailed,
                "text was sent, but no UI field is focused afterward",
            )
            .with_context_string("requested_text", text.to_string())
            .with_suggestion("The field may have lost focus or the screen changed.")
            .with_suggestion("Run `androidforge ui dump` and inspect the current screen.")
            .device(serial.to_string()));
        };
        let after_text = focused_after.text.clone();
        let mut verified = false;
        if focused_after.password {
            notes.push(
                "field is a password field; text entry cannot be read back or verified".to_string(),
            );
        } else if self.text_verify {
            let expected = if clear {
                text.to_string()
            } else {
                format!("{}{}", before_text.clone().unwrap_or_default(), text)
            };
            let placeholder_after_clear = clear
                && text.is_empty()
                && (focused_after.text_is_placeholder_after_clear() || placeholder_unchanged);
            let actual = after_text.as_deref().unwrap_or("");
            if text_verification_matches(actual, text, &expected) || placeholder_after_clear {
                verified = true;
                if !clear && actual == text && !before_text.as_deref().unwrap_or("").is_empty() {
                    let note =
                        "the field replaced its existing/hint text; accepted the requested text";
                    if !notes.iter().any(|existing| existing == note) {
                        notes.push(note.to_string());
                    }
                }
                if placeholder_after_clear && !after_text.as_deref().unwrap_or("").is_empty() {
                    let note = "the field exposes its hint/placeholder as text after clearing; treating it as empty";
                    if !notes.iter().any(|existing| existing == note) {
                        notes.push(note.to_string());
                    }
                }
            } else {
                notes.extend(output_notes);
                return Err(ForgeError::new(
                    ErrorCode::TextVerificationFailed,
                    "the field text does not match the requested text after input",
                )
                .with_context_string("requested_text", text.to_string())
                .with_context_string(
                    "expected_text",
                    expected,
                )
                .with_context_string(
                    "actual_text",
                    after_text.clone().unwrap_or_default(),
                )
                .with_context(
                    "focused_node",
                    serde_json::to_value(UiMatch::from_node(&hierarchy_after, 0, focused_after))
                        .unwrap_or(Value::Null),
                )
                .with_context(
                    "note",
                    "The IME may have dropped unsupported characters, or the field transformed the input.",
                )
                .with_suggestion("Verify the text with `androidforge ui find --text ...`.")
                .with_suggestion("Use --clear when replacing existing field contents.")
                .with_suggestion("Non-ASCII text support depends on the device IME; inspect the focused field.")
                .device(serial.to_string()));
            }
        } else {
            notes.push("text verification was disabled with --no-verify".to_string());
        }

        let mut keyboard_hidden = false;
        if hide_keyboard {
            match self.adb.hide_keyboard(serial).await {
                Ok(_) => keyboard_hidden = true,
                Err(error) => notes.push(format!("could not hide keyboard: {}", error.message())),
            }
        }

        Ok(TextResult {
            text: text.to_string(),
            cleared,
            verified,
            focused: UiMatch::from_node(&hierarchy_after, 0, focused_after),
            before_text,
            after_text,
            keyboard_was_visible: keyboard_visible,
            keyboard_hidden,
            notes,
        })
    }

    pub async fn wait(
        &self,
        serial: &str,
        selector: &Selector,
        timeout: Duration,
        absent: bool,
    ) -> Result<WaitResult> {
        let started = Instant::now();
        let mut attempts = 0u32;
        let mut last_hierarchy = None;
        while started.elapsed() <= timeout || attempts == 0 {
            attempts += 1;
            let hierarchy = self.dump(serial).await?;
            let matches = find(&hierarchy, selector)?;
            let present = !matches.is_empty();
            if absent {
                if !present {
                    return Ok(WaitResult {
                        present: false,
                        matched: true,
                        attempts,
                        duration_ms: elapsed_ms(started),
                        matched_element: None,
                        hierarchy_package: hierarchy.package_name,
                    });
                }
            } else if let Some(first) = matches.first() {
                return Ok(WaitResult {
                    present: true,
                    matched: true,
                    attempts,
                    duration_ms: elapsed_ms(started),
                    matched_element: Some(first.clone()),
                    hierarchy_package: hierarchy.package_name,
                });
            }
            last_hierarchy = Some((hierarchy, matches));
            if started.elapsed() >= timeout {
                break;
            }
            tokio::time::sleep(self.poll_interval).await;
        }
        let (hierarchy, matches) = last_hierarchy.unwrap_or_else(|| {
            (
                UiHierarchy {
                    xml: String::new(),
                    rotation: None,
                    nodes: Vec::new(),
                    root: None,
                    package_name: None,
                },
                Vec::new(),
            )
        });
        if absent {
            Err(ForgeError::new(
                ErrorCode::UiAssertionFailed,
                "element still exists after waiting for its absence",
            )
            .with_context("selector", selector.describe())
            .with_context(
                "matches",
                serde_json::to_value(matches).unwrap_or(Value::Null),
            )
            .with_suggestion(
                "Inspect the current hierarchy and check whether the element is stale.",
            )
            .device(serial.to_string()))
        } else {
            let error = selector_not_found(&hierarchy, selector)
                .with_context("wait_attempts", Value::from(attempts))
                .with_context("wait_timeout_ms", Value::from(timeout.as_millis() as u64));
            Err(error)
        }
    }

    pub async fn assert(
        &self,
        serial: &str,
        selector: &Selector,
        timeout: Duration,
        absent: bool,
    ) -> Result<WaitResult> {
        self.wait(serial, selector, timeout, absent).await
    }

    /// Wait until none of the supplied strings appear as text or content
    /// description. Returns the lingering matches instead of failing, so the
    /// caller can choose wait-vs-assert semantics.
    pub async fn wait_texts_absent(
        &self,
        serial: &str,
        texts: &[String],
        timeout: Duration,
    ) -> Result<TextAbsenceResult> {
        let started = Instant::now();
        let mut attempts = 0u32;
        let mut last_package = None;
        let mut lingering = Vec::new();
        while started.elapsed() <= timeout || attempts == 0 {
            attempts += 1;
            let hierarchy = self.dump(serial).await?;
            last_package = hierarchy.package_name.clone();
            lingering = texts_visible_in(&hierarchy, texts);
            if lingering.is_empty() {
                return Ok(TextAbsenceResult {
                    absent: true,
                    attempts,
                    duration_ms: elapsed_ms(started),
                    lingering,
                    hierarchy_package: last_package,
                });
            }
            if started.elapsed() >= timeout {
                break;
            }
            tokio::time::sleep(self.poll_interval).await;
        }
        Ok(TextAbsenceResult {
            absent: false,
            attempts,
            duration_ms: elapsed_ms(started),
            lingering,
            hierarchy_package: last_package,
        })
    }

    /// Wait for the node identified by a previous observation. Identity is
    /// re-checked on every poll; a changed node never proves absence.
    pub async fn wait_expected(
        &self,
        serial: &str,
        expected: &crate::reference::NodeExpectation,
        timeout: Duration,
        absent: bool,
    ) -> Result<WaitResult> {
        let started = Instant::now();
        let mut attempts = 0u32;
        let mut last_package = None;
        let mut last_identity_changed = false;
        let mut last_element = None;
        while started.elapsed() <= timeout || attempts == 0 {
            attempts += 1;
            let hierarchy = self.dump(serial).await?;
            last_package = hierarchy.package_name.clone();
            let at_path = hierarchy
                .nodes
                .iter()
                .find(|node| node.path == expected.path);
            if absent && at_path.is_none() {
                return Ok(WaitResult {
                    present: false,
                    matched: true,
                    attempts,
                    duration_ms: elapsed_ms(started),
                    matched_element: None,
                    hierarchy_package: hierarchy.package_name,
                });
            }
            if let Some(node) = at_path {
                if !absent && expected.matches_identity(node) {
                    return Ok(WaitResult {
                        present: true,
                        matched: true,
                        attempts,
                        duration_ms: elapsed_ms(started),
                        matched_element: Some(UiMatch::from_node(&hierarchy, 0, node)),
                        hierarchy_package: hierarchy.package_name,
                    });
                }
                last_identity_changed = !expected.matches_identity(node);
                last_element = Some(UiMatch::from_node(&hierarchy, 0, node));
            }
            if started.elapsed() >= timeout {
                break;
            }
            tokio::time::sleep(self.poll_interval).await;
        }
        let mut error = if last_identity_changed {
            ForgeError::new(
                ErrorCode::ElementIdentityChanged,
                "the element at the referenced path changed identity while waiting",
            )
            .with_context(
                "observed",
                serde_json::to_value(expected).unwrap_or(Value::Null),
            )
            .with_context(
                "current",
                serde_json::to_value(last_element.clone()).unwrap_or(Value::Null),
            )
            .with_suggestion(
                "Run `androidforge ui observe --session <ID> --json` and choose a new ref.",
            )
            .with_suggestion("AndroidForge never treats a changed node as the observed node.")
        } else if absent {
            ForgeError::new(
                ErrorCode::UiAssertionFailed,
                "the referenced element still exists after waiting for its absence",
            )
            .with_context("ref_path", expected.path.clone())
            .with_context(
                "matches",
                serde_json::to_value(last_element.clone()).unwrap_or(Value::Null),
            )
            .with_suggestion(
                "Inspect the current hierarchy and check whether the element is stale.",
            )
        } else {
            ForgeError::new(
                ErrorCode::UiAssertionFailed,
                "the referenced element did not reach its recorded identity before the timeout",
            )
            .with_context("ref_path", expected.path.clone())
            .with_context(
                "matches",
                serde_json::to_value(last_element.clone()).unwrap_or(Value::Null),
            )
            .with_suggestion(
                "Run `androidforge ui observe --session <ID> --json` and choose a new ref.",
            )
        };
        error = error
            .with_context("wait_attempts", Value::from(attempts))
            .with_context("wait_timeout_ms", Value::from(timeout.as_millis() as u64))
            .with_context(
                "hierarchy_package",
                serde_json::to_value(last_package).unwrap_or(Value::Null),
            )
            .device(serial.to_string());
        Err(error)
    }
    pub async fn settle(&self, serial: &str, timeout: Duration) -> Result<SettleResult> {
        let started = Instant::now();
        let mut attempts = 0u32;
        let mut previous: Option<String> = None;
        loop {
            attempts += 1;
            let hierarchy = self.dump(serial).await?;
            let xml = hierarchy.xml.clone();
            if previous.as_deref() == Some(xml.as_str()) {
                return Ok(SettleResult {
                    stable: true,
                    attempts,
                    duration_ms: elapsed_ms(started),
                    xml_sha256: androidforge_core::util::sha256_hex(xml.as_bytes()),
                });
            }
            previous = Some(xml);
            if started.elapsed() >= timeout {
                return Err(ForgeError::new(
                    ErrorCode::UiAssertionFailed,
                    "UI hierarchy did not settle before the timeout",
                )
                .with_suggestion(
                    "Increase the timeout or inspect the current screen for ongoing animation.",
                )
                .device(serial.to_string()));
            }
            tokio::time::sleep(self.settle_interval).await;
        }
    }

    /// Wait until the hierarchy has stayed byte-identical for `stable_for`.
    pub async fn settle_for(
        &self,
        serial: &str,
        stable_for: Duration,
        timeout: Duration,
    ) -> Result<SettleResult> {
        let started = Instant::now();
        let mut attempts = 0u32;
        let mut last_hash: Option<String> = None;
        let mut stable_since = Instant::now();
        loop {
            attempts += 1;
            let hierarchy = self.dump(serial).await?;
            let hash = androidforge_core::util::sha256_hex(hierarchy.xml.as_bytes());
            if last_hash.as_deref() != Some(hash.as_str()) {
                last_hash = Some(hash);
                stable_since = Instant::now();
            } else if stable_since.elapsed() >= stable_for {
                return Ok(SettleResult {
                    stable: true,
                    attempts,
                    duration_ms: elapsed_ms(started),
                    xml_sha256: last_hash.unwrap_or_default(),
                });
            }
            if started.elapsed() >= timeout {
                return Err(ForgeError::new(
                    ErrorCode::UiAssertionFailed,
                    "UI hierarchy did not remain stable before the timeout",
                )
                .with_context(
                    "stable_for_ms",
                    serde_json::json!(stable_for.as_millis() as u64),
                )
                .with_suggestion("Increase --stable-for or --timeout for slow animations.")
                .device(serial.to_string()));
            }
            tokio::time::sleep(self.settle_interval).await;
        }
    }

    /// Wait until the same selector resolves to identical bounds for
    /// `stable_for` within `timeout`.
    ///
    /// Every poll re-resolves the selector through [`Self::bounds`], so a lost
    /// element (`E_SELECTOR_NOT_FOUND`) or an ambiguous one
    /// (`E_SELECTOR_AMBIGUOUS`) fails closed instead of being treated as
    /// stable. A change in bounds restarts the stability window. This method
    /// only reads the hierarchy; no input is dispatched.
    pub async fn wait_for_bounds_stable(
        &self,
        serial: &str,
        selector: &Selector,
        stable_for: Duration,
        timeout: Duration,
    ) -> Result<BoundsStabilityResult> {
        selector.validate()?;
        let started = Instant::now();
        let mut tracker = BoundsStabilityTracker::new();
        loop {
            let bounds = self.bounds(serial, selector).await?;
            let now = Instant::now();
            match bounds_stability_step(&mut tracker, bounds, now, started, stable_for, timeout) {
                StabilityStep::Stable => {
                    return Ok(BoundsStabilityResult {
                        stable: true,
                        bounds,
                        attempts: tracker.attempts,
                        duration_ms: elapsed_ms(started),
                        stable_for_ms: stable_for.as_millis().min(u128::from(u64::MAX)) as u64,
                    });
                }
                StabilityStep::TimedOut => {
                    return Err(ForgeError::new(
                        ErrorCode::UiAssertionFailed,
                        "UI element bounds did not remain stable before the timeout",
                    )
                    .with_context("selector", selector.describe())
                    .with_context(
                        "last_bounds",
                        serde_json::to_value(bounds).unwrap_or(Value::Null),
                    )
                    .with_context(
                        "stable_for_ms",
                        serde_json::json!(stable_for.as_millis().min(u128::from(u64::MAX)) as u64),
                    )
                    .with_context(
                        "timeout_ms",
                        serde_json::json!(timeout.as_millis().min(u128::from(u64::MAX)) as u64),
                    )
                    .with_context("attempts", serde_json::json!(tracker.attempts))
                    .with_suggestion("Increase the timeout or wait for animations to finish.")
                    .device(serial.to_string()));
                }
                StabilityStep::Continue => {}
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }

    pub async fn focused(&self, serial: &str) -> Result<Vec<UiMatch>> {
        let hierarchy = self.dump(serial).await?;
        Ok(hierarchy
            .focused()
            .into_iter()
            .enumerate()
            .map(|(index, node)| UiMatch::from_node(&hierarchy, index, node))
            .collect())
    }

    pub async fn scroll(
        &self,
        serial: &str,
        direction: &str,
        selector: Option<&Selector>,
    ) -> Result<ScrollResult> {
        let hierarchy = self.dump(serial).await?;
        let selected_scrollable = if let Some(selector) = selector {
            let selection = select(hierarchy.clone(), selector)?;
            let node = selection.selected_node();
            if node.scrollable {
                Some(node.clone())
            } else {
                hierarchy
                    .clickable_ancestor(node)
                    .filter(|ancestor| ancestor.scrollable)
                    .cloned()
                    .or_else(|| {
                        hierarchy
                            .nodes
                            .iter()
                            .filter(|candidate| candidate.scrollable && candidate.visible())
                            .max_by_key(|candidate| candidate.bounds.area())
                            .cloned()
                    })
            }
        } else {
            hierarchy
                .nodes
                .iter()
                .filter(|node| node.scrollable && node.visible())
                .max_by_key(|node| node.bounds.area())
                .cloned()
        };
        let display = self.display_size(serial, hierarchy.rotation).await;
        let (target, used_node) = if let Some(node) = selected_scrollable {
            (node.bounds, true)
        } else {
            let (width, height) = display.unwrap_or((1080, 2400));
            (
                Rect {
                    left: 0,
                    top: 0,
                    right: width as i32,
                    bottom: height as i32,
                },
                false,
            )
        };
        let (x1, y1, x2, y2) = scroll_points(target, direction)?;
        if [x1, y1, x2, y2].iter().any(|value| *value < 0) {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                "scroll bounds produced a negative coordinate",
            )
            .with_suggestion("Re-observe the screen; the node bounds are malformed.")
            .device(serial.to_string()));
        }
        self.adb
            .input_swipe(serial, x1 as u32, y1 as u32, x2 as u32, y2 as u32, 350)
            .await?;
        Ok(ScrollResult {
            direction: direction.to_string(),
            start_x: x1,
            start_y: y1,
            end_x: x2,
            end_y: y2,
            target_bounds: target,
            used_scrollable_node: used_node,
        })
    }

    /// Scroll inside the node identified by a previous observation. Unlike the
    /// selector path, there is no fallback to an unrelated scrollable node: the
    /// referenced node itself must be scrollable.
    pub async fn scroll_expected(
        &self,
        serial: &str,
        direction: &str,
        expected: &crate::reference::NodeExpectation,
    ) -> Result<ScrollResult> {
        let hierarchy = self.dump(serial).await?;
        let node = expected.find(&hierarchy)?;
        if !node.scrollable {
            return Err(ForgeError::new(
                ErrorCode::ElementNotActionable,
                "the referenced element is not scrollable",
            )
            .with_context_string("path", expected.path.clone())
            .with_suggestion("Choose a scrollable container ref from `ui observe --json`.")
            .device(serial.to_string()));
        }
        let target = node.bounds;
        let (x1, y1, x2, y2) = scroll_points(target, direction)?;
        if [x1, y1, x2, y2].iter().any(|value| *value < 0) {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                "scroll bounds produced a negative coordinate",
            )
            .with_suggestion("Re-observe the screen; the node bounds are malformed.")
            .device(serial.to_string()));
        }
        self.adb
            .input_swipe(serial, x1 as u32, y1 as u32, x2 as u32, y2 as u32, 350)
            .await?;
        Ok(ScrollResult {
            direction: direction.to_string(),
            start_x: x1,
            start_y: y1,
            end_x: x2,
            end_y: y2,
            target_bounds: target,
            used_scrollable_node: true,
        })
    }
    pub async fn scroll_to(
        &self,
        serial: &str,
        selector: &Selector,
        direction: &str,
        max_swipes: u32,
        timeout: Duration,
    ) -> Result<ScrollToResult> {
        selector.validate()?;
        let started = Instant::now();
        let mut attempts = 0u32;
        let mut last_hierarchy = None;
        let mut notes = Vec::new();
        while attempts <= max_swipes && started.elapsed() < timeout {
            let hierarchy = self.dump(serial).await?;
            let matches = find(&hierarchy, selector)?;
            if let Some(first) = matches.first() {
                return Ok(ScrollToResult {
                    found: true,
                    swipes: attempts,
                    matched_element: Some(first.clone()),
                    notes,
                });
            }
            if attempts >= max_swipes {
                last_hierarchy = Some(hierarchy);
                break;
            }
            last_hierarchy = Some(hierarchy);
            let scroll = self.scroll(serial, direction, None).await?;
            if !scroll.used_scrollable_node {
                notes.push("no scrollable container was found; used the display area".to_string());
            }
            attempts += 1;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let hierarchy = last_hierarchy.unwrap_or_else(|| UiHierarchy {
            xml: String::new(),
            rotation: None,
            nodes: Vec::new(),
            root: None,
            package_name: None,
        });
        let mut error = if hierarchy.nodes.is_empty() {
            ForgeError::new(
                ErrorCode::SelectorNotFound,
                "selector was not found before scrolling exhausted",
            )
            .with_suggestion("Run `androidforge ui dump` and inspect the current hierarchy.")
        } else {
            selector_not_found(&hierarchy, selector)
        };
        error = error
            .with_context_string("scroll_direction", direction)
            .with_context_string("swipes", attempts.to_string());
        Err(error)
    }

    pub async fn back(&self, serial: &str, hide_keyboard: bool) -> Result<BackResult> {
        let keyboard_was_visible = self.adb.keyboard_shown(serial).await;
        let mut keyboard_hidden = false;
        if keyboard_was_visible == Some(true) {
            if !hide_keyboard {
                return Err(ForgeError::new(
                    ErrorCode::ImeVisible,
                    "the soft keyboard is visible; refusing to send Back because it would dismiss the keyboard instead of navigating",
                )
                .with_suggestion("Run `androidforge ui hide-keyboard` first.")
                .with_suggestion("Or pass --hide-keyboard to dismiss the IME and then press Back.")
                .device(serial.to_string()));
            }
            self.adb.hide_keyboard(serial).await?;
            if self.adb.keyboard_shown(serial).await == Some(true) {
                return Err(ForgeError::new(
                    ErrorCode::ImeVisible,
                    "the soft keyboard is still visible after attempting to hide it",
                )
                .with_suggestion("Dismiss the keyboard manually or retry.")
                .device(serial.to_string()));
            }
            keyboard_hidden = true;
        }
        self.adb.keyevent(serial, "KEYCODE_BACK", None).await?;
        Ok(BackResult {
            keyboard_was_visible,
            keyboard_hidden,
            back_sent: true,
        })
    }

    pub async fn home(&self, serial: &str) -> Result<KeyResult> {
        self.adb.keyevent(serial, "KEYCODE_HOME", None).await?;
        Ok(KeyResult {
            keycode: "KEYCODE_HOME".to_string(),
        })
    }

    pub async fn keyevent(&self, serial: &str, keycode: &str) -> Result<KeyResult> {
        self.adb.keyevent(serial, keycode, None).await?;
        Ok(KeyResult {
            keycode: keycode.to_string(),
        })
    }

    pub async fn hide_keyboard(&self, serial: &str) -> Result<bool> {
        self.adb.hide_keyboard(serial).await
    }

    async fn tap_selection(&self, serial: &str, selection: Selection) -> Result<TapResult> {
        let hierarchy = &selection.hierarchy;
        let node = selection.selected_node();
        let (target, ancestor) = self.tap_target(hierarchy, node)?;
        let (x, y) = target.bounds.center();
        let display = self.display_size(serial, hierarchy.rotation).await;
        self.validate_point(serial, x, y, display)?;
        if !target.enabled {
            return Err(ForgeError::new(
                ErrorCode::UiAssertionFailed,
                "the matched UI element is disabled",
            )
            .with_context(
                "element",
                serde_json::to_value(UiMatch::from_node(hierarchy, 0, target))
                    .unwrap_or(Value::Null),
            )
            .with_suggestion("Wait until the control is enabled or inspect the screen state.")
            .device(serial.to_string()));
        }
        self.adb.input_tap(serial, x as u32, y as u32).await?;
        Ok(TapResult {
            matched: UiMatch::from_node(hierarchy, 0, node),
            tap_x: x,
            tap_y: y,
            used_clickable_ancestor: ancestor.is_some(),
            ancestor: ancestor.map(|node| UiMatch::from_node(hierarchy, 0, node)),
            display_size: display,
            rotation: hierarchy.rotation,
        })
    }

    fn tap_target<'a>(
        &self,
        hierarchy: &'a UiHierarchy,
        node: &'a UiNode,
    ) -> Result<(&'a UiNode, Option<&'a UiNode>)> {
        if node.visible() && node.clickable {
            return Ok((node, None));
        }
        if self.tap_clickable_ancestor
            && let Some(ancestor) = hierarchy.clickable_ancestor(node)
            && ancestor.visible()
        {
            return Ok((ancestor, Some(ancestor)));
        }
        if node.visible() {
            return Ok((node, None));
        }
        Err(ForgeError::new(
            ErrorCode::SelectorNotFound,
            "the matched element has no tappable bounds",
        )
        .with_context(
            "element",
            serde_json::to_value(UiMatch::from_node(hierarchy, 0, node)).unwrap_or(Value::Null),
        )
        .with_suggestion("Use `androidforge ui dump` to inspect the current bounds.")
        .with_suggestion("Use a clickable ancestor selector or --x/--y as a fallback."))
    }

    async fn display_size(&self, serial: &str, rotation: Option<i32>) -> Option<(u32, u32)> {
        let size = self.adb.display_size(serial).await?;
        if matches!(rotation, Some(1) | Some(3)) {
            Some((size.1, size.0))
        } else {
            Some(size)
        }
    }

    fn validate_point(
        &self,
        serial: &str,
        x: i32,
        y: i32,
        display: Option<(u32, u32)>,
    ) -> Result<()> {
        if let Some((width, height)) = display
            && (x < 0 || y < 0 || x >= width as i32 || y >= height as i32)
        {
            return Err(ForgeError::new(
                    ErrorCode::InvalidArgument,
                    format!(
                        "coordinate ({x}, {y}) is outside the current display {width}x{height}"
                    ),
                )
                .with_suggestion(
                    "Coordinates are real device pixels in the current display orientation. Never scale from an unrelated screenshot.",
                )
                .device(serial.to_string()));
        }
        Ok(())
    }

    async fn wait_for_focused_field(
        &self,
        serial: &str,
        targeted: bool,
    ) -> Result<(UiHierarchy, usize, Option<bool>)> {
        let started = Instant::now();
        let timeout = Duration::from_secs(5);
        let mut last_focused = Vec::new();
        while started.elapsed() <= timeout {
            let hierarchy = self.dump(serial).await?;
            let keyboard = self.adb.keyboard_shown(serial).await;
            let focused_ids = hierarchy
                .nodes
                .iter()
                .filter(|node| node.focused)
                .map(|node| node.id)
                .collect::<Vec<_>>();
            if focused_ids.is_empty() {
                last_focused = Vec::new();
            } else {
                last_focused = focused_ids
                    .iter()
                    .map(|id| UiMatch::from_node(&hierarchy, 0, &hierarchy.nodes[*id]))
                    .collect();
                let editable = focused_ids
                    .iter()
                    .copied()
                    .find(|id| hierarchy.nodes[*id].editable_like());
                let chosen = editable.or_else(|| {
                    if focused_ids.len() == 1
                        && (keyboard == Some(true)
                            || (targeted && hierarchy.nodes[focused_ids[0]].clickable))
                    {
                        Some(focused_ids[0])
                    } else {
                        None
                    }
                });
                if let Some(id) = chosen {
                    return Ok((hierarchy, id, keyboard));
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let mut error = ForgeError::new(
            ErrorCode::TextVerificationFailed,
            "no editable field appears to be focused; refusing to send text to an unknown target",
        )
        .with_context(
            "focused_nodes",
            serde_json::to_value(last_focused).unwrap_or(Value::Null),
        )
        .with_suggestion("Tap the intended field first, or pass --selector with --text.")
        .with_suggestion("Run `androidforge ui dump` and inspect the focused node.")
        .with_suggestion(
            "The soft keyboard may be hidden or the field may use nonstandard semantics.",
        )
        .device(serial.to_string());
        if !targeted {
            error = error.with_suggestion("Use `androidforge ui text <text> --selector ...`.");
        }
        Err(error)
    }

    async fn clear_focused_field(
        &self,
        serial: &str,
        node: &UiNode,
        notes: &mut Vec<String>,
    ) -> Result<(bool, bool)> {
        let Some(current) = node.text.clone() else {
            return Ok((false, false));
        };
        if current.is_empty() {
            return Ok((false, false));
        }
        if node.text_is_hint() {
            notes.push(
                "the field only contains its hint/placeholder; no deletion was necessary"
                    .to_string(),
            );
            return Ok((false, true));
        }
        self.adb.keyevent(serial, "KEYCODE_MOVE_END", None).await?;
        let delete_count = if node.password {
            current.chars().count().clamp(64, 300)
        } else {
            current.chars().count().min(300)
        };
        let mut remaining = delete_count;
        while remaining > 0 {
            let batch = remaining.min(80);
            let mut args = vec!["input".to_string(), "keyevent".to_string()];
            for _ in 0..batch {
                args.push("KEYCODE_DEL".to_string());
            }
            self.adb
                .shell_args(serial, &args, Some(Duration::from_secs(20)), None)
                .await?;
            remaining -= batch;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let after = self.dump(serial).await?;
        let focused_after = after.nodes.iter().find(|candidate| candidate.focused);
        if node.password {
            notes.push("password field cannot be read back after clearing".to_string());
            return Ok((true, true));
        }
        if let Some(after_node) = focused_after {
            if after_node.text_is_placeholder_after_clear() {
                if !after_node.text.as_deref().unwrap_or("").is_empty() {
                    notes.push(
                        "the field exposes its hint/placeholder as text after clearing; treating it as empty"
                            .to_string(),
                    );
                }
                return Ok((true, true));
            }
            // Some apps expose a placeholder only as `text` and ignore the
            // delete keys. If the text is byte-identical after clearing, treat
            // it as a placeholder; the subsequent input verification still
            // fails if the field was really holding user text.
            if after_node.text.as_deref().unwrap_or("").is_empty() {
                return Ok((true, true));
            }
            self.adb.keyevent(serial, "KEYCODE_MOVE_END", None).await?;
            self.adb.keyevent(serial, "KEYCODE_DEL", None).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
            let probe = self.dump(serial).await?;
            let probe_node = probe.nodes.iter().find(|candidate| candidate.focused);
            let Some(probe_node) = probe_node else {
                return Err(ForgeError::new(
                    ErrorCode::TextVerificationFailed,
                    "focus was lost during the clear probe",
                )
                .with_suggestion("Tap the field again and retry.")
                .device(serial.to_string()));
            };
            if probe_node.path != node.path
                || probe_node.class != node.class
                || probe_node.resource_id != node.resource_id
            {
                return Err(ForgeError::new(
                    ErrorCode::TextVerificationFailed,
                    "the focused field changed during the clear probe",
                )
                .with_suggestion("Tap the intended field again and retry.")
                .device(serial.to_string()));
            }
            let probe_text = probe_node.text.clone();
            if probe_text == after_node.text {
                notes.push(
                    "a probe delete did not change the remaining text; treating it as a placeholder"
                        .to_string(),
                );
                return Ok((true, true));
            }
            if probe_text.as_deref().unwrap_or("").is_empty() {
                notes.push("a probe delete cleared the remaining text".to_string());
                return Ok((true, true));
            }
            return Err(ForgeError::new(
                ErrorCode::TextVerificationFailed,
                "the field still contains text after the clear operation",
            )
            .with_context(
                "element",
                serde_json::to_value(UiMatch::from_node(&after, 0, after_node)).unwrap_or(Value::Null),
            )
            .with_suggestion("The field may have a custom editor or the deletes were intercepted by the IME.")
            .with_suggestion("Re-run the operation after tapping the field; use --no-verify only when necessary.")
            .device(serial.to_string()));
        }
        Err(ForgeError::new(
            ErrorCode::TextVerificationFailed,
            "focus was lost while clearing the field",
        )
        .with_suggestion("Tap the field again and retry.")
        .device(serial.to_string()))
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

/// Resolve a selector against an already captured hierarchy in one step.
///
/// Delegates to [`select`] so ambiguity, `index` and not-found handling stay
/// identical to every other UI surface; only the selected node's `Rect` is
/// converted into the shared [`Bounds`] shape.
fn bounds_from_hierarchy(hierarchy: &UiHierarchy, selector: &Selector) -> Result<Bounds> {
    let selection = select(hierarchy.clone(), selector)?;
    Ok(Bounds::from(selection.selected_node().bounds))
}

/// Compare `bottom_y` to `expected` with an absolute tolerance.
pub fn compare_bottom_y(actual: i32, expected: i32, tolerance: i32) -> BoundsComparison {
    let tolerance = tolerance.max(0);
    let delta = (i64::from(actual) - i64::from(expected)).abs();
    BoundsComparison {
        relation: BoundsRelation::Equal,
        expected,
        actual,
        tolerance,
        passed: delta <= i64::from(tolerance),
    }
}

/// Compare `bottom_y > expected + tolerance` (strictly beyond the dead band).
pub fn compare_bottom_y_gt(actual: i32, expected: i32, tolerance: i32) -> BoundsComparison {
    let tolerance = tolerance.max(0);
    BoundsComparison {
        relation: BoundsRelation::GreaterThan,
        expected,
        actual,
        tolerance,
        passed: i64::from(actual) > i64::from(expected) + i64::from(tolerance),
    }
}

/// Compare `bottom_y < expected - tolerance` (strictly beyond the dead band).
pub fn compare_bottom_y_lt(actual: i32, expected: i32, tolerance: i32) -> BoundsComparison {
    let tolerance = tolerance.max(0);
    BoundsComparison {
        relation: BoundsRelation::LessThan,
        expected,
        actual,
        tolerance,
        passed: i64::from(actual) < i64::from(expected) - i64::from(tolerance),
    }
}

/// Tracks whether one bounds value has been observed continuously.
#[derive(Debug, Clone)]
struct BoundsStabilityTracker {
    bounds: Option<Bounds>,
    stable_since: Option<Instant>,
    attempts: u32,
}

impl BoundsStabilityTracker {
    fn new() -> Self {
        Self {
            bounds: None,
            stable_since: None,
            attempts: 0,
        }
    }

    /// Record one observation. Returns true only when the bounds have stayed
    /// identical for at least `stable_for` as of `now`.
    fn observe(&mut self, bounds: Bounds, now: Instant, stable_for: Duration) -> bool {
        self.attempts += 1;
        if self.bounds != Some(bounds) {
            self.bounds = Some(bounds);
            self.stable_since = Some(now);
            return false;
        }
        self.stable_since
            .is_some_and(|since| now.duration_since(since) >= stable_for)
    }
}

/// Outcome of one poll of the bounds-stability loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StabilityStep {
    Stable,
    Continue,
    TimedOut,
}

/// Pure policy for one bounds-stability poll, kept separate from ADB polling
/// so success and timeout behaviour can be tested without a device.
fn bounds_stability_step(
    tracker: &mut BoundsStabilityTracker,
    bounds: Bounds,
    now: Instant,
    started: Instant,
    stable_for: Duration,
    timeout: Duration,
) -> StabilityStep {
    if tracker.observe(bounds, now, stable_for) {
        return StabilityStep::Stable;
    }
    if now.duration_since(started) >= timeout {
        return StabilityStep::TimedOut;
    }
    StabilityStep::Continue
}

fn text_verification_matches(actual: &str, requested: &str, expected: &str) -> bool {
    actual == expected || (!requested.is_empty() && actual == requested)
}

/// Find visible nodes whose text or content description contains one of the
/// requested strings.
fn texts_visible_in(hierarchy: &UiHierarchy, texts: &[String]) -> Vec<UiMatch> {
    let mut matches = Vec::new();
    for node in hierarchy.nodes.iter().filter(|node| node.visible()) {
        let text = node.text.as_deref().unwrap_or_default();
        let desc = node.content_desc.as_deref().unwrap_or_default();
        let hit = texts.iter().any(|wanted| {
            !wanted.is_empty() && (text.contains(wanted.as_str()) || desc.contains(wanted.as_str()))
        });
        if hit {
            matches.push(UiMatch::from_node(hierarchy, matches.len(), node));
        }
    }
    matches
}

fn scroll_points(bounds: Rect, direction: &str) -> Result<(i32, i32, i32, i32)> {
    let width = bounds.width();
    let height = bounds.height();
    if width < 20 || height < 20 {
        return Err(ForgeError::new(
            ErrorCode::UiAssertionFailed,
            "scroll target has no usable bounds",
        ));
    }
    let cx = bounds.left + width / 2;
    let cy = bounds.top + height / 2;
    let inset_x = (width as f64 * 0.18) as i32;
    let inset_y = (height as f64 * 0.18) as i32;
    Ok(match direction {
        "down" => (cx, bounds.bottom - inset_y, cx, bounds.top + inset_y),
        "up" => (cx, bounds.top + inset_y, cx, bounds.bottom - inset_y),
        "right" => (bounds.left + inset_x, cy, bounds.right - inset_x, cy),
        "left" => (bounds.right - inset_x, cy, bounds.left + inset_x, cy),
        _ => {
            return Err(ForgeError::invalid_argument(format!(
                "unsupported scroll direction '{direction}'; expected up, down, left or right"
            )));
        }
    })
}

fn is_transient_uiautomator_failure(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    if lower.contains("not found") || lower.contains("no such file") {
        return false;
    }
    lower.contains("exit code 137")
        || lower.contains("could not get idle state")
        || lower.contains("null root node")
        || lower.contains("empty or partial xml")
        || lower.contains("empty hierarchy")
        || (lower.contains("uiautomator")
            && (lower.contains("killed")
                || lower.contains("timeout")
                || lower.contains("timed out")
                || lower.contains("not responding")))
}

async fn recover_uiautomator(adb: &AdbClient, serial: &str) {
    // Best effort: a killed or wedged uiautomator helper is process-local to
    // this session's device. Android's toolbox pkill is available on all
    // supported API levels.
    let _ = adb
        .shell_args(
            serial,
            &[
                "pkill".to_string(),
                "-9".to_string(),
                "-f".to_string(),
                "uiautomator".to_string(),
            ],
            Some(Duration::from_secs(5)),
            None,
        )
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_uiautomator_failures_are_recognized() {
        assert!(is_transient_uiautomator_failure(
            "ADB command failed with exit code 137"
        ));
        assert!(is_transient_uiautomator_failure(
            "ERROR: could not get idle state"
        ));
        assert!(is_transient_uiautomator_failure("ERROR: null root node"));
        assert!(is_transient_uiautomator_failure(
            "uiautomator produced empty or partial XML"
        ));
        assert!(is_transient_uiautomator_failure(
            "uiautomator returned an empty hierarchy"
        ));
        assert!(!is_transient_uiautomator_failure(
            "uiautomator: executable not found"
        ));
    }

    #[test]
    fn scroll_points_respect_direction() {
        let bounds = Rect {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 2000,
        };
        let (_, y1, _, y2) = scroll_points(bounds, "down").unwrap();
        assert!(y1 > y2);
        let (_, y1, _, y2) = scroll_points(bounds, "up").unwrap();
        assert!(y1 < y2);
    }

    #[test]
    fn text_verification_accepts_replaced_placeholder() {
        assert!(text_verification_matches("query", "query", "Searchquery"));
        assert!(text_verification_matches(
            "Searchquery",
            "query",
            "Searchquery"
        ));
        assert!(!text_verification_matches("other", "query", "Searchquery"));
        assert!(!text_verification_matches("", "", "Search"));
    }

    const BOUNDS_XML: &str = r#"<hierarchy rotation="0">
   <node index="0" text="Save" resource-id="com.example:id/save" class="android.widget.Button" package="com.example" content-desc="Save changes" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[100,200][300,300]" />
   <node index="1" text="Save" resource-id="com.example:id/save2" class="android.widget.Button" package="com.example" content-desc="Save draft" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[100,400][300,500]" />
 </hierarchy>"#;

    #[test]
    fn bounds_snapshot_derives_edges_size_and_center() {
        let bounds = Bounds::from(Rect {
            left: 100,
            top: 200,
            right: 300,
            bottom: 340,
        });
        assert_eq!(bounds.left, 100);
        assert_eq!(bounds.top, 200);
        assert_eq!(bounds.right, 300);
        assert_eq!(bounds.bottom, 340);
        assert_eq!(bounds.width, 200);
        assert_eq!(bounds.height, 140);
        assert_eq!(bounds.center_x, 200);
        assert_eq!(bounds.center_y, 270);
        assert_eq!(bounds.bottom_y(), 340);
    }

    #[test]
    fn bounds_resolution_reuses_selectors_and_fails_closed_on_ambiguity() {
        let hierarchy = UiHierarchy::parse(BOUNDS_XML).unwrap();
        let ambiguous = Selector {
            text: Some("Save".to_string()),
            ..Selector::default()
        };
        let error = bounds_from_hierarchy(&hierarchy, &ambiguous).unwrap_err();
        assert_eq!(error.code(), ErrorCode::SelectorAmbiguous);

        let missing = Selector {
            text: Some("Nope".to_string()),
            ..Selector::default()
        };
        let error = bounds_from_hierarchy(&hierarchy, &missing).unwrap_err();
        assert_eq!(error.code(), ErrorCode::SelectorNotFound);

        let exact = Selector {
            resource_id: Some("com.example:id/save".to_string()),
            ..Selector::default()
        };
        let bounds = bounds_from_hierarchy(&hierarchy, &exact).unwrap();
        assert_eq!(bounds.bottom_y(), 300);
        assert_eq!(bounds.center_y, 250);
    }

    #[test]
    fn bottom_y_comparison_covers_exact_tolerance_gt_and_lt() {
        let exact = compare_bottom_y(300, 300, 0);
        assert_eq!(exact.relation, BoundsRelation::Equal);
        assert_eq!(exact.expected, 300);
        assert_eq!(exact.actual, 300);
        assert_eq!(exact.tolerance, 0);
        assert!(exact.passed);
        assert!(!compare_bottom_y(301, 300, 0).passed);
        assert!(compare_bottom_y(304, 300, 5).passed);
        assert!(!compare_bottom_y(306, 300, 5).passed);
        // Negative tolerances are clamped, never inverted.
        assert!(!compare_bottom_y(301, 300, -5).passed);

        let gt = compare_bottom_y_gt(306, 300, 5);
        assert_eq!(gt.relation, BoundsRelation::GreaterThan);
        assert!(gt.passed);
        assert!(!compare_bottom_y_gt(305, 300, 5).passed);
        assert!(!compare_bottom_y_gt(300, 300, 5).passed);

        let lt = compare_bottom_y_lt(294, 300, 5);
        assert_eq!(lt.relation, BoundsRelation::LessThan);
        assert!(lt.passed);
        assert!(!compare_bottom_y_lt(295, 300, 5).passed);
        assert!(!compare_bottom_y_lt(306, 300, 5).passed);
    }

    #[test]
    fn failed_bottom_y_comparison_uses_ui_assertion_failed_with_context() {
        let error = compare_bottom_y(320, 300, 2).require_passed().unwrap_err();
        assert_eq!(error.code(), ErrorCode::UiAssertionFailed);
        assert_eq!(error.body().context["expected"], serde_json::json!(300));
        assert_eq!(error.body().context["actual"], serde_json::json!(320));
        assert_eq!(error.body().context["tolerance"], serde_json::json!(2));
        assert_eq!(error.body().context["passed"], serde_json::json!(false));
        assert_eq!(
            error.body().context["comparison"]["relation"],
            serde_json::json!("eq")
        );
        assert!(compare_bottom_y(301, 300, 2).require_passed().is_ok());
    }

    fn stability_bounds(bottom: i32) -> Bounds {
        Bounds::from(Rect {
            left: 0,
            top: 0,
            right: 100,
            bottom,
        })
    }

    #[test]
    fn bounds_stability_succeeds_only_after_holding_the_same_bounds() {
        let base = Instant::now();
        let mut tracker = BoundsStabilityTracker::new();
        let stable_for = Duration::from_millis(1000);
        let timeout = Duration::from_secs(5);
        let first = stability_bounds(200);
        let moved = stability_bounds(240);

        assert_eq!(
            bounds_stability_step(&mut tracker, first, base, base, stable_for, timeout),
            StabilityStep::Continue
        );
        assert_eq!(
            bounds_stability_step(
                &mut tracker,
                moved,
                base + Duration::from_millis(200),
                base,
                stable_for,
                timeout
            ),
            StabilityStep::Continue
        );
        assert_eq!(
            bounds_stability_step(
                &mut tracker,
                moved,
                base + Duration::from_millis(900),
                base,
                stable_for,
                timeout
            ),
            StabilityStep::Continue
        );
        assert_eq!(
            bounds_stability_step(
                &mut tracker,
                moved,
                base + Duration::from_millis(1300),
                base,
                stable_for,
                timeout
            ),
            StabilityStep::Stable
        );
        assert_eq!(tracker.attempts, 4);
    }

    #[test]
    fn bounds_stability_times_out_while_bounds_keep_changing() {
        let base = Instant::now();
        let mut tracker = BoundsStabilityTracker::new();
        let stable_for = Duration::from_millis(1000);
        let timeout = Duration::from_millis(700);

        assert_eq!(
            bounds_stability_step(
                &mut tracker,
                stability_bounds(200),
                base,
                base,
                stable_for,
                timeout
            ),
            StabilityStep::Continue
        );
        assert_eq!(
            bounds_stability_step(
                &mut tracker,
                stability_bounds(240),
                base + Duration::from_millis(250),
                base,
                stable_for,
                timeout
            ),
            StabilityStep::Continue
        );
        assert_eq!(
            bounds_stability_step(
                &mut tracker,
                stability_bounds(200),
                base + Duration::from_millis(500),
                base,
                stable_for,
                timeout
            ),
            StabilityStep::Continue
        );
        assert_eq!(
            bounds_stability_step(
                &mut tracker,
                stability_bounds(240),
                base + Duration::from_millis(750),
                base,
                stable_for,
                timeout
            ),
            StabilityStep::TimedOut
        );
    }
}
