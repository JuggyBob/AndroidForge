//! ADB integration.
//!
//! This crate owns all knowledge of ADB command construction and output
//! parsing. Higher-level crates ask for semantic operations instead of
//! concatenating device shell commands themselves.

mod client;
mod devices;

pub use client::{AdbClient, BOOT_COMPLETE_PROP, BOOT_COMPLETED_PROP, ForegroundApp, WakeResult};
pub use devices::{parse_devices_output, parse_getprop_output, parse_wm_density, parse_wm_size};
