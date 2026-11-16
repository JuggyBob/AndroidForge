use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

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

    /// Trace external command argv, timeouts and exit codes to stderr.
    #[arg(long, global = true)]
    pub trace: bool,

    /// Print the planned operation without changing device or project state.
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Confirm a destructive operation. Required for data-clearing and restore commands.
    #[arg(long, global = true)]
    pub yes: bool,

    /// Session id. Also read from ANDROIDFORGE_SESSION.
    #[arg(long, global = true)]
    pub session: Option<String>,

    /// Device serial. Must be explicit when multiple devices are connected.
    #[arg(long, global = true)]
    pub serial: Option<String>,

    /// Project directory containing .androidforge/project.toml.
    #[arg(long, global = true)]
    pub project: Option<PathBuf>,

    /// Append to an existing run id. Also read from ANDROIDFORGE_RUN_ID.
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

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Inspect host, SDK, ADB, emulator, tools, AVDs, disks and sessions.
    Doctor {
        #[command(subcommand)]
        command: Option<DoctorCommand>,
    },
    /// Compact, agent-oriented snapshot of the project, session, device, app and screen.
    Context(ContextArgs),
    /// Run a transparent high-level agent workflow.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Report implemented, best-effort and unsupported capabilities.
    Capabilities,
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
    /// Discover and wait for Android devices.
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
    /// Run the transparent ensure/inspect/install/conformance/explain pipeline.
    Test(AgentTestArgs),
}

#[derive(Debug, Args)]
pub struct AgentTestArgs {
    /// APK to inspect and optionally install before testing.
    #[arg(long)]
    pub apk: Option<PathBuf>,
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
    /// Not implemented. Kept as an explicit structured refusal rather than a fake clone.
    Clone {
        source: String,
        #[arg(long)]
        name: Option<String>,
    },
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
    Install(AppInstallArgs),
    Uninstall(AppUninstallArgs),
    ClearData(AppPackageArgs),
    Launch(AppLaunchArgs),
    ForceStop(AppPackageArgs),
    Kill(AppPackageArgs),
    Status(AppStatusArgs),
    Info(AppPackageArgs),
    List(AppListArgs),
    FilePush(AppFilePushArgs),
    FilePull(AppFilePullArgs),
    DataBackup(AppDataArgs),
    DataRestore(AppDataArgs),
}

#[derive(Debug, Args)]
pub struct AppInstallArgs {
    pub apk: PathBuf,
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
    #[arg(long)]
    pub no_wake: bool,
}

#[derive(Debug, Args)]
pub struct AppStatusArgs {
    pub package: String,
    #[arg(long)]
    pub expect: Option<String>,
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

#[derive(Debug, Args)]
pub struct AppDataArgs {
    pub package: String,
    pub path: PathBuf,
}

#[derive(Debug, Subcommand)]
pub enum ApkCommand {
    Inspect { apk: PathBuf },
    Verify(ApkVerifyArgs),
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

#[derive(Debug, Args, Default, Clone)]
pub struct SelectorArgs {
    #[arg(long)]
    pub text: Option<String>,
    #[arg(long)]
    pub contains: Option<String>,
    #[arg(long = "content-desc")]
    pub content_desc: Option<String>,
    #[arg(long = "content-desc-contains", alias = "desc-contains")]
    pub content_desc_contains: Option<String>,
    #[arg(long = "resource-id")]
    pub resource_id: Option<String>,
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
}

#[derive(Debug, Subcommand)]
pub enum UiCommand {
    Dump(UiDumpArgs),
    /// Dump and return the parsed hierarchy without creating a run artifact.
    Inspect(UiInspectArgs),
    /// Capture a compact semantic observation and persist an observation token.
    Observe(UiObserveArgs),
    /// Compare a previous observation with a new or stored observation.
    Diff(UiDiffArgs),
    Find(UiFindArgs),
    Tap(UiTapArgs),
    LongPress(UiTapArgs),
    Text(UiTextArgs),
    Clear(UiSelectorArgs),
    Wait(UiWaitArgs),
    Assert(UiWaitArgs),
    Settle(UiSettleArgs),
    Scroll(UiScrollArgs),
    ScrollTo(UiScrollToArgs),
    Swipe(UiSwipeArgs),
    Back(UiBackArgs),
    Home,
    Keyevent(UiKeyEventArgs),
    HideKeyboard,
    Focused,
}

#[derive(Debug, Args)]
pub struct UiObserveArgs {
    /// Return the complete parsed hierarchy in addition to the compact view.
    #[arg(long)]
    pub full: bool,
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
    #[command(flatten)]
    pub postcondition: PostconditionArgs,
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
    #[arg(long)]
    pub timeout: Option<String>,
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
    Record {
        #[command(subcommand)]
        command: RecordCommand,
    },
}

#[derive(Debug, Args)]
pub struct CaptureScreenshotArgs {
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long)]
    pub output: Option<PathBuf>,
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct CaptureFrameArgs {
    /// Destination PNG path. Parent directories are created.
    #[arg(long)]
    pub output: PathBuf,
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
    #[arg(long)]
    pub time_limit: Option<u64>,
    #[arg(long)]
    pub bit_rate: Option<u64>,
    #[arg(long)]
    pub size: Option<String>,
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
    Start(LogcatStartArgs),
    Stop,
    Status,
}

#[derive(Debug, Args)]
pub struct LogcatTailArgs {
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
    Validate { name: String },
    Run(FixtureRunArgs),
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
}

#[derive(Debug, Args)]
pub struct RunStartArgs {
    #[arg(long)]
    pub name: String,
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
    Explain(ReportExplainArgs),
    Import {
        #[command(subcommand)]
        command: ReportImportCommand,
    },
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
}

#[derive(Debug, Args)]
pub struct ReportExplainArgs {
    pub run: String,
    /// Include expanded evidence and metadata.
    #[arg(long)]
    pub full: bool,
    /// Emit a human-readable Markdown explanation instead of JSON.
    #[arg(long)]
    pub markdown: bool,
}

#[derive(Debug, Args)]
pub struct ReportFlakyArgs {
    #[arg(long, default_value_t = 30)]
    pub limit: usize,
    #[arg(long, default_value_t = 3)]
    pub min_runs: usize,
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
            resource_id: self.resource_id,
            class: self.class,
            index: self.index,
            clickable: self.clickable.then_some(true),
            enabled: self.enabled.then_some(true),
            focused: self.focused.then_some(true),
            scrollable: self.scrollable.then_some(true),
            checked: self.checked.then_some(true),
            visible: self.visible.then_some(true),
        }
    }

    pub fn has_selector(&self) -> bool {
        self.text.is_some()
            || self.contains.is_some()
            || self.content_desc.is_some()
            || self.content_desc_contains.is_some()
            || self.resource_id.is_some()
            || self.class.is_some()
            || self.clickable
            || self.enabled
            || self.focused
            || self.scrollable
            || self.checked
            || self.visible
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
