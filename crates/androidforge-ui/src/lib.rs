//! UI hierarchy parsing, semantic selectors and device interaction.
//!
//! UIAutomator XML is the source of truth. Selectors are exact by default and
//! ambiguity is an error rather than an arbitrary tap.

mod hierarchy;
mod manager;
mod selector;

pub use hierarchy::{Rect, UiHierarchy, UiNode};
pub use manager::{
    BackResult, FindResult, KeyResult, ScrollResult, ScrollToResult, SettleResult, TapResult,
    TextResult, UiManager, WaitResult,
};
pub use selector::{Selection, Selector, UiMatch, find, select};
