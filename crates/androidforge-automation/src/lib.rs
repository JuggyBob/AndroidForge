//! Project fixtures, debug hooks, conformance flows and optional external tools.

mod conformance;
mod external;
mod fixture;
mod hook;
mod instrumentation;
mod maestro;

pub use conformance::{
    ConformanceExpectation, ConformanceFlow, ConformanceResult, run_conformance,
};
pub use external::{ExternalRunOptions, ExternalRunResult, run_external};
pub use fixture::{
    FixtureAction, FixtureManifest, FixturePhase, FixtureRunOptions, FixtureRunResult,
    FixtureStatus, FixtureStep, FixtureStepOutcome, list_fixtures, load_fixture, run_fixture,
    validate_fixture,
};
pub use hook::{HookResult, run_hook};
pub use instrumentation::{InstrumentationResult, run_instrumentation};
pub use maestro::{MaestroResult, run_maestro};
