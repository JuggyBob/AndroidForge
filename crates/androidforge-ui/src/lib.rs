//! UI hierarchy parsing, semantic selectors and device interaction.
//!
//! UIAutomator XML is the source of truth. Selectors are exact by default and
//! ambiguity is an error rather than an arbitrary tap.

mod hierarchy;
mod manager;
mod reference;
mod selector;

pub mod a11y;
pub mod color;
pub mod visual;

pub use a11y::{
    A11yFinding, A11yMeasurement, A11yOptions, A11yReport, A11yRule, ContrastCheck, Severity,
    accessible_label, audit_hierarchy, audit_region_contrast, check_contrast, contrast_finding,
    region_from_bounds,
};
pub use color::{ColorComparison, Region, Rgb, Rgba};
pub use visual::{Image, decode, decode_reader};

pub use hierarchy::{Rect, UiHierarchy, UiNode};
pub use manager::{
    BackResult, Bounds, BoundsComparison, BoundsRelation, BoundsStabilityResult, FindResult,
    KeyResult, ScrollResult, ScrollToResult, SettleResult, TapResult, TextAbsenceResult,
    TextResult, UiManager, WaitResult, compare_bottom_y, compare_bottom_y_gt, compare_bottom_y_lt,
};
pub use reference::NodeExpectation;
pub use selector::{
    Selection, Selector, UiMatch, find, observation_ref, observation_ref_with_length, select,
};
