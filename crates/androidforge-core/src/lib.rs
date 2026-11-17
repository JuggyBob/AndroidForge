//! Shared AndroidForge vocabulary: structured errors, configuration, sessions,
//! artifacts, run reports and the JSON envelope.
//!
//! Nothing in this crate knows about a specific Android project. Project
//! differences belong in `.androidforge/project.toml`, fixtures and hooks.

pub mod analysis;
pub mod artifact;
pub mod config;
pub mod diagnosis;
pub mod error;
pub mod model;
pub mod output;
pub mod paths;
pub mod recurrence;
pub mod redact;
pub mod retry;
pub mod run;
pub mod session;
pub mod toolchain;
pub mod util;

pub use error::{ErrorBody, ErrorCode, ForgeError, Result};
pub use model::{ArtifactRef, BuildIdentity, Device, DeviceKind, DeviceState, ToolInfo};
pub use run::{FailureInfo, RunInvocation, RunReport, RunStatus, RunStep, StepStatus};
pub use session::{Session, SessionKind, SessionState};
