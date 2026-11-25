//! Project fixtures, debug hooks, conformance flows and optional external tools.

mod conformance;
mod external;
mod fixture;
mod hook;
mod instrumentation;
mod maestro;
mod scenario;

pub use conformance::{
    ConformanceExpectation, ConformanceFlow, ConformanceResult, run_conformance,
};
pub use external::{ExternalRunOptions, ExternalRunResult, run_external};
pub use fixture::{
    DEFAULT_FIXTURE_PREFS_PATH, DEFAULT_FIXTURE_VERSION_PATH, FixtureAction, FixtureHistoryState,
    FixtureInstalledVersionMode, FixtureInstalledVersionState, FixtureManifest, FixturePhase,
    FixturePrefsState, FixtureRunOptions, FixtureRunResult, FixtureState, FixtureStateExecution,
    FixtureStateOutcome, FixtureStatePlan, FixtureStatus, FixtureStep, FixtureStepOutcome,
    MAX_FIXTURE_HISTORY_COUNT, MAX_FIXTURE_STATE_PATH_COMPONENTS, MAX_FIXTURE_STATE_PATH_LEN,
    PlannedFixtureHistory, PlannedFixtureInstalledVersion, PlannedFixturePrefs,
    apply_fixture_state, fixture_state_reset_commands, list_fixtures, load_fixture,
    plan_fixture_state, require_fixture_data_mutation, require_fixture_debuggable,
    reset_fixture_state, run_fixture, run_fixture_with_data_policy, serialize_fixture_prefs,
    validate_fixture, validate_fixture_state_report, validate_state_path,
};
pub use hook::{HookResult, run_hook};
pub use instrumentation::{InstrumentationResult, run_instrumentation};
pub use maestro::{MaestroResult, run_maestro};
pub use scenario::{
    AssertSpec, AuditedOperation, BoundsRelation, ColorExpectation, FlakeSummary, IterationResult,
    PlannedStep, SCENARIO_VERSION, ScenarioConsent, ScenarioExecutor, ScenarioManifest,
    ScenarioPlan, ScenarioRepeat, ScenarioRunOptions, ScenarioRunResult, ScenarioStep,
    ScenarioSummary, SelectorSpec, StepContext, StepErrorPolicy, StepOutcome, StepRecord,
    UNSAFE_STEP_KINDS, load_scenario, parse_scenario_text, plan_scenario, run_scenario, summarize,
    validate_manifest, validate_manifest_shape,
};
