use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "androidforge",
    version,
    about = "Local-first Android device testing and orchestration for AI coding agents",
    long_about = "AndroidForge is a local-first CLI for AI coding agents working on Android apps. It manages emulators, sessions, installs, UI inspection, diagnostics, fixtures and evidence without a daemon or cloud service.",
    arg_required_else_help = true,
    propagate_version = true,
    disable_help_subcommand = false
)]
pub struct Cli {
    /// Machine-readable JSON on stdout. Diagnostics and progress go to stderr.
    #[arg(long, global = true)]
    pub json: bool,

    /// NDJSON on stdout: implies `--json`, minifies every line and writes one
    /// envelope line (plus streamed iteration lines for `scenario run`).
    #[arg(long, global = true)]
    pub jsonl: bool,

    /// Keep only these comma-separated top-level keys of the result object.
    /// Envelope fields are never removed. Envelope trimming is still available
    /// through `--output-profile compact|brief`.
    #[arg(long, global = true, value_name = "KEYS")]
    pub fields: Option<String>,

    /// Write one JSON stats line to stderr after the primary payload.
    #[arg(long, global = true)]
    pub stats: bool,

    /// Output detail profile. `compact` minifies and caps evidence; `brief` is the smallest decision-oriented view.
    #[arg(long, global = true, value_enum, env = "ANDROIDFORGE_PROFILE")]
    pub output_profile: Option<OutputProfile>,

    /// Shorthand for `--output-profile brief`.
    #[arg(long, global = true)]
    pub brief: bool,

    /// Shorthand for `--output-profile compact`.
    #[arg(long, global = true)]
    pub compact: bool,

    /// Maximum list items in compact/brief output.
    #[arg(long, global = true, env = "ANDROIDFORGE_MAX_ITEMS")]
    pub max_items: Option<usize>,

    /// Maximum characters for text fields in compact/brief output.
    #[arg(long, global = true, env = "ANDROIDFORGE_MAX_TEXT_CHARS")]
    pub max_text_chars: Option<usize>,

    /// Trace external command argv, timeouts and exit codes to stderr.
    #[arg(long, global = true)]
    pub trace: bool,

    /// Print the planned operation without changing device or project state.
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Confirm a destructive operation. Required for data-clearing, uninstall and restore commands.
    #[arg(long, global = true)]
    pub yes: bool,

    /// Wait for an OEM on-device install confirmation dialog after a restricted
    /// install failure, tap the single unambiguous confirm button through the
    /// audited UI path, then retry the install once. Install consent is unchanged.
    #[arg(long, global = true)]
    pub wait_for_confirmation: bool,

    /// Button label that confirms the OEM dialog. Repeatable; when omitted a
    /// built-in list of localized labels is used. Matching is case-insensitive.
    #[arg(long = "confirm-text", global = true)]
    pub confirm_text: Vec<String>,

    /// Maximum time to wait for the OEM confirmation dialog, for example 15s.
    #[arg(long = "confirmation-timeout", global = true)]
    pub confirmation_timeout: Option<String>,

    /// Allow data-changing operations on a physical device. Emulators never need this flag.
    /// Superset of every scoped `--allow-physical-*` flag below.
    #[arg(long, global = true)]
    pub allow_physical_mutation: bool,

    /// Allow app launch, force-stop, broadcast, service, kill and intent operations on a physical device.
    #[arg(long, global = true)]
    pub allow_physical_app_control: bool,

    /// Allow app-private data and fixture operations on a physical device.
    #[arg(long, global = true)]
    pub allow_physical_data: bool,

    /// Allow network state, proxy and mock changes on a physical device.
    #[arg(long, global = true)]
    pub allow_physical_network: bool,

    /// Allow install and uninstall operations on a physical device.
    #[arg(long, global = true)]
    pub allow_physical_install: bool,

    /// Allow input injection (tap, text, clear, scroll, back, home and keys) on a physical device.
    #[arg(long, global = true)]
    pub allow_physical_input: bool,

    /// Session id. Also read from ANDROIDFORGE_SESSION.
    #[arg(long, global = true)]
    pub session: Option<String>,

    /// Device serial. Must be explicit when multiple devices are connected.
    #[arg(long, global = true)]
    pub serial: Option<String>,

    /// Project directory containing .androidforge/project.toml.
    #[arg(long, global = true)]
    pub project: Option<PathBuf>,

    /// Append to an existing run id when the command records run steps. Also read from
    /// ANDROIDFORGE_RUN_ID. Commands that do not record steps print a warning.
    #[arg(long, global = true)]
    pub run: Option<String>,

    /// Suppress human-readable output on stdout. JSON mode is unaffected.
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Do not capture automatic failure evidence.
    #[arg(long, global = true)]
    pub no_failure_capture: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum OutputProfile {
    Full,
    Compact,
    Brief,
}

impl OutputProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Compact => "compact",
            Self::Brief => "brief",
        }
    }

    pub fn is_compact(self) -> bool {
        self != Self::Full
    }
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
    /// Inspect host, SDK, ADB, emulator, tools, AVDs, disks and sessions.
    Doctor {
        /// Report conservative host RAM/commit/disk capacity and per-AVD estimates.
        /// Unknown readings stay `null`/unknown with a reason; this flag is read-only.
        #[arg(long)]
        capacity: bool,
        #[command(subcommand)]
        command: Option<DoctorCommand>,
    },
    /// Compact, agent-oriented snapshot of the project, session, device, app and screen.
    #[command(alias = "ctx")]
    Context(ContextArgs),
    /// Run a transparent high-level agent workflow.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Report implemented, best-effort and unsupported capabilities.
    Capabilities,
    /// Run the local Model Context Protocol server.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Export the MCP/function-calling tool catalog.
    Tools {
        #[command(subcommand)]
        command: ToolsCommand,
    },
    /// Print AndroidForge JSON schemas.
    Schema {
        #[command(subcommand)]
        command: SchemaCommand,
    },
    /// Create a project adapter or global configuration.
    Init(InitArgs),
    /// Inspect and validate project/global configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Manage device ownership sessions.
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    /// Discover, inspect and operate Android devices. `device` is the retained
    /// 1.0.0 alias for this canonical command.
    #[command(alias = "device")]
    Devices {
        #[command(subcommand)]
        command: DevicesCommand,
    },
    /// List Android Virtual Devices.
    Avd {
        #[command(subcommand)]
        command: AvdCommand,
    },
    /// Inspect emulator console/ADB/gRPC port allocation without a device.
    Ports {
        #[command(subcommand)]
        command: PortsCommand,
    },
    /// Start, inspect and stop Android emulators.
    Emulator {
        #[command(subcommand)]
        command: EmulatorCommand,
    },
    /// Manage emulator snapshots. Loading is destructive and requires --yes.
    Snapshot {
        #[command(subcommand)]
        command: SnapshotCommand,
    },
    /// Install and control applications.
    App {
        #[command(subcommand)]
        command: AppCommand,
    },
    /// Build a configured variant, optionally install the result and launch it.
    Build(BuildArgs),
    /// Inspect and verify APKs locally.
    Apk {
        #[command(subcommand)]
        command: ApkCommand,
    },
    /// Inspect and interact with Android UI Automation hierarchies.
    Ui {
        #[command(subcommand)]
        command: UiCommand,
    },
    /// Capture screenshots and screen recordings.
    Capture {
        #[command(subcommand)]
        command: CaptureCommand,
    },
    /// Inspect AndroidForge-owned artifacts without a session or device.
    Artifacts {
        #[command(subcommand)]
        command: ArtifactsCommand,
    },
    /// Collect logcat, crashes, process/task state and dumpsys evidence.
    Diagnostics {
        #[command(subcommand)]
        command: DiagnosticsCommand,
    },
    /// Inspect and wait for notifications.
    Notifications {
        #[command(subcommand)]
        command: NotificationsCommand,
    },
    /// Inspect and trigger JobScheduler jobs.
    Jobs {
        #[command(subcommand)]
        command: JobsCommand,
    },
    /// Inspect AlarmManager alarms.
    Alarms {
        #[command(subcommand)]
        command: AlarmsCommand,
    },
    /// Check, grant and revoke Android permissions.
    Permissions {
        #[command(subcommand)]
        command: PermissionsCommand,
    },
    /// Read and change AppOps modes.
    Appops {
        #[command(subcommand)]
        command: AppOpsCommand,
    },
    /// Inspect or force Doze.
    Doze {
        #[command(subcommand)]
        command: DozeCommand,
    },
    /// Inspect or change app standby buckets.
    Standby {
        #[command(subcommand)]
        command: StandbyCommand,
    },
    /// Inspect or modify battery state.
    Battery {
        #[command(subcommand)]
        command: BatteryCommand,
    },
    /// Inspect or change network state.
    Network {
        #[command(subcommand)]
        command: NetworkCommand,
    },
    /// Inspect and control device time. Changes require --yes.
    Time {
        #[command(subcommand)]
        command: TimeCommand,
    },
    /// Run explicit debug-only app hooks configured by the project adapter.
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },
    /// List, validate and run project fixtures.
    Fixture {
        #[command(subcommand)]
        command: FixtureCommand,
    },
    /// Run a project conformance flow.
    Conformance {
        #[command(subcommand)]
        command: ConformanceCommand,
    },
    /// Run or validate an allowlisted declarative scenario manifest.
    Scenario {
        #[command(subcommand)]
        command: ScenarioCommand,
    },
    /// Run optional Maestro or instrumentation tests.
    Test {
        #[command(subcommand)]
        command: TestCommand,
    },
    /// Start, finish or execute within a run report.
    Run {
        #[command(subcommand)]
        command: RunCommand,
    },
    /// Inspect, locate, prune or import run reports.
    Report {
        #[command(subcommand)]
        command: ReportCommand,
    },
    /// Inspect or clean AndroidForge-owned storage.
    Maintenance {
        #[command(subcommand)]
        command: MaintenanceCommand,
    },
    /// Unrestricted raw ADB passthrough. This is explicitly unsafe.
    Adb(AdbArgs),
}

#[derive(Debug, Subcommand)]
pub enum DoctorCommand {
    /// Produce a read-only, executable repair plan for environment problems.
    Fix(DoctorFixArgs),
}

#[derive(Debug, Args)]
pub struct DoctorFixArgs {
    /// Print the repair plan without modifying the environment. This is the default.
    #[arg(long)]
    pub plan: bool,
    /// Re-check the environment and return a plan. Read-only.
    #[arg(long)]
    pub check: bool,
}

#[derive(Debug, Args)]
pub struct ContextArgs {
    /// Include only these sections: project, environment, session, device, app, screen, last-run.
    #[arg(long, value_delimiter = ',')]
    pub include: Vec<String>,
    /// Include complete available information instead of the compact default.
    #[arg(long)]
    pub full: bool,
    /// Refresh the screen observation even if a recent observation is cached.
    #[arg(long)]
    pub refresh_screen: bool,
    /// Skip the screen section entirely.
    #[arg(long)]
    pub no_screen: bool,
}

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// One small project/session/device/app/screen snapshot for an agent turn.
    #[command(alias = "b")]
    Brief,
    /// Run the transparent ensure/inspect/install/conformance/explain pipeline.
    #[command(alias = "t")]
    Test(AgentTestArgs),
    /// Run the same flow against multiple independent AVDs or serials.
    Matrix(AgentMatrixArgs),
}

#[derive(Debug, Args)]
pub struct AgentMatrixArgs {
    /// AVD target. Repeat for multiple devices.
    #[arg(long)]
    pub avd: Vec<String>,
    /// Connected-device serial target. Repeat for multiple devices.
    #[arg(long)]
    pub serial: Vec<String>,
    /// Host-tagged device target. `tag:<name>` resolves through the local tag
    /// store to concrete serials; literal serials are accepted too. Repeatable.
    #[arg(long = "device")]
    pub device: Vec<String>,
    /// Flow to run, for example conformance or a path.
    #[arg(long)]
    pub flow: Option<String>,
    #[arg(long)]
    pub fixture: Option<String>,
    #[arg(long)]
    pub apk: Option<PathBuf>,
    #[arg(long)]
    pub variant: Option<String>,
    #[arg(long)]
    pub owner: Option<String>,
    #[arg(long)]
    pub profile: Option<String>,
    #[arg(long)]
    pub ttl: Option<String>,
    #[arg(long)]
    pub install: bool,
    /// Stop after the first target failure. In parallel mode, already-running
    /// targets finish and queued targets may still start.
    #[arg(long)]
    pub fail_fast: bool,
    /// Number of parallel workers (1-32). 1 is sequential.
    #[arg(long, default_value_t = 1)]
    pub parallel: usize,
    /// Keep matrix-created sessions after the run instead of releasing them.
    #[arg(long)]
    pub keep_sessions: bool,
    /// Per-target child operation timeout, for example 20m.
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct AgentTestArgs {
    /// APK to inspect and optionally install before testing. Takes precedence over --variant.
    #[arg(long)]
    pub apk: Option<PathBuf>,
    /// Additional split APKs for an explicit base APK.
    #[arg(long = "split")]
    pub split: Vec<PathBuf>,
    /// Resolve an APK from [build.variants] configuration without running Gradle.
    #[arg(long)]
    pub variant: Option<String>,
    /// Existing session id. If omitted, a session is reused or ensured.
    #[arg(long)]
    pub session_id: Option<String>,
    /// AVD for session ensure.
    #[arg(long)]
    pub avd: Option<String>,
    /// Explicit serial for session ensure instead of an AVD.
    #[arg(long)]
    pub serial: Option<String>,
    /// Owner for session ensure.
    #[arg(long)]
    pub owner: Option<String>,
    /// Profile label for session ensure.
    #[arg(long)]
    pub profile: Option<String>,
    /// Lease duration for session ensure, for example 30m.
    #[arg(long)]
    pub ttl: Option<String>,
    /// Emulator console port to use when launching the requested AVD.
    #[arg(long)]
    pub port: Option<u16>,
    /// Permit installing the APK when the installed build cannot be proven current.
    #[arg(long)]
    pub install: bool,
    /// Run the configured conformance flow. Use `--flow conformance` for the default flow or a path for a specific flow.
    #[arg(long)]
    pub flow: Option<String>,
    /// Run a project fixture before conformance.
    #[arg(long)]
    pub fixture: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum SchemaCommand {
    List,
    Show { name: String },
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Run MCP over stdin/stdout. Only JSON-RPC lines are written to stdout.
    Serve,
}

#[derive(Debug, Subcommand)]
pub enum ToolsCommand {
    /// Export the tool catalog as MCP, OpenAI function-calling or plain JSON.
    Schema(ToolsSchemaArgs),
    /// Print the complete CLI command catalog used by the desktop tool database.
    Catalog,
    /// Install an optional AndroidForge-managed tool. Currently only Maestro.
    Install {
        /// Tool name: maestro.
        tool: String,
        /// Replace an existing installation of the same version.
        #[arg(long)]
        force: bool,
    },
    /// Report optional tool availability and managed install paths.
    Status {
        /// Tool name: maestro. Omit to report every optional tool.
        tool: Option<String>,
    },
    /// Remove an AndroidForge-managed optional tool.
    Uninstall {
        /// Tool name: maestro.
        tool: String,
    },
}

#[derive(Debug, Args)]
pub struct ToolsSchemaArgs {
    /// Output format: mcp, openai or json.
    #[arg(long, default_value = "mcp")]
    pub format: String,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub package: Option<String>,
    #[arg(long)]
    pub avd: Option<String>,
    #[arg(long)]
    pub global: bool,
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Show,
    Validate,
    Path,
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    Start(SessionStartArgs),
    /// Idempotently acquire or reuse a compatible session.
    Ensure(SessionEnsureArgs),
    Attach(SessionAttachArgs),
    Claim(SessionAttachArgs),
    List {
        /// Hide released/failed/abandoned tombstones.
        #[arg(long)]
        active: bool,
        /// Only sessions owned by the current ANDROIDFORGE_OWNER/OS user.
        #[arg(long)]
        mine: bool,
        /// Only sessions with this owner label (implies --mine semantics).
        #[arg(long)]
        owner: Option<String>,
    },
    Inspect,
    Status,
    Release(SessionReleaseArgs),
    Stop(SessionStopArgs),
    Use {
        session_id: String,
    },
    Current,
    Gc(SessionGcArgs),
    /// Remove old released/abandoned/failed session tombstones.
    Prune(SessionPruneArgs),
    Restart(SessionRestartArgs),
}

#[derive(Debug, Args, Clone)]
pub struct SessionEnsureArgs {
    /// AVD to use if a compatible emulator session does not already exist.
    #[arg(long)]
    pub avd: Option<String>,
    /// Explicitly attach to an already-running serial instead of starting an AVD.
    #[arg(long)]
    pub serial: Option<String>,
    /// Owner label recorded on the session.
    #[arg(long)]
    pub owner: Option<String>,
    /// Profile label stored with the session.
    #[arg(long)]
    pub profile: Option<String>,
    /// Lease duration metadata, for example 30m or 2h. Does not stop devices on expiry.
    #[arg(long)]
    pub ttl: Option<String>,
    /// Emulator console port to use when launching the requested AVD.
    #[arg(long)]
    pub port: Option<u16>,
    /// Start the emulator headless.
    #[arg(long)]
    pub headless: bool,
    /// Allow manual emulator snapshots by omitting the default -no-snapshot-save flag.
    #[arg(long)]
    pub snapshot_save: bool,
    /// Wait for a busy device instead of failing immediately.
    #[arg(long)]
    pub wait: bool,
    /// Maximum wait duration when --wait is used.
    #[arg(long, default_value = "10m")]
    pub timeout: String,
    /// Prepare animations/stay-awake settings.
    #[arg(long)]
    pub prepare: bool,
}

#[derive(Debug, Args)]
pub struct SessionStartArgs {
    #[arg(long)]
    pub avd: String,
    #[arg(long)]
    pub owner: Option<String>,
    #[arg(long)]
    pub port: Option<u16>,
    #[arg(long)]
    pub headless: bool,
    #[arg(long)]
    pub windowed: bool,
    #[arg(long)]
    pub memory: Option<u32>,
    #[arg(long)]
    pub cores: Option<u32>,
    #[arg(long)]
    pub cold_boot: bool,
    /// Allow manual emulator snapshots by omitting the default -no-snapshot-save flag.
    #[arg(long)]
    pub snapshot_save: bool,
    #[arg(long)]
    pub reuse: bool,
    #[arg(long)]
    pub prepare: bool,
    #[arg(long)]
    pub no_wake: bool,
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub struct SessionAttachArgs {
    #[arg(long)]
    pub owner: Option<String>,
    #[arg(long)]
    pub avd: Option<String>,
    #[arg(long)]
    pub prepare: bool,
    #[arg(long)]
    pub no_wake: bool,
}

#[derive(Debug, Args, Clone)]
pub struct SessionReleaseArgs {
    #[arg(long)]
    pub stop: bool,
    #[arg(long)]
    pub stop_attached: bool,
    #[arg(long)]
    pub keep_prepared: bool,
    /// Owner recorded when the session started. Defaults to ANDROIDFORGE_OWNER
    /// or the operating-system user.
    #[arg(long)]
    pub owner: Option<String>,
    /// Release a session owned by someone else. This is a deliberate takeover.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args, Clone)]
pub struct SessionStopArgs {
    #[arg(long)]
    pub attached: bool,
    #[arg(long)]
    pub keep_prepared: bool,
    /// Owner recorded when the session started. Defaults to ANDROIDFORGE_OWNER
    /// or the operating-system user.
    #[arg(long)]
    pub owner: Option<String>,
    /// Stop a session owned by someone else. This is a deliberate takeover.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args, Clone)]
pub struct SessionGcArgs {
    #[arg(long)]
    pub reclaim: bool,
    #[arg(long)]
    pub stop: bool,
    /// Release the device lock of terminal (released/failed/abandoned) session
    /// tombstones without deleting their history. Dry-run by default when used.
    #[arg(long)]
    pub forget_terminal: bool,
    /// Owner allowed to forget terminal sessions. Defaults to ANDROIDFORGE_OWNER.
    #[arg(long)]
    pub owner: Option<String>,
    /// Forget terminal sessions owned by someone else. Never steals a live lock.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args, Clone)]
pub struct SessionPruneArgs {
    #[arg(long)]
    pub older_than: Option<String>,
    #[arg(long)]
    pub keep_last: Option<usize>,
}

#[derive(Debug, Args, Clone)]
pub struct SessionRestartArgs {
    #[arg(long)]
    pub cold_boot: bool,
    #[arg(long)]
    pub timeout: Option<String>,
    /// Owner recorded when the session started. Defaults to ANDROIDFORGE_OWNER
    /// or the operating-system user.
    #[arg(long)]
    pub owner: Option<String>,
    /// Restart a session owned by someone else. This is a deliberate takeover.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Subcommand)]
pub enum DevicesCommand {
    List,
    Inspect,
    Wait(DevicesWaitArgs),
    Prepare(PrepareArgs),
    Restore,
    /// Pair with an Android wireless-debugging endpoint using a six-digit code.
    Pair(DevicesPairArgs),
    /// Connect to a paired wireless-debugging endpoint.
    Connect(DevicesEndpointArgs),
    /// Disconnect a wireless ADB endpoint.
    Disconnect(DevicesEndpointArgs),
    /// Collect structured, read-only device health.
    Health(DevicesHealthArgs),
    /// Apply one device setting through the typed scenario API.
    Scenario(DevicesScenarioArgs),
    /// Manage host-only device tags used by `agent matrix --device`.
    Tag {
        #[command(subcommand)]
        command: DevicesTagCommand,
    },
    /// Run a shell command through the typed ADB path. `--read-only` requires an allowlisted command and no physical grant.
    Shell(DeviceShellArgs),
    /// Inject input through the audited UI path under the narrow `Risk::Input` gate.
    Input {
        #[command(subcommand)]
        command: DeviceInputCommand,
    },
}

#[derive(Debug, Args, Clone)]
pub struct DevicesScenarioArgs {
    /// Night mode: yes, no or auto. Mutates device settings.
    #[arg(long, conflicts_with_all = ["locale", "font", "network"])]
    pub night: Option<String>,
    /// BCP-47 locale tag, for example en-US. Mutates device settings.
    #[arg(long, conflicts_with_all = ["night", "font", "network"])]
    pub locale: Option<String>,
    /// Font scale between 0.5 and 2.0, for example 1.15. Mutates device settings.
    #[arg(long, conflicts_with_all = ["night", "locale", "network"])]
    pub font: Option<String>,
    /// Network mode: offline, online, wifi-only or data-only. Mutates network state.
    #[arg(long, conflicts_with_all = ["night", "locale", "font"])]
    pub network: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum DevicesTagCommand {
    /// Add a safe host-only tag to a serial. Idempotent.
    Add(DevicesTagArgs),
    /// Remove a tag from a serial. Idempotent.
    Remove(DevicesTagArgs),
    /// List tags for one serial, or every tagged serial.
    List {
        /// Serial to inspect. Omit to list all tagged serials.
        #[arg(long)]
        serial: Option<String>,
    },
    /// Remove every tag from a serial. Idempotent.
    Clear {
        /// Serial whose tags should be cleared.
        #[arg(long)]
        serial: String,
    },
}

#[derive(Debug, Args, Clone)]
pub struct DevicesTagArgs {
    /// Device serial, for example emulator-5554.
    #[arg(long)]
    pub serial: String,
    /// Safe token: 1-64 lowercase letters/digits/'.'/'_'/'-'.
    #[arg(long)]
    pub tag: String,
}

#[derive(Debug, Args)]
pub struct DevicesPairArgs {
    /// Pairing endpoint from the device, for example 192.168.1.20:37123.
    pub endpoint: String,
    /// Six-digit pairing code. Prefer --code-stdin to keep it out of shell history.
    #[arg(long, conflicts_with = "code_stdin")]
    pub code: Option<String>,
    /// Read the pairing code from stdin.
    #[arg(long = "code-stdin")]
    pub code_stdin: bool,
}

#[derive(Debug, Args)]
pub struct DevicesEndpointArgs {
    /// ADB wireless endpoint, for example 192.168.1.20:37521.
    pub endpoint: String,
}

#[derive(Debug, Args)]
pub struct DevicesHealthArgs {
    /// Omit raw command evidence from the result.
    #[arg(long = "no-evidence")]
    pub no_evidence: bool,
}

#[derive(Debug, Args)]
pub struct DevicesWaitArgs {
    #[arg(long)]
    pub boot: bool,
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub struct PrepareArgs {
    #[arg(long)]
    pub animations: bool,
    #[arg(long)]
    pub keep_awake: bool,
    #[arg(long)]
    pub wake: bool,
    #[arg(long)]
    pub locale: Option<String>,
    #[arg(long)]
    pub timezone: Option<String>,
    #[arg(long)]
    pub font_scale: Option<String>,
    #[arg(long)]
    pub dark_mode: Option<bool>,
    #[arg(long)]
    pub orientation: Option<String>,
    #[arg(long)]
    pub density: Option<u32>,
}

#[derive(Debug, Subcommand)]
pub enum AvdCommand {
    List,
    /// List installed SDK system images available for AVD creation. Read-only.
    Images,
    /// Create an AVD through `avdmanager`. Host-side mutation: requires --yes.
    Create(AvdCreateArgs),
    /// Not implemented. Kept as an explicit structured refusal rather than a fake clone.
    Clone {
        source: String,
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Debug, Args, Clone)]
pub struct AvdCreateArgs {
    /// AVD name. Letters, numbers, '.', '_' and '-'; no path separators.
    pub name: String,
    /// Full SDK system-image package id, for example system-images;android-35;google_apis;x86_64.
    #[arg(long)]
    pub image: String,
    /// Device profile id accepted by `avdmanager -d`, for example pixel_7.
    #[arg(long)]
    pub device: String,
}

#[derive(Debug, Subcommand)]
pub enum PortsCommand {
    /// Report every conventional emulator console port with its ADB/gRPC pair,
    /// ownership from active AndroidForge sessions and free/allocated/occupied state.
    List,
}

#[derive(Debug, Subcommand)]
pub enum EmulatorCommand {
    Start(SessionStartArgs),
    Status,
    Stop(EmulatorStopArgs),
}

#[derive(Debug, Args)]
pub struct EmulatorStopArgs {
    #[arg(long)]
    pub attached: bool,
    #[arg(long)]
    pub owner: Option<String>,
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Subcommand)]
pub enum SnapshotCommand {
    List,
    Save { name: String },
    Load { name: String },
    Delete { name: String },
}

#[derive(Debug, Subcommand)]
pub enum AppCommand {
    #[command(alias = "i")]
    Install(AppInstallArgs),
    Uninstall(AppUninstallArgs),
    ClearData(AppPackageArgs),
    #[command(alias = "l")]
    Launch(AppLaunchArgs),
    /// Send a broadcast intent.
    Broadcast(AppBroadcastArgs),
    /// Start or stop a service intent.
    Service(AppServiceArgs),
    ForceStop(AppPackageArgs),
    Kill(AppPackageArgs),
    Status(AppStatusArgs),
    Info(AppPackageArgs),
    List(AppListArgs),
    FilePush(AppFilePushArgs),
    FilePull(AppFilePullArgs),
    DataBackup(AppDataExportArgs),
    DataRestore(AppDataArgs),
    /// Verify the live installed APK against a local APK without installing.
    VerifyInstalled(AppVerifyInstalledArgs),
    /// Export, import, reset or seed application-private data.
    Data {
        #[command(subcommand)]
        command: AppDataCommand,
    },
}

#[derive(Debug, Args)]
pub struct AppVerifyInstalledArgs {
    /// Local APK to compare with the live installed base APK.
    #[arg(long)]
    pub apk: PathBuf,
    /// Additional split APKs belonging to the same candidate set.
    #[arg(long = "split")]
    pub split: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct AppDataResetArgs {
    pub package: String,
    /// Data archive to restore after clearing (created by `app data export`).
    #[arg(long)]
    pub baseline: Option<PathBuf>,
    /// Save the current data to this archive before clearing.
    #[arg(long = "backup-current")]
    pub backup_current: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct AppDataSeedArgs {
    pub package: String,
    /// Local file to write into app-private storage.
    #[arg(long)]
    pub file: PathBuf,
    /// Path relative to the app-private data directory.
    #[arg(long)]
    pub remote: String,
}

#[derive(Debug, Subcommand)]
pub enum AppDataCommand {
    /// Export app-private data through run-as (debuggable apps).
    Export(AppDataExportArgs),
    /// Import app-private data through run-as (debuggable apps).
    Import(AppDataArgs),
    /// Clear app data and optionally restore a baseline archive.
    Reset(AppDataResetArgs),
    /// Write a local file into app-private storage.
    Seed(AppDataSeedArgs),
}

#[derive(Debug, Args)]
pub struct AppInstallArgs {
    pub apk: PathBuf,
    /// Additional split APKs that belong to the same split set.
    #[arg(long = "split")]
    pub split: Vec<PathBuf>,
    #[arg(long)]
    pub fresh: bool,
    #[arg(long)]
    pub downgrade: bool,
    #[arg(long)]
    pub no_verify: bool,
    #[arg(long)]
    pub expected_package: Option<String>,
    /// Additional certificate fingerprint enforcement for this install.
    #[arg(long)]
    pub expected_cert_sha256: Option<String>,
    #[arg(long)]
    pub timeout: Option<String>,
    /// Skip installation when the candidate APK is byte-equivalent to the installed base APK.
    #[arg(long)]
    pub only_if_changed: bool,
}

#[derive(Debug, Args)]
pub struct AppUninstallArgs {
    pub package: String,
    #[arg(long)]
    pub keep_data: bool,
}

#[derive(Debug, Args)]
pub struct AppPackageArgs {
    pub package: String,
}

#[derive(Debug, Args)]
pub struct AppLaunchArgs {
    pub package: String,
    #[arg(long)]
    pub activity: Option<String>,
    /// Explicit component, for example com.example/.MainActivity.
    #[arg(long)]
    pub component: Option<String>,
    /// Intent action, for example android.intent.action.VIEW.
    #[arg(long)]
    pub action: Option<String>,
    /// Intent data URI, for example myapp://checkout/42.
    #[arg(long)]
    pub data: Option<String>,
    #[arg(long = "mime-type")]
    pub mime_type: Option<String>,
    #[arg(long)]
    pub category: Vec<String>,
    /// Typed extra in key:type:value form; type is s, i, l, f, z, esa or eia.
    #[arg(long = "extra")]
    pub extra: Vec<String>,
    /// Intent flag, for example 0x10000000.
    #[arg(long)]
    pub flag: Vec<String>,
    #[arg(long)]
    pub wait: bool,
    #[arg(long = "force-stop")]
    pub force_stop: bool,
    #[arg(long)]
    pub no_wake: bool,
}

#[derive(Debug, Args)]
pub struct AppBroadcastArgs {
    /// Optional package recorded with the intent and used for validation.
    #[arg(long)]
    pub package: Option<String>,
    #[arg(long)]
    pub action: Option<String>,
    #[arg(long)]
    pub data: Option<String>,
    #[arg(long = "mime-type")]
    pub mime_type: Option<String>,
    #[arg(long)]
    pub category: Vec<String>,
    #[arg(long = "extra")]
    pub extra: Vec<String>,
    #[arg(long)]
    pub flag: Vec<String>,
    #[arg(long = "receiver-permission")]
    pub receiver_permission: Option<String>,
    #[arg(long)]
    pub asynchronous: bool,
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct AppServiceArgs {
    #[arg(long)]
    pub package: Option<String>,
    /// Service component, for example .SyncService or com.example/.SyncService.
    #[arg(long)]
    pub service: Option<String>,
    #[arg(long)]
    pub component: Option<String>,
    #[arg(long)]
    pub action: Option<String>,
    #[arg(long)]
    pub data: Option<String>,
    #[arg(long = "extra")]
    pub extra: Vec<String>,
    #[arg(long)]
    pub stop: bool,
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct AppStatusArgs {
    pub package: String,
    #[arg(long)]
    pub expect: Option<String>,
    /// Live installed versionName must equal this value.
    #[arg(long = "expect-version-name")]
    pub expect_version_name: Option<String>,
    /// Live installed versionCode must equal this value.
    #[arg(long = "expect-version-code")]
    pub expect_version_code: Option<i64>,
    /// Live installed base APK SHA-256 must equal this value.
    #[arg(long = "expect-apk-sha256")]
    pub expect_apk_sha256: Option<String>,
    #[arg(long)]
    pub assert_task_exists: bool,
    #[arg(long)]
    pub assert_task_absent: bool,
    #[arg(long)]
    pub assert_process_stable: bool,
    #[arg(long)]
    pub stable_for: Option<String>,
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct AppListArgs {
    #[arg(long)]
    pub third_party: bool,
    #[arg(long)]
    pub filter: Option<String>,
}

#[derive(Debug, Args)]
pub struct AppFilePushArgs {
    pub local: PathBuf,
    pub remote: String,
    #[arg(long)]
    pub package: Option<String>,
    #[arg(long)]
    pub external: bool,
}

#[derive(Debug, Args)]
pub struct AppFilePullArgs {
    pub remote: String,
    pub local: PathBuf,
    #[arg(long)]
    pub package: Option<String>,
    #[arg(long)]
    pub external: bool,
}

#[derive(Debug, Args, Clone)]
pub struct AppDataArgs {
    pub package: String,
    pub path: PathBuf,
}

/// Arguments for `app data export` (and its `app data backup` alias).
///
/// The positional `path` keeps the original explicit-destination form; the
/// named `--output` flag is equivalent. When neither is supplied the safe
/// managed `<artifacts_dir>/export` default is used. Destinations inside the
/// project root and existing files require an explicit opt-in.
#[derive(Debug, Args, Clone)]
pub struct AppDataExportArgs {
    pub package: String,
    /// Explicit destination archive. Omit to use the managed artifacts/export
    /// default, which is outside the project's source tree.
    pub path: Option<PathBuf>,
    /// Named equivalent of the positional destination path.
    #[arg(long = "output", conflicts_with = "path")]
    pub output: Option<PathBuf>,
    /// Allow an explicit destination inside the project root.
    #[arg(long = "allow-in-project")]
    pub allow_in_project: bool,
    /// Replace an existing destination archive instead of refusing it.
    #[arg(long)]
    pub overwrite: bool,
}

#[derive(Debug, Subcommand)]
pub enum ApkCommand {
    Inspect {
        apk: PathBuf,
    },
    Verify(ApkVerifyArgs),
    /// Resolve the newest valid APK for a configured variant without running Gradle.
    Resolve(ApkResolveArgs),
    /// Compare a candidate APK with an installed package.
    Diff(ApkDiffArgs),
}

#[derive(Debug, Args)]
pub struct ApkResolveArgs {
    /// Variant configured in [build.variants], for example debug or freeDebug.
    #[arg(long)]
    pub variant: Option<String>,
    /// Restrict candidates to a module directory, for example app.
    #[arg(long)]
    pub module: Option<String>,
}

#[derive(Debug, Args)]
pub struct ApkDiffArgs {
    /// Either `installed:<package>` or a local APK path.
    pub target: String,
    /// The other side of the comparison (APK path or `installed:<package>`).
    pub apk: String,
    /// Additional split APKs for the candidate side.
    #[arg(long = "split")]
    pub split: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ApkVerifyArgs {
    pub apk: PathBuf,
    #[arg(long)]
    pub expected_cert_sha256: Option<String>,
    #[arg(long)]
    pub expected_package: Option<String>,
    #[arg(long)]
    pub min_version_code: Option<i64>,
}

/// Arguments for `androidforge build`.
///
/// Build alone is host-side: it resolves a configured argv from
/// `[build.commands.<variant>]`, runs it with a bounded capture and a timeout,
/// then resolves the produced APK with the same deterministic logic as
/// `apk resolve`. It needs no session and touches no device. The install/launch
/// flags opt into the existing verified app paths and their physical gates.
#[derive(Debug, Args)]
pub struct BuildArgs {
    /// Variant configured in [build.commands.<variant>]. Defaults to [build].default_variant.
    #[arg(long)]
    pub variant: Option<String>,
    /// Restrict the produced APK to one module directory when several outputs match.
    #[arg(long)]
    pub module: Option<String>,
    /// Override the build timeout, for example 20m or 300s.
    #[arg(long)]
    pub timeout: Option<String>,
    /// Require the built APK to declare this exact package identity.
    #[arg(long = "expected-package")]
    pub expected_package: Option<String>,
    /// Install the built APK through the verified `app install` path.
    #[arg(long)]
    pub install: bool,
    /// Launch after installing. Implies --install.
    #[arg(long)]
    pub launch: bool,
    /// Force-stop then launch with a verified new PID. Implies --launch.
    #[arg(long)]
    pub cold: bool,
    /// Skip installation when the installed build is byte-equivalent.
    #[arg(long = "only-if-changed")]
    pub only_if_changed: bool,
    /// Uninstall before installing, deleting app data. Requires --yes.
    #[arg(long)]
    pub fresh: bool,
    /// Allow installing an older version code.
    #[arg(long)]
    pub downgrade: bool,
    /// Skip the post-install identity verification.
    #[arg(long = "no-verify")]
    pub no_verify: bool,
    /// Launcher activity, for example .MainActivity. Requires --launch or --cold.
    #[arg(long)]
    pub activity: Option<String>,
}

#[derive(Debug, Args, Default, Clone)]
pub struct SelectorArgs {
    #[arg(long = "ref")]
    pub reference: Option<String>,
    #[arg(long = "expect-token", alias = "token")]
    pub expect_token: Option<String>,
    #[arg(long)]
    pub text: Option<String>,
    #[arg(long)]
    pub contains: Option<String>,
    #[arg(long = "content-desc", alias = "desc")]
    pub content_desc: Option<String>,
    #[arg(long = "content-desc-contains", alias = "desc-contains")]
    pub content_desc_contains: Option<String>,
    #[arg(long = "resource-id", alias = "id")]
    pub resource_id: Option<String>,
    /// Compose testTag for debug builds that expose testTagsAsResourceId.
    #[arg(long = "test-tag", alias = "tag")]
    pub test_tag: Option<String>,
    #[arg(long = "class")]
    pub class: Option<String>,
    #[arg(long)]
    pub index: Option<usize>,
    #[arg(long)]
    pub clickable: bool,
    #[arg(long)]
    pub enabled: bool,
    #[arg(long)]
    pub focused: bool,
    #[arg(long)]
    pub scrollable: bool,
    #[arg(long)]
    pub checked: bool,
    #[arg(long)]
    pub visible: bool,
    #[arg(long)]
    pub role: Option<String>,
    #[arg(long = "text-regex")]
    pub text_regex: Option<String>,
    #[arg(long)]
    pub within: Option<String>,
    #[arg(long)]
    pub above: Option<String>,
    #[arg(long)]
    pub below: Option<String>,
    #[arg(long = "left-of")]
    pub left_of: Option<String>,
    #[arg(long = "right-of")]
    pub right_of: Option<String>,
    /// Device-pixel bounds x1,y1,x2,y2.
    #[arg(long)]
    pub region: Option<String>,
    /// Try local OCR when UIAutomator has no matching text.
    #[arg(long = "ocr-fallback")]
    pub ocr_fallback: bool,
}

#[derive(Debug, Subcommand)]
pub enum UiCommand {
    Dump(UiDumpArgs),
    /// Dump and return the parsed hierarchy without creating a run artifact.
    Inspect(UiInspectArgs),
    /// Capture a compact semantic observation and persist an observation token.
    #[command(alias = "o")]
    Observe(UiObserveArgs),
    /// Compare a previous observation with a new or stored observation.
    #[command(alias = "d")]
    Diff(UiDiffArgs),
    Find(UiFindArgs),
    #[command(alias = "t")]
    Tap(UiTapArgs),
    LongPress(UiTapArgs),
    /// Perform a press-move-release drag with optional holds and mid-gesture capture.
    Drag(UiDragArgs),
    Text(UiTextArgs),
    Clear(UiSelectorArgs),
    #[command(alias = "w")]
    Wait(UiWaitArgs),
    Assert(UiWaitArgs),
    /// Read the current bounds of a semantic UI element. Read-only.
    Bounds(UiBoundsArgs),
    /// Assert the bottom edge (`bottom_y`) of a semantic UI element. Read-only.
    AssertBounds(UiAssertBoundsArgs),
    /// Assert that text is not present.
    AssertNoText(UiAssertNoTextArgs),
    /// Dismiss known SystemUI immersive/permission overlays with a safe allowlist.
    DismissOverlays(UiDismissOverlaysArgs),
    Settle(UiSettleArgs),
    Scroll(UiScrollArgs),
    ScrollTo(UiScrollToArgs),
    Swipe(UiSwipeArgs),
    Back(UiBackArgs),
    Home,
    Keyevent(UiKeyEventArgs),
    HideKeyboard,
    Focused,
    /// Read-only accessibility and contrast audit of the current hierarchy.
    Audit(UiAuditArgs),
}

/// Arguments for `ui audit`. The only audit today is `a11y`; keeping it nested
/// leaves room for other read-only audits without a breaking command rename.
#[derive(Debug, Args)]
pub struct UiAuditArgs {
    #[command(subcommand)]
    pub command: UiAuditCommand,
}

#[derive(Debug, Subcommand)]
pub enum UiAuditCommand {
    /// Audit the hierarchy for structural accessibility issues and, optionally,
    /// WCAG contrast of text against sampled screenshot backgrounds. Read-only.
    A11y(UiAuditA11yArgs),
}

/// Arguments for `ui audit a11y`. Every option maps directly onto
/// `androidforge_ui::a11y::A11yOptions`; omitted numeric values keep the core
/// defaults. No command in this group injects input or mutates the device.
#[derive(Debug, Args)]
pub struct UiAuditA11yArgs {
    /// Skip the missing content-description rule.
    #[arg(long = "no-content-descriptions")]
    pub no_content_descriptions: bool,
    /// Skip the tiny touch-target rule.
    #[arg(long = "no-touch-targets")]
    pub no_touch_targets: bool,
    /// Skip the duplicate accessible-label rule.
    #[arg(long = "no-duplicate-labels")]
    pub no_duplicate_labels: bool,
    /// Minimum touch target in dp (core default: 48).
    #[arg(long)]
    pub min_touch_target_dp: Option<f64>,
    /// Device density in px per dp; bounds are device pixels (default: 1.0).
    #[arg(long)]
    pub density: Option<f64>,
    /// WCAG ratio required for normal text (default: 4.5).
    #[arg(long)]
    pub min_contrast_ratio: Option<f64>,
    /// WCAG ratio required for large text (default: 3.0).
    #[arg(long)]
    pub large_text_contrast_ratio: Option<f64>,
    /// Text size in sp at which the large-text ratio applies (default: 24).
    #[arg(long)]
    pub large_text_min_sp: Option<f64>,
    /// Maximum findings returned (core default: 200).
    #[arg(long)]
    pub max_findings: Option<usize>,
    /// Maximum visible nodes examined (core default: 5000).
    #[arg(long)]
    pub max_nodes: Option<usize>,
    /// Also capture a screenshot and merge per-node region contrast findings.
    #[arg(long)]
    pub contrast: bool,
    /// Text colour used for contrast sampling, in #RRGGBB form. Requires --contrast.
    #[arg(long, required_if_eq("contrast", "true"))]
    pub text_rgb: Option<String>,
    /// Apply the large-text contrast exception to sampled text.
    #[arg(long = "large-text", requires = "contrast")]
    pub large_text: bool,
    /// Screenshot timeout when --contrast is used, for example 10s.
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct UiObserveArgs {
    /// Return the complete parsed hierarchy in addition to the compact view.
    #[arg(long)]
    pub full: bool,
    /// Include a bounded semantic view with Compose test tags and derived roles.
    #[arg(long)]
    pub semantics: bool,
    /// Bounded retries for transient read failures (0-5). This command is read-only.
    #[arg(long, default_value_t = 0)]
    pub retries: u32,
    /// Compare against a previous observation token.
    #[arg(long)]
    pub since: Option<String>,
    /// Filter actionable nodes to this package.
    #[arg(long)]
    pub package: Option<String>,
}

#[derive(Debug, Args)]
pub struct UiDiffArgs {
    /// Previous observation token.
    #[arg(long)]
    pub from: String,
    /// Bounded retries when a fresh target observation is captured (0-5).
    #[arg(long, default_value_t = 0)]
    pub retries: u32,
    /// Optional target observation token. A fresh observation is captured when omitted.
    #[arg(long)]
    pub to: Option<String>,
    /// Include up to 100 added/removed nodes instead of the compact 12.
    #[arg(long)]
    pub full: bool,
}

#[derive(Debug, Args)]
pub struct UiDumpArgs {
    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct UiInspectArgs {
    /// Also write the raw XML to this local path.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct UiFindArgs {
    #[command(flatten)]
    pub selector: SelectorArgs,
}

#[derive(Debug, Args)]
pub struct UiTapArgs {
    #[command(flatten)]
    pub selector: SelectorArgs,
    #[arg(long)]
    pub x: Option<i32>,
    #[arg(long)]
    pub y: Option<i32>,
    /// Hold duration for `ui long-press` (default 900ms).
    #[arg(long)]
    pub duration: Option<u64>,
    /// Capture a screenshot during a long-press.
    #[arg(long = "capture-mid")]
    pub capture_mid: Option<PathBuf>,
    /// Capture a screenshot immediately before dispatching the tap.
    #[arg(long = "capture-before")]
    pub capture_before: Option<PathBuf>,
    /// Capture a screenshot after the tap and postconditions complete.
    #[arg(long = "capture-after")]
    pub capture_after: Option<PathBuf>,
    /// Return a compact action receipt with before/after observations and an effect classification.
    #[arg(long)]
    pub receipt: bool,
    /// Include expanded node details in the action receipt.
    #[arg(long)]
    pub receipt_full: bool,
    #[command(flatten)]
    pub postcondition: PostconditionArgs,
}

#[derive(Debug, Args)]
pub struct UiDragArgs {
    #[arg(long = "from-x")]
    pub from_x: Option<i32>,
    #[arg(long = "from-y")]
    pub from_y: Option<i32>,
    #[arg(long = "to-x")]
    pub to_x: Option<i32>,
    #[arg(long = "to-y")]
    pub to_y: Option<i32>,
    #[arg(long = "from-text")]
    pub from_text: Option<String>,
    #[arg(long = "to-text")]
    pub to_text: Option<String>,
    #[arg(long = "from-id")]
    pub from_id: Option<String>,
    #[arg(long = "to-id")]
    pub to_id: Option<String>,
    /// Start selector expression, for example "text=Save" or "id=fab".
    #[arg(long)]
    pub from: Option<String>,
    /// End selector expression, for example "text=Trash" or "tag=card".
    #[arg(long)]
    pub to: Option<String>,
    /// Total movement time in milliseconds.
    #[arg(long = "duration", default_value_t = 800)]
    pub duration_ms: u64,
    #[arg(long = "hold-before", default_value_t = 0)]
    pub hold_before_ms: u64,
    #[arg(long = "hold-after", default_value_t = 0)]
    pub hold_after_ms: u64,
    #[arg(long = "capture-mid")]
    pub capture_mid: Option<PathBuf>,
    #[command(flatten)]
    pub postcondition: PostconditionArgs,
}

#[derive(Debug, Args)]
pub struct UiAssertNoTextArgs {
    #[arg(long = "text", value_name = "TEXT")]
    pub text: Option<String>,
    #[arg(long = "contains", value_name = "TEXT")]
    pub contains: Option<String>,
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct UiTextArgs {
    #[arg(value_name = "TEXT")]
    pub value: String,
    #[command(flatten)]
    pub selector: SelectorArgs,
    #[arg(long)]
    pub clear: bool,
    #[arg(long)]
    pub hide_keyboard: bool,
    #[arg(long)]
    pub no_verify: bool,
    /// Return a compact action receipt with before/after observations and an effect classification.
    #[arg(long)]
    pub receipt: bool,
    /// Include expanded node details in the action receipt.
    #[arg(long)]
    pub receipt_full: bool,
    #[command(flatten)]
    pub postcondition: PostconditionArgs,
}

#[derive(Debug, Args, Clone, Default)]
pub struct PostconditionArgs {
    /// Wait until visible text contains this value.
    #[arg(long = "wait-for-text", value_name = "TEXT")]
    pub wait_for_text: Vec<String>,
    /// Wait until the selected/focused edit field contains this value.
    #[arg(long = "wait-for-value", value_name = "VALUE")]
    pub wait_for_value: Vec<String>,
    /// Wait until this package is foreground.
    #[arg(long = "wait-for-package", value_name = "PACKAGE")]
    pub wait_for_package: Vec<String>,
    /// Wait until this activity is foreground.
    #[arg(long = "wait-for-activity", value_name = "ACTIVITY")]
    pub wait_for_activity: Vec<String>,
    /// Fail if this text is visible at the end of the wait.
    #[arg(long = "wait-for-absent-text", value_name = "TEXT")]
    pub wait_for_absent_text: Vec<String>,
    /// Postcondition timeout, for example 10s.
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct UiSelectorArgs {
    #[command(flatten)]
    pub selector: SelectorArgs,
}

#[derive(Debug, Args)]
pub struct UiWaitArgs {
    #[command(flatten)]
    pub selector: SelectorArgs,
    #[arg(long)]
    pub not_exists: bool,
    /// Wait until each specified text is absent. Does not require a selector.
    #[arg(long = "for-text-gone", value_name = "TEXT")]
    pub for_text_gone: Vec<String>,
    /// Wait until the hierarchy has stopped changing for one polling interval.
    #[arg(long = "for-idle")]
    pub for_idle: bool,
    /// Wait until the hierarchy stays unchanged for --stable-for.
    #[arg(long = "for-stable")]
    pub for_stable: bool,
    /// Wait until the selected element's bounds stay unchanged for --stable-for. Requires a selector.
    #[arg(long = "for-bounds-stable")]
    pub for_bounds_stable: bool,
    /// Stability window for --for-stable and --for-bounds-stable (default 2s).
    #[arg(long = "stable-for")]
    pub stable_for: Option<String>,
    #[arg(long)]
    pub timeout: Option<String>,
}

/// Arguments for `ui bounds`: one semantic selector, one hierarchy read,
/// eight integer bounds fields. Read-only; never gated by physical consent.
#[derive(Debug, Args)]
pub struct UiBoundsArgs {
    #[command(flatten)]
    pub selector: SelectorArgs,
}

/// Arguments for `ui assert-bounds`. Exactly one of `--bottom-y`,
/// `--bottom-y-gt` or `--bottom-y-lt` must be supplied; they are mutually
/// exclusive. All comparisons use `--tolerance` (default 0). Read-only.
#[derive(Debug, Args)]
pub struct UiAssertBoundsArgs {
    #[command(flatten)]
    pub selector: SelectorArgs,
    /// Expected bottom edge (device pixels) for equality comparison.
    #[arg(long = "bottom-y", conflicts_with_all = ["bottom_y_gt", "bottom_y_lt"])]
    pub bottom_y: Option<i32>,
    /// Assert bottom_y is greater than this value plus --tolerance.
    #[arg(long = "bottom-y-gt", conflicts_with_all = ["bottom_y", "bottom_y_lt"])]
    pub bottom_y_gt: Option<i32>,
    /// Assert bottom_y is less than this value minus --tolerance.
    #[arg(long = "bottom-y-lt", conflicts_with_all = ["bottom_y", "bottom_y_gt"])]
    pub bottom_y_lt: Option<i32>,
    /// Dead-band in device pixels applied to every comparison (default 0).
    #[arg(long, default_value_t = 0)]
    pub tolerance: i32,
}

#[derive(Debug, Args)]
pub struct UiDismissOverlaysArgs {
    /// Maximum number of overlays to dismiss in one call.
    #[arg(long, default_value_t = 3)]
    pub max: usize,
}

#[derive(Debug, Args)]
pub struct UiSettleArgs {
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct UiScrollArgs {
    #[arg(long, default_value = "down")]
    pub direction: String,
    #[command(flatten)]
    pub selector: SelectorArgs,
    /// Return a compact action receipt with before/after observations and an effect classification.
    #[arg(long)]
    pub receipt: bool,
    /// Include expanded node details in the action receipt.
    #[arg(long)]
    pub receipt_full: bool,
    #[command(flatten)]
    pub postcondition: PostconditionArgs,
}

#[derive(Debug, Args)]
pub struct UiScrollToArgs {
    #[command(flatten)]
    pub selector: SelectorArgs,
    #[arg(long, default_value = "down")]
    pub direction: String,
    #[arg(long, default_value_t = 10)]
    pub max_swipes: u32,
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct UiSwipeArgs {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
    #[arg(long, default_value_t = 300)]
    pub duration_ms: u32,
}

#[derive(Debug, Args)]
pub struct UiBackArgs {
    #[arg(long)]
    pub hide_keyboard: bool,
}

#[derive(Debug, Args)]
pub struct UiKeyEventArgs {
    pub keycodes: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum CaptureCommand {
    Screenshot(CaptureScreenshotArgs),
    /// Capture a single preview frame without creating a run or artifact.
    Frame(CaptureFrameArgs),
    /// Sample launch frames, detect the first app frame and optionally assert its colour.
    Launch(CaptureLaunchArgs),
    /// Assert the colour of a fresh, host-only screenshot.
    AssertColor(CaptureAssertColorArgs),
    /// Save, update, compare and list named screenshot baselines. All store
    /// operations are host-side; capturing a fresh screenshot is read-only.
    Baseline {
        #[command(subcommand)]
        command: BaselineCommand,
    },
    /// Check WCAG contrast of a fresh, host-only screenshot region. Read-only.
    Contrast(CaptureContrastArgs),
    Record {
        #[command(subcommand)]
        command: RecordCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum BaselineCommand {
    /// Save a new named baseline. Refuses to overwrite an existing name.
    Save(BaselineSaveArgs),
    /// Replace an existing named baseline deliberately.
    Update(BaselineSaveArgs),
    /// Compare a fresh screenshot with a saved baseline.
    Compare(BaselineCompareArgs),
    /// List saved baseline metadata without touching the device.
    List,
}

#[derive(Debug, Args)]
pub struct BaselineSaveArgs {
    /// Validated baseline name: 1-64 ASCII letters/digits/'.'/'_'/'-'.
    #[arg(long)]
    pub name: String,
    /// Use this local PNG instead of capturing a fresh screenshot.
    #[arg(long)]
    pub source: Option<PathBuf>,
    /// Also copy the saved PNG to this local path.
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Screenshot capture timeout when --source is absent.
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct BaselineCompareArgs {
    /// Saved baseline name to compare against.
    #[arg(long)]
    pub name: String,
    /// Use this local PNG instead of capturing a fresh screenshot.
    #[arg(long)]
    pub current: Option<PathBuf>,
    /// Device-pixel region x1,y1,x2,y2 for comparison.
    #[arg(long)]
    pub region: Option<String>,
    /// Maximum changed-pixel ratio that still passes (0.0 = exact).
    #[arg(long = "max-diff-ratio", default_value_t = 0.0)]
    pub max_diff_ratio: f64,
    /// Per-channel tolerance treated as unchanged (0-255).
    #[arg(long = "diff-threshold", default_value_t = 0)]
    pub diff_threshold: u8,
    /// Optional path for the diff visualization PNG.
    #[arg(long = "diff-output")]
    pub diff_output: Option<PathBuf>,
    /// Also copy the captured PNG to this local path.
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Screenshot capture timeout when --current is absent.
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct CaptureContrastArgs {
    /// Text/foreground colour in #RRGGBB form.
    #[arg(long)]
    pub rgb: String,
    /// Sample a single device pixel at x,y instead of a region.
    #[arg(long, conflicts_with = "region")]
    pub at: Option<String>,
    /// Device-pixel background region x,y,w,h.
    #[arg(long, conflicts_with = "at")]
    pub region: Option<String>,
    /// Use the most frequent colour in the region instead of its average.
    #[arg(long)]
    pub majority: bool,
    /// WCAG ratio threshold (default: 4.5, or 3.0 with --large-text).
    #[arg(long)]
    pub threshold: Option<f64>,
    /// Apply the WCAG large-text 3:1 exception.
    #[arg(long = "large-text")]
    pub large_text: bool,
    /// Optional label recorded in a failed contrast finding.
    #[arg(long)]
    pub label: Option<String>,
    /// Use a local PNG instead of capturing a fresh screenshot.
    #[arg(long)]
    pub source: Option<PathBuf>,
    /// Screenshot capture timeout when --source is absent.
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct CaptureScreenshotArgs {
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long)]
    pub output: Option<PathBuf>,
    #[arg(long)]
    pub timeout: Option<String>,
    /// Baseline PNG to compare the fresh screenshot against.
    #[arg(long)]
    pub compare: Option<PathBuf>,
    /// Device-pixel region x1,y1,x2,y2 for comparison.
    #[arg(long)]
    pub region: Option<String>,
    /// Maximum changed-pixel ratio that still passes (0.0 = exact match).
    #[arg(long = "max-diff-ratio", default_value_t = 0.0)]
    pub max_diff_ratio: f64,
    /// Per-channel tolerance treated as unchanged (0-255).
    #[arg(long = "diff-threshold", default_value_t = 0)]
    pub diff_threshold: u8,
    /// Optional path for a diff visualization PNG.
    #[arg(long = "diff-output")]
    pub diff_output: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct CaptureFrameArgs {
    /// Destination PNG path. Parent directories are created.
    #[arg(long)]
    pub output: PathBuf,
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct CaptureLaunchArgs {
    /// Package whose focus marks the first app frame. Launched by this command.
    pub package: String,
    /// Launcher activity, for example .MainActivity.
    #[arg(long)]
    pub activity: Option<String>,
    /// Requested samples per second (1-30, default 10).
    #[arg(long)]
    pub fps: Option<u32>,
    /// Sampling window, for example 5s (100-30000 ms, default 5s).
    #[arg(long)]
    pub duration: Option<String>,
    /// Fall back to `uiautomator dump /dev/tty` when focus does not match. No device file is created.
    #[arg(long)]
    pub ui_dump_package: bool,
    /// Fail with E_UI_ASSERTION_FAILED when no first app frame is observed.
    #[arg(long)]
    pub expect_first_frame: bool,
    /// Fail with E_UI_ASSERTION_FAILED when the first app frame is blank white or black.
    #[arg(long)]
    pub no_splash: bool,
    /// Expected #RRGGBB colour of the first app frame.
    #[arg(long = "expected-color")]
    pub expected_color: Option<String>,
    /// Which colour to check against --expected-color.
    #[arg(long = "color-sample", value_parser = ["centre", "dominant"])]
    pub color_sample: Option<String>,
    /// Per-channel tolerance for --expected-color (0-255, default 16).
    #[arg(long)]
    pub tolerance: Option<u8>,
}

#[derive(Debug, Args)]
pub struct CaptureAssertColorArgs {
    /// Expected opaque colour in #RRGGBB form.
    #[arg(long)]
    pub rgb: String,
    /// Sample a single device pixel at x,y instead of the image centre.
    #[arg(long, conflicts_with = "region")]
    pub at: Option<String>,
    /// Device-pixel region x,y,w,h to sample.
    #[arg(long, conflicts_with = "at")]
    pub region: Option<String>,
    /// Use the most frequent colour in the sampled region instead of its average.
    #[arg(long)]
    pub majority: bool,
    /// Per-channel tolerance (0-255, default 16).
    #[arg(long)]
    pub tolerance: Option<u8>,
    /// Screenshot capture timeout, for example 10s.
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum RecordCommand {
    Start(RecordStartArgs),
    Stop,
    Status,
}

#[derive(Debug, Args)]
pub struct RecordStartArgs {
    /// Maximum recording time in seconds.
    #[arg(long)]
    pub time_limit: Option<u64>,
    #[arg(long)]
    pub bit_rate: Option<u64>,
    #[arg(long)]
    pub size: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum ArtifactsCommand {
    /// Scan stored artifact text for suspected secrets without printing their values.
    ScanSecrets(ArtifactsScanSecretsArgs),
}

#[derive(Debug, Args)]
pub struct ArtifactsScanSecretsArgs {
    /// Scan this directory directly instead of the project's `.artifacts` store.
    #[arg(long)]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct DeviceShellArgs {
    /// Require every command prefix to match an allowlist and no deny prefix. No physical grant is needed.
    #[arg(long)]
    pub read_only: bool,
    /// Additional read-only command prefix. Repeatable; replaces the conservative default list when present.
    #[arg(long)]
    pub allow: Vec<String>,
    /// Refuse a command whose tokens start with (or contain) this prefix. Repeatable.
    #[arg(long)]
    pub deny: Vec<String>,
    /// Per-command timeout, for example 30s.
    #[arg(long)]
    pub timeout: Option<String>,
    /// Exact remote argv. Everything after `--` is passed to `adb shell` as separate, quoted arguments.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
    pub command: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum DeviceInputCommand {
    /// Tap exact device-pixel coordinates through `UiManager::tap_xy`.
    Tap(DeviceInputTapArgs),
}

#[derive(Debug, Args)]
pub struct DeviceInputTapArgs {
    /// Device-pixel X coordinate.
    pub x: i32,
    /// Device-pixel Y coordinate.
    pub y: i32,
}

#[derive(Debug, Subcommand)]
pub enum DiagnosticsCommand {
    Collect(DiagnosticsCollectArgs),
    App {
        package: String,
    },
    Logcat {
        #[command(subcommand)]
        command: LogcatCommand,
    },
    Crashes {
        package: String,
    },
    Inspect(DiagnosticsInspectArgs),
    /// Measure startup, jank, memory and CPU explicitly.
    #[command(alias = "perf")]
    Performance(DiagnosticsPerformanceArgs),
    /// Parse frame timing from `dumpsys gfxinfo` and write frames/report.json.
    Frames(DiagnosticsFramesArgs),
}

#[derive(Debug, Args)]
pub struct DiagnosticsFramesArgs {
    /// Package whose gfxinfo frame counters are collected.
    pub package: String,
    /// Reset gfxinfo counters with `dumpsys gfxinfo <package> reset` first.
    #[arg(long)]
    pub reset: bool,
    /// Explicit sampling window to wait before collection, for example 5s.
    #[arg(long)]
    pub duration: Option<String>,
    /// Per-command timeout, for example 30s.
    #[arg(long)]
    pub timeout: Option<String>,
    /// Baseline frames/report.json to compare against.
    #[arg(long)]
    pub baseline: Option<PathBuf>,
    /// Allowed increase in jank percentage points before a regression (default 0).
    #[arg(long)]
    pub max_janky: Option<f64>,
    /// Allowed increase in the 99th percentile frame time in ms (default 0).
    #[arg(long)]
    pub max_p99: Option<f64>,
    /// Allowed increase in the longest frame time in ms (default 0).
    #[arg(long)]
    pub max_frame_ms: Option<f64>,
}

#[derive(Debug, Args)]
pub struct DiagnosticsPerformanceArgs {
    /// Package to measure. Defaults to [project].package.
    pub package: Option<String>,
    /// Explicit launcher activity, for example com.example/.MainActivity.
    #[arg(long)]
    pub activity: Option<String>,
    /// Force-stop the application before measuring. This is an explicit cold-start request.
    #[arg(long)]
    pub cold: bool,
    /// Wait after launch before sampling frames, memory and CPU, for example 2s.
    #[arg(long)]
    pub settle: Option<String>,
    /// Per-command timeout, for example 30s.
    #[arg(long)]
    pub timeout: Option<String>,
    /// Wake the screen before measuring. Explicit opt-in only; never implicit.
    #[arg(long)]
    pub wake: bool,
    /// Skip the screen-on preflight. Only use when a sleeping display is acceptable evidence.
    #[arg(long)]
    pub no_screen_check: bool,
    /// Minimum rendered frames before a jank percentage is reported.
    #[arg(long, default_value_t = androidforge_diagnostics::DEFAULT_MIN_FRAMES)]
    pub min_frames: u64,
    /// Sample frames for this duration after launch, for example 5s.
    #[arg(long)]
    pub frame_window: Option<String>,
    /// Additional warmup before frame sampling, for example 2s.
    #[arg(long)]
    pub warmup: Option<String>,
}

#[derive(Debug, Args)]
pub struct DiagnosticsCollectArgs {
    #[arg(long)]
    pub package: Option<String>,
    #[arg(long)]
    pub full: bool,
    #[arg(long)]
    pub log_lines: Option<usize>,
}

#[derive(Debug, Subcommand)]
pub enum LogcatCommand {
    Tail(LogcatTailArgs),
    /// Read a bounded logcat window without creating a run or artifact.
    Peek(LogcatPeekArgs),
    /// Read a bounded window and return structured filtered records.
    Query(LogcatQueryArgs),
    /// Stream log records, crashes and ANRs as NDJSON.
    Watch(LogcatWatchArgs),
    Start(LogcatStartArgs),
    Stop,
    Status,
}

#[derive(Debug, Args)]
pub struct LogcatQueryArgs {
    /// Literal token prefixes to redact everywhere before output or storage. Repeatable or comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub redact_patterns: Vec<String>,
    /// Bounded retries for transient read failures (0-5).
    #[arg(long, default_value_t = 0)]
    pub retries: u32,
    /// Keep the package's process logs plus lines that mention the package name.
    #[arg(long)]
    pub package: Option<String>,
    #[arg(long, default_value_t = 2000)]
    pub lines: usize,
    /// Minimum Android log priority: verbose, debug, info, warn, error, fatal.
    #[arg(long)]
    pub level: Option<String>,
    /// Only records at or after this device timestamp (MM-DD HH:MM:SS.mmm).
    #[arg(long)]
    pub since: Option<String>,
    /// Only records at or before this device timestamp (MM-DD HH:MM:SS.mmm).
    #[arg(long)]
    pub until: Option<String>,
    /// Regular expression applied to tag and message.
    #[arg(long)]
    pub grep: Option<String>,
    #[arg(long, default_value_t = 500)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct LogcatWatchArgs {
    /// Literal token prefixes to redact before each NDJSON record is emitted. Repeatable or comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub redact_patterns: Vec<String>,
    /// Keep the package's process logs plus lines that mention the package name.
    /// Package PIDs are sampled at the start of the watch; if the app restarts
    /// mid-stream, only lines mentioning the package name are guaranteed to
    /// match.
    #[arg(long)]
    pub package: Option<String>,
    #[arg(long)]
    pub level: Option<String>,
    /// Regular expression applied to tag and message.
    #[arg(long)]
    pub grep: Option<String>,
    /// Emit only crash/FATAL events (plus any --grep/--anr match).
    #[arg(long)]
    pub crash: bool,
    /// Emit only ANR events (plus any --grep/--crash match).
    #[arg(long)]
    pub anr: bool,
    #[arg(long, default_value = "60s")]
    pub timeout: String,
    #[arg(long, default_value_t = 100)]
    pub max_events: usize,
}

#[derive(Debug, Args)]
pub struct LogcatTailArgs {
    /// Literal token prefixes to redact before printing, emitting or storing. Repeatable or comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub redact_patterns: Vec<String>,
    /// Bounded retries for transient read failures (0-5). Logcat reads are read-only.
    #[arg(long, default_value_t = 0)]
    pub retries: u32,
    #[arg(long)]
    pub package: Option<String>,
    #[arg(long)]
    pub lines: Option<usize>,
    #[arg(long)]
    pub duration: Option<String>,
    #[arg(long)]
    pub crash: bool,
}

#[derive(Debug, Args)]
pub struct LogcatPeekArgs {
    /// Literal token prefixes to redact before printing or emitting. Repeatable or comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub redact_patterns: Vec<String>,
    /// Bounded retries for transient read failures (0-5).
    #[arg(long, default_value_t = 0)]
    pub retries: u32,
    #[arg(long)]
    pub package: Option<String>,
    #[arg(long)]
    pub lines: Option<usize>,
    /// Minimum Android log priority: verbose, debug, info, warn, error, fatal.
    #[arg(long)]
    pub level: Option<String>,
    /// Only print lines at or after this device timestamp (MM-DD HH:MM:SS.mmm).
    #[arg(long)]
    pub since: Option<String>,
}

#[derive(Debug, Args)]
pub struct LogcatStartArgs {
    #[arg(long)]
    pub package: Option<String>,
    /// Literal token prefixes persisted for `logcat stop` redaction. Repeatable or comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub redact_patterns: Vec<String>,
}

#[derive(Debug, Args)]
pub struct DiagnosticsInspectArgs {
    #[arg(value_parser = ["activity", "jobs", "alarms", "package", "notifications", "deviceidle", "battery", "connectivity", "jobscheduler", "alarm", "notification"])]
    pub service: String,
    #[arg(long)]
    pub package: Option<String>,
}

#[derive(Debug, Args, Default, Clone)]
pub struct NotificationSelectorArgs {
    #[arg(long)]
    pub package: Option<String>,
    #[arg(long)]
    pub tag: Option<String>,
    #[arg(long)]
    pub channel: Option<String>,
    #[arg(long)]
    pub id: Option<i32>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub text: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum NotificationsCommand {
    List {
        #[arg(long)]
        package: Option<String>,
    },
    Wait(NotificationWaitArgs),
    Assert(NotificationWaitArgs),
    Channels {
        #[arg(long)]
        package: Option<String>,
    },
    Tap(NotificationSelectorArgs),
    Clear(NotificationSelectorArgs),
}

#[derive(Debug, Args)]
pub struct NotificationWaitArgs {
    #[command(flatten)]
    pub selector: NotificationSelectorArgs,
    #[arg(long)]
    pub not_exists: bool,
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum JobsCommand {
    List {
        #[arg(long)]
        package: Option<String>,
    },
    Run {
        package: String,
        job_id: String,
    },
    Cancel {
        package: String,
        job_id: String,
    },
    Wait(JobWaitArgs),
    Assert(JobWaitArgs),
}

#[derive(Debug, Args)]
pub struct JobWaitArgs {
    #[arg(long)]
    pub package: Option<String>,
    #[arg(long)]
    pub job_id: Option<String>,
    #[arg(long)]
    pub state: Option<String>,
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum AlarmsCommand {
    Next {
        #[arg(long)]
        package: Option<String>,
    },
    List {
        #[arg(long)]
        package: Option<String>,
    },
    Wait {
        #[arg(long)]
        package: Option<String>,
        #[arg(long)]
        timeout: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum PermissionsCommand {
    Check {
        #[arg(long)]
        package: Option<String>,
        permission: String,
    },
    Grant {
        #[arg(long)]
        package: Option<String>,
        permission: String,
    },
    Revoke {
        #[arg(long)]
        package: Option<String>,
        permission: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum AppOpsCommand {
    Get {
        #[arg(long)]
        package: Option<String>,
        operation: String,
    },
    Set {
        #[arg(long)]
        package: Option<String>,
        operation: String,
        mode: String,
    },
    Reset {
        #[arg(long)]
        package: Option<String>,
        operation: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum DozeCommand {
    Status,
    Force,
    Unforce,
}

#[derive(Debug, Subcommand)]
pub enum StandbyCommand {
    Get {
        #[arg(long)]
        package: Option<String>,
    },
    Set {
        #[arg(long)]
        package: Option<String>,
        bucket: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum BatteryCommand {
    Status,
    Unplug,
    Reset,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum NetworkCommand {
    Status,
    Airplane {
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long)]
        disable: bool,
    },
    Wifi {
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long)]
        disable: bool,
    },
    Data {
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long)]
        disable: bool,
    },
    /// Read, set or clear the global HTTP proxy.
    Proxy {
        #[command(subcommand)]
        command: NetworkProxyCommand,
    },
    /// Apply a named network profile and record the previous state.
    Profile {
        #[command(subcommand)]
        command: NetworkProfileCommand,
    },
    /// Restore network/proxy state recorded in the current session.
    Restore,
    /// Manage `adb reverse` port mappings for this session.
    Reverse {
        #[command(subcommand)]
        command: NetworkReverseCommand,
    },
    /// Run a bounded local mock HTTP(S) server and optionally reverse it to the device.
    Mock {
        #[command(subcommand)]
        command: NetworkMockCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum NetworkReverseCommand {
    Add(NetworkReverseAddArgs),
    List,
    Remove { remote: String },
    Clear,
}

#[derive(Debug, Args)]
pub struct NetworkReverseAddArgs {
    /// Remote (device) port mapping, for example tcp:8080.
    pub remote: String,
    /// Local host port mapping; defaults to the same tcp:PORT.
    #[arg(long)]
    pub local: Option<String>,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)] // Serving carries the full bounded option set; listing is empty.
pub enum NetworkMockCommand {
    Serve(NetworkMockServeArgs),
    /// List the built-in, loopback-only network mock scenarios. Read-only.
    Scenarios,
}

#[derive(Debug, Args)]
pub struct NetworkMockServeArgs {
    /// Built-in scenario to serve: newer-release, offline, bad-digest or no-APK.
    /// Scenarios are loopback-only, bounded, redacted and host-only (no --reverse).
    #[arg(long)]
    pub scenario: Option<String>,
    /// Local port to listen on.
    #[arg(long, default_value_t = 8080)]
    pub port: u16,
    /// Bind address. Defaults to loopback so the mock is not exposed to the LAN.
    #[arg(long, default_value = "127.0.0.1")]
    pub bind: String,
    /// Explicitly allow a non-loopback bind address. The mock has no authentication.
    #[arg(long = "allow-external-bind")]
    pub allow_external_bind: bool,
    /// JSON routes file: {"routes":[{"method":"GET","path":"/health","status":200,"body":"ok"}]}.
    #[arg(long)]
    pub routes: Option<PathBuf>,
    /// Inline route: METHOD PATH=STATUS[:BODY]. May be repeated.
    #[arg(long = "route")]
    pub route: Vec<String>,
    #[arg(long = "default-status", default_value_t = 404)]
    pub default_status: u16,
    #[arg(long = "default-body")]
    pub default_body: Option<String>,
    /// Artificial response delay in milliseconds.
    #[arg(long = "delay-ms", default_value_t = 0)]
    pub delay_ms: u64,
    /// PEM certificate chain for local TLS. Requires --tls-key.
    #[arg(long = "tls-cert")]
    pub tls_cert: Option<PathBuf>,
    /// PEM private key for local TLS.
    #[arg(long = "tls-key")]
    pub tls_key: Option<PathBuf>,
    /// NDJSON request log path. Defaults to a run artifact.
    #[arg(long)]
    pub log: Option<PathBuf>,
    /// Stop after N requests; 0 runs until interrupted.
    #[arg(long = "max-requests", default_value_t = 0)]
    pub max_requests: u64,
    /// Stop after a duration such as 30s or 5m.
    #[arg(long)]
    pub duration: Option<String>,
    /// Add `adb reverse tcp:PORT tcp:PORT` before serving and remove it on exit.
    #[arg(long)]
    pub reverse: bool,
    /// Maximum request body bytes retained in the log.
    #[arg(long = "max-body-bytes", default_value_t = 65536)]
    pub max_body_bytes: usize,
}

#[derive(Debug, Subcommand)]
pub enum NetworkProxyCommand {
    Get,
    Set { proxy: String },
    Clear,
}

#[derive(Debug, Subcommand)]
pub enum NetworkProfileCommand {
    List,
    Apply { name: String },
}

#[derive(Debug, Subcommand)]
pub enum TimeCommand {
    Get,
    Set(TimeSetArgs),
    Restore,
}

#[derive(Debug, Args)]
pub struct TimeSetArgs {
    #[arg(long)]
    pub epoch: Option<i64>,
    #[arg(long)]
    pub iso: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum HookCommand {
    List,
    Run { name: String },
}

#[derive(Debug, Subcommand)]
pub enum FixtureCommand {
    List,
    Validate {
        name: String,
    },
    Run(FixtureRunArgs),
    /// Apply only the declarative state block of a fixture (debug-only).
    Apply(FixtureStateArgs),
    /// Remove only the files created by a fixture's declarative state.
    Reset(FixtureStateArgs),
}

#[derive(Debug, Args)]
pub struct FixtureStateArgs {
    pub name: String,
    /// Package whose app-private state is changed. Defaults to the project adapter package.
    #[arg(long)]
    pub package: Option<String>,
    #[arg(long = "var")]
    pub vars: Vec<String>,
}

#[derive(Debug, Args)]
pub struct FixtureRunArgs {
    pub name: String,
    #[arg(long)]
    pub phase: Vec<String>,
    #[arg(long)]
    pub apk: Option<PathBuf>,
    #[arg(long = "var")]
    pub vars: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum ConformanceCommand {
    Run(ConformanceRunArgs),
}

#[derive(Debug, Args)]
pub struct ConformanceRunArgs {
    #[arg(long)]
    pub flow: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum ScenarioCommand {
    /// Execute an allowlisted scenario. Mutating steps need --yes plus the
    /// scoped physical flags for the device risks the scenario exercises.
    Run(ScenarioRunArgs),
    /// Parse and validate a scenario manifest without touching a device.
    Validate(ScenarioValidateArgs),
}

#[derive(Debug, Args)]
pub struct ScenarioRunArgs {
    /// Scenario manifest path. `.json` (or a JSON object body) is parsed as
    /// JSON; every other file is parsed as TOML.
    pub file: PathBuf,
    /// Override the manifest's repeat count for this run.
    #[arg(long)]
    pub repeat: Option<u32>,
}

#[derive(Debug, Args)]
pub struct ScenarioValidateArgs {
    /// Scenario manifest path. `.json` (or a JSON object body) is parsed as
    /// JSON; every other file is parsed as TOML.
    pub file: PathBuf,
}

#[derive(Debug, Subcommand)]
pub enum TestCommand {
    Maestro {
        flow: PathBuf,
        #[arg(long)]
        timeout: Option<String>,
    },
    Instrumentation {
        package: String,
        #[arg(long)]
        runner: Option<String>,
        #[arg(long)]
        class: Option<String>,
        #[arg(long)]
        method: Option<String>,
        #[arg(long = "extra")]
        extras: Vec<String>,
        #[arg(long)]
        timeout: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum RunCommand {
    Start(RunStartArgs),
    End(RunEndArgs),
    Status,
    Exec(RunExecArgs),
    /// Record a named resume point in a run.
    Checkpoint(RunCheckpointArgs),
    /// Reopen a terminal run and continue appending to it.
    Resume(RunResumeArgs),
    /// Print the run's NDJSON event stream; --follow tails it.
    Events(RunEventsArgs),
}

#[derive(Debug, Args)]
pub struct RunCheckpointArgs {
    pub name: String,
    #[arg(long)]
    pub note: Option<String>,
}

#[derive(Debug, Args)]
pub struct RunResumeArgs {
    /// Run id; defaults to --run, ANDROIDFORGE_RUN_ID or the active run.
    pub run: Option<String>,
    #[arg(long = "from-checkpoint")]
    pub from_checkpoint: Option<String>,
}

#[derive(Debug, Args)]
pub struct RunEventsArgs {
    /// Run id; defaults to --run, ANDROIDFORGE_RUN_ID or the active run.
    pub run: Option<String>,
    #[arg(long)]
    pub follow: bool,
    #[arg(long, default_value = "300s")]
    pub timeout: String,
}

#[derive(Debug, Args)]
pub struct RunStartArgs {
    #[arg(long)]
    pub name: String,
    /// Free-form label stored on the run report and metadata. Repeat to attach several labels.
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// KEY=VALUE metadata stored on the run. Secret-looking keys are rejected. Repeatable.
    #[arg(long = "meta")]
    pub meta: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RunEndArgs {
    /// Optional explicit status. Without it, failed steps make the run failed.
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long)]
    pub exit_code: Option<i32>,
    /// Explicitly allow --status passed even when the report contains failed steps.
    #[arg(long)]
    pub allow_failures: bool,
}

#[derive(Debug, Args)]
pub struct RunExecArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub capture_screen: bool,
    #[arg(long)]
    pub capture_logcat: bool,
    #[arg(long)]
    pub timeout: Option<String>,
    #[arg(last = true, required = true, allow_hyphen_values = true)]
    pub program: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum MaintenanceCommand {
    /// Show storage paths, usage and cleanup candidates.
    Summary,
    /// Prune old terminal sessions and run artifacts.
    Clean(MaintenanceCleanArgs),
}

#[derive(Debug, Args)]
pub struct MaintenanceCleanArgs {
    #[arg(long)]
    pub older_than: Option<String>,
    #[arg(long)]
    pub keep_last: Option<usize>,
    /// Protect runs carrying any of these tags from retention. Repeatable;
    /// OR semantics. Using this flag without `--yes` forces a dry run.
    #[arg(long = "keep-tag")]
    pub keep_tag: Vec<String>,
    /// Include released/failed/abandoned session tombstones.
    #[arg(long)]
    pub sessions: bool,
    /// Include historical runs whose reports are terminal.
    #[arg(long)]
    pub runs: bool,
}

#[derive(Debug, Subcommand)]
pub enum ReportCommand {
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Include runs from the current directory and ancestor project artifact stores.
        #[arg(long)]
        all: bool,
        /// Only list runs carrying every one of these tags. Repeatable.
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Only list runs started at or after this RFC3339 timestamp or duration ago.
        #[arg(long)]
        since: Option<String>,
    },
    Show {
        run: String,
    },
    Path {
        run: String,
    },
    Prune(ReportPruneArgs),
    /// Reconcile runs left in the running state by a dead or cancelled operation.
    Reconcile(ReportReconcileArgs),
    /// Compare a run with a compatible baseline.
    Compare(ReportCompareArgs),
    /// Analyze recent runs for flaky steps.
    Flaky(ReportFlakyArgs),
    /// Explain a failure deterministically and return exact next commands.
    #[command(alias = "why")]
    Explain(ReportExplainArgs),
    /// Show every stored occurrence of a deterministic failure fingerprint.
    History(ReportHistoryArgs),
    /// Aggregate recurring, regressed and new failure fingerprints across local runs.
    Recurrences(ReportRecurrencesArgs),
    /// Create a portable diagnostic bundle or a Markdown/JSON evidence summary.
    Bundle(ReportBundleArgs),
    /// Emit a copyable PowerShell reproduction script from a run.
    Repro(ReportReproArgs),
    Import {
        #[command(subcommand)]
        command: ReportImportCommand,
    },
}

#[derive(Debug, Args)]
pub struct ReportReproArgs {
    pub run: String,
    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ReportPruneArgs {
    #[arg(long)]
    pub older_than: Option<String>,
    #[arg(long)]
    pub keep_last: Option<usize>,
}

#[derive(Debug, Args)]
pub struct ReportReconcileArgs {
    #[arg(long)]
    pub run: Option<String>,
    #[arg(long)]
    pub latest_running: bool,
    /// Outcome for explicitly targeted runs: interrupted, cancelled, timed-out or abandoned.
    #[arg(long)]
    pub outcome: Option<String>,
}

#[derive(Debug, Args)]
pub struct ReportCompareArgs {
    pub run: String,
    #[arg(long)]
    pub baseline: Option<String>,
    /// Baseline runs must carry every one of these tags. Repeatable.
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Baseline runs must have started at or after this RFC3339 timestamp or duration ago.
    #[arg(long)]
    pub since: Option<String>,
    /// Fail with E_PERFORMANCE_REGRESSION (exit 11) when a budget check fails.
    #[arg(long)]
    pub fail_on_regression: bool,
    /// Full PerformanceBudget as a JSON object; supports absolute, delta and named-metric thresholds.
    #[arg(long = "budget")]
    pub budget: Option<String>,
    /// Convenience threshold: maximum current startup time in milliseconds.
    #[arg(long = "max-startup-ms")]
    pub max_startup_ms: Option<f64>,
    /// Convenience threshold: maximum jank percentage.
    #[arg(long = "max-jank-percent")]
    pub max_jank_percent: Option<f64>,
    /// Convenience threshold: maximum p99 frame time in milliseconds.
    #[arg(long = "max-p99-ms")]
    pub max_p99_ms: Option<f64>,
    /// Convenience threshold: maximum longest frame time in milliseconds.
    #[arg(long = "max-frame-ms")]
    pub max_frame_ms: Option<f64>,
    /// Convenience threshold: maximum APK size in bytes.
    #[arg(long = "max-apk-size-bytes")]
    pub max_apk_size_bytes: Option<f64>,
    /// Convenience threshold: maximum number of failed tests.
    #[arg(long = "max-failed-tests")]
    pub max_failed_tests: Option<f64>,
}

#[derive(Debug, Args)]
pub struct ReportExplainArgs {
    /// Run id, `latest` for the newest completed run, or `latest-failure` for
    /// the newest failed/partial run in this project.
    pub run: String,
    /// Include expanded evidence and metadata.
    #[arg(long)]
    pub full: bool,
    /// Emit a human-readable Markdown explanation instead of JSON.
    #[arg(long)]
    pub markdown: bool,
    /// `latest`/`latest-failure` must carry every one of these tags. Repeatable.
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// `latest`/`latest-failure` must have started at or after this RFC3339 timestamp or duration ago.
    #[arg(long)]
    pub since: Option<String>,
}

#[derive(Debug, Args)]
pub struct ReportFlakyArgs {
    #[arg(long, default_value_t = 30)]
    pub limit: usize,
    #[arg(long, default_value_t = 3)]
    pub min_runs: usize,
}

#[derive(Debug, Args)]
pub struct ReportHistoryArgs {
    /// Deterministic fingerprint such as sha256:<hex>.
    #[arg(long)]
    pub fingerprint: String,
    /// Maximum number of occurrences to return.
    #[arg(long, default_value_t = 100)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct ReportRecurrencesArgs {
    /// Only fingerprints last seen within this duration, for example 30d.
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long)]
    pub package: Option<String>,
    /// Filter by status: new, recurring, regressed or unknown.
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long)]
    pub device: Option<String>,
    #[arg(long = "app-version")]
    pub app_version: Option<String>,
    /// Include only fingerprints for this project root. Defaults to the current project.
    #[arg(long)]
    pub project_root: Option<String>,
    /// Include failures from every discovered local project instead of only the current project.
    #[arg(long)]
    pub all_projects: bool,
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct ReportBundleArgs {
    /// Run id, `latest` for the newest completed run, or `latest-failure` for
    /// the newest failed/partial run in this project.
    pub run: String,
    /// Output path for a ZIP, JSON manifest or Markdown file. ZIP defaults to .artifacts/bundles/<run>-evidence.zip.
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Produce a Markdown evidence summary instead of a ZIP.
    #[arg(long)]
    pub markdown: bool,
    /// Output format: zip, json or markdown.
    #[arg(long)]
    pub format: Option<String>,
    /// Include screenshots found in the run artifacts.
    #[arg(long)]
    pub include_screenshots: bool,
    /// Include screen recordings found in the run artifacts.
    #[arg(long)]
    pub include_recordings: bool,
    /// Include the full stored logcat instead of a bounded excerpt.
    #[arg(long = "include-full-logs")]
    pub include_full_logs: bool,
    /// Emit the manifest, summary and explanation only; exclude every run artifact.
    #[arg(long)]
    pub metadata_only: bool,
    /// Maximum uncompressed payload size in bytes.
    #[arg(long, default_value_t = 26_214_400)]
    pub max_bytes: u64,
    /// Force text redaction even when configured redaction is disabled, then refuse any bundle with a remaining suspected secret.
    #[arg(long)]
    pub redact_secrets: bool,
    /// `latest`/`latest-failure` must carry every one of these tags. Repeatable.
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// `latest`/`latest-failure` must have started at or after this RFC3339 timestamp or duration ago.
    #[arg(long)]
    pub since: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum ReportImportCommand {
    Junit {
        /// One or more XML files, or directories containing XML files.
        #[arg(required = true, num_args = 1..)]
        paths: Vec<PathBuf>,
    },
}

#[derive(Debug, Args)]
pub struct AdbArgs {
    #[arg(long)]
    pub timeout: Option<String>,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
    pub args: Vec<String>,
}

impl SelectorArgs {
    pub fn into_selector(self) -> androidforge_ui::Selector {
        androidforge_ui::Selector {
            text: self.text,
            text_contains: self.contains,
            content_desc: self.content_desc,
            content_desc_contains: self.content_desc_contains,
            resource_id: self.resource_id.or(self.test_tag),
            class: self.class,
            index: self.index,
            clickable: self.clickable.then_some(true),
            enabled: self.enabled.then_some(true),
            focused: self.focused.then_some(true),
            scrollable: self.scrollable.then_some(true),
            checked: self.checked.then_some(true),
            visible: self.visible.then_some(true),
            role: self.role,
            text_regex: self.text_regex,
            within: self.within,
            above: self.above,
            below: self.below,
            left_of: self.left_of,
            right_of: self.right_of,
            region: self.region,
        }
    }

    pub fn has_selector(&self) -> bool {
        self.text.is_some()
            || self.contains.is_some()
            || self.content_desc.is_some()
            || self.content_desc_contains.is_some()
            || self.resource_id.is_some()
            || self.test_tag.is_some()
            || self.class.is_some()
            || self.clickable
            || self.enabled
            || self.focused
            || self.scrollable
            || self.checked
            || self.visible
            || self.role.is_some()
            || self.text_regex.is_some()
            || self.within.is_some()
            || self.above.is_some()
            || self.below.is_some()
            || self.left_of.is_some()
            || self.right_of.is_some()
            || self.region.is_some()
    }

    pub fn has_ref(&self) -> bool {
        self.reference.is_some()
    }

    pub fn has_non_ref_selector(&self) -> bool {
        self.has_selector() || self.index.is_some()
    }
}

impl NotificationSelectorArgs {
    pub fn into_selector(self) -> androidforge_diagnostics::NotificationSelector {
        androidforge_diagnostics::NotificationSelector {
            package: self.package,
            tag: self.tag,
            channel: self.channel,
            id: self.id,
            title_contains: self.title,
            text_contains: self.text,
        }
    }
}
