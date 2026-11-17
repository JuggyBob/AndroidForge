pub mod adb;
pub mod agent;
pub mod apk;
pub mod app;
pub mod automation;
pub mod bundle;
pub mod common;
pub mod context;
pub mod devices;
pub mod diagnostics;
pub mod doctor;
pub mod maintenance;
pub mod matrix;
pub mod observation;
pub mod receipt;
pub mod report;
pub mod session;
pub mod system;
pub mod ui;

use androidforge_core::error::Result;
use androidforge_core::output::CommandSuccess;

use crate::cli::{
    AppCommand, CaptureCommand, Cli, Command, DevicesCommand, DiagnosticsCommand, EmulatorCommand,
    FixtureCommand, HookCommand, ReportCommand, RunCommand, SessionCommand, SnapshotCommand,
    TestCommand, UiCommand,
};
use crate::context::CliContext;

pub async fn execute(cli: &Cli) -> Result<CommandSuccess> {
    match &cli.command {
        Command::Doctor { command } => match command {
            Some(crate::cli::DoctorCommand::Fix(args)) => doctor::fix_plan(cli, args).await,
            None => doctor::doctor(cli).await,
        },
        Command::Context(args) => context::context(cli, args).await,
        Command::Agent { command } => agent::agent(cli, command).await,
        Command::Capabilities => doctor::capabilities(cli).await,
        Command::Schema { command } => doctor::schema(command),
        Command::Init(args) => doctor::init(cli, args),
        Command::Config { command } => doctor::config(cli, command).await,
        Command::Adb(args) => {
            let ctx = CliContext::new(cli)?;
            adb::run(&ctx, args).await
        }
        Command::Session { command } => {
            let ctx = CliContext::new(cli)?;
            session::session(&ctx, command).await
        }
        Command::Devices { command } => {
            let ctx = CliContext::new(cli)?;
            devices::devices(&ctx, command).await
        }
        Command::Avd { command } => {
            let ctx = CliContext::new(cli)?;
            session::avd(&ctx, command).await
        }
        Command::Emulator { command } => {
            let ctx = CliContext::new(cli)?;
            session::emulator(&ctx, command).await
        }
        Command::Snapshot { command } => {
            let ctx = CliContext::new(cli)?;
            session::snapshot(&ctx, command).await
        }
        Command::App { command } => {
            let ctx = CliContext::new(cli)?;
            app::app(&ctx, command).await
        }
        Command::Apk {
            command: crate::cli::ApkCommand::Inspect { apk },
        } => {
            let ctx = CliContext::new(cli)?;
            app::apk_inspect(&ctx, apk).await
        }
        Command::Apk {
            command: crate::cli::ApkCommand::Verify(args),
        } => {
            let ctx = CliContext::new(cli)?;
            app::apk_verify(&ctx, args).await
        }
        Command::Apk {
            command: crate::cli::ApkCommand::Resolve(args),
        } => {
            let ctx = CliContext::new(cli)?;
            apk::resolve_command(&ctx, args).await
        }
        Command::Apk {
            command: crate::cli::ApkCommand::Diff(args),
        } => {
            let ctx = CliContext::new(cli)?;
            apk::diff_command(&ctx, args).await
        }
        Command::Ui { command } => {
            let ctx = CliContext::new(cli)?;
            ui::ui(&ctx, command).await
        }
        Command::Capture { command } => {
            let ctx = CliContext::new(cli)?;
            diagnostics::capture(&ctx, command).await
        }
        Command::Diagnostics { command } => {
            let ctx = CliContext::new(cli)?;
            diagnostics::diagnostics(&ctx, command).await
        }
        Command::Notifications { command } => {
            let ctx = CliContext::new(cli)?;
            system::notifications(&ctx, command).await
        }
        Command::Jobs { command } => {
            let ctx = CliContext::new(cli)?;
            system::jobs(&ctx, command).await
        }
        Command::Alarms { command } => {
            let ctx = CliContext::new(cli)?;
            system::alarms(&ctx, command).await
        }
        Command::Permissions { command } => {
            let ctx = CliContext::new(cli)?;
            system::permissions(&ctx, command).await
        }
        Command::Appops { command } => {
            let ctx = CliContext::new(cli)?;
            system::appops(&ctx, command).await
        }
        Command::Doze { command } => {
            let ctx = CliContext::new(cli)?;
            system::doze(&ctx, command).await
        }
        Command::Standby { command } => {
            let ctx = CliContext::new(cli)?;
            system::standby(&ctx, command).await
        }
        Command::Battery { command } => {
            let ctx = CliContext::new(cli)?;
            system::battery(&ctx, command).await
        }
        Command::Network { command } => {
            let ctx = CliContext::new(cli)?;
            system::network(&ctx, command).await
        }
        Command::Time { command } => {
            let ctx = CliContext::new(cli)?;
            system::time(&ctx, command).await
        }
        Command::Hook { command } => {
            let ctx = CliContext::new(cli)?;
            automation::hook(&ctx, command).await
        }
        Command::Fixture { command } => {
            let ctx = CliContext::new(cli)?;
            automation::fixture(&ctx, command).await
        }
        Command::Conformance { command } => {
            let ctx = CliContext::new(cli)?;
            automation::conformance(&ctx, command).await
        }
        Command::Test { command } => {
            let ctx = CliContext::new(cli)?;
            automation::test(&ctx, command).await
        }
        Command::Run { command } => {
            let ctx = CliContext::new(cli)?;
            report::run_command(&ctx, command).await
        }
        Command::Report { command } => {
            let ctx = CliContext::new(cli)?;
            report::report(&ctx, command).await
        }
        Command::Maintenance { command } => {
            let ctx = CliContext::new(cli)?;
            maintenance::maintenance(&ctx, command).await
        }
    }
}

pub fn canonical_name(command: &Command) -> String {
    match command {
        Command::Doctor { command } => match command {
            Some(crate::cli::DoctorCommand::Fix(_)) => "doctor.fix".to_string(),
            None => "doctor".to_string(),
        },
        Command::Context(_) => "context".to_string(),
        Command::Agent { command } => match command {
            crate::cli::AgentCommand::Test(_) => "agent.test".to_string(),
            crate::cli::AgentCommand::Matrix(_) => "agent.matrix".to_string(),
        },
        Command::Capabilities => "capabilities".to_string(),
        Command::Schema { .. } => "schema".to_string(),
        Command::Init(_) => "init".to_string(),
        Command::Config { .. } => "config".to_string(),
        Command::Session { command } => format!("session.{}", session_name(command)),
        Command::Devices { command } => format!("devices.{}", devices_name(command)),
        Command::Avd { command } => match command {
            crate::cli::AvdCommand::List => "avd.list".to_string(),
            crate::cli::AvdCommand::Clone { .. } => "avd.clone".to_string(),
        },
        Command::Emulator { command } => format!("emulator.{}", emulator_name(command)),
        Command::Snapshot { command } => format!("snapshot.{}", snapshot_name(command)),
        Command::App {
            command: app_command,
        } => format!("app.{}", app_name(app_command)),
        Command::Apk { command } => match command {
            crate::cli::ApkCommand::Inspect { .. } => "apk.inspect".to_string(),
            crate::cli::ApkCommand::Verify(_) => "apk.verify".to_string(),
            crate::cli::ApkCommand::Resolve(_) => "apk.resolve".to_string(),
            crate::cli::ApkCommand::Diff(_) => "apk.diff".to_string(),
        },
        Command::Ui { command } => format!("ui.{}", ui_name(command)),
        Command::Capture { command } => capture_name(command),
        Command::Diagnostics { command } => format!("diagnostics.{}", diagnostics_name(command)),
        Command::Notifications { command } => format!("notifications.{}", nested_name(command)),
        Command::Jobs { command } => format!("jobs.{}", nested_name(command)),
        Command::Alarms { command } => format!("alarms.{}", nested_name(command)),
        Command::Permissions { command } => format!("permissions.{}", nested_name(command)),
        Command::Appops { command } => format!("appops.{}", nested_name(command)),
        Command::Doze { command } => format!("doze.{}", nested_name(command)),
        Command::Standby { command } => format!("standby.{}", nested_name(command)),
        Command::Battery { command } => format!("battery.{}", nested_name(command)),
        Command::Network { command } => format!("network.{}", nested_name(command)),
        Command::Time { command } => format!("time.{}", nested_name(command)),
        Command::Hook {
            command: HookCommand::List,
        } => "hook.list".to_string(),
        Command::Hook { .. } => "hook.run".to_string(),
        Command::Fixture { command } => format!("fixture.{}", fixture_name(command)),
        Command::Conformance { .. } => "conformance.run".to_string(),
        Command::Test { command } => match command {
            TestCommand::Maestro { .. } => "test.maestro".to_string(),
            TestCommand::Instrumentation { .. } => "test.instrumentation".to_string(),
        },
        Command::Run { command } => format!("run.{}", run_name(command)),
        Command::Report { command } => format!("report.{}", report_name(command)),
        Command::Maintenance { command } => format!("maintenance.{}", maintenance_name(command)),
        Command::Adb(_) => "adb".to_string(),
    }
}

fn session_name(command: &SessionCommand) -> &'static str {
    match command {
        SessionCommand::Start(_) => "start",
        SessionCommand::Ensure(_) => "ensure",
        SessionCommand::Attach(_) => "attach",
        SessionCommand::Claim(_) => "claim",
        SessionCommand::List { .. } => "list",
        SessionCommand::Inspect => "inspect",
        SessionCommand::Status => "status",
        SessionCommand::Release(_) => "release",
        SessionCommand::Stop(_) => "stop",
        SessionCommand::Use { .. } => "use",
        SessionCommand::Current => "current",
        SessionCommand::Gc(_) => "gc",
        SessionCommand::Prune(_) => "prune",
        SessionCommand::Restart(_) => "restart",
    }
}

fn devices_name(command: &DevicesCommand) -> &'static str {
    match command {
        DevicesCommand::List => "list",
        DevicesCommand::Inspect => "inspect",
        DevicesCommand::Wait(_) => "wait",
        DevicesCommand::Prepare(_) => "prepare",
        DevicesCommand::Restore => "restore",
        DevicesCommand::Pair(_) => "pair",
        DevicesCommand::Connect(_) => "connect",
        DevicesCommand::Disconnect(_) => "disconnect",
        DevicesCommand::Health(_) => "health",
    }
}

fn emulator_name(command: &EmulatorCommand) -> &'static str {
    match command {
        EmulatorCommand::Start(_) => "start",
        EmulatorCommand::Status => "status",
        EmulatorCommand::Stop(_) => "stop",
    }
}

fn snapshot_name(command: &SnapshotCommand) -> &'static str {
    match command {
        SnapshotCommand::List => "list",
        SnapshotCommand::Save { .. } => "save",
        SnapshotCommand::Load { .. } => "load",
        SnapshotCommand::Delete { .. } => "delete",
    }
}

fn app_name(command: &AppCommand) -> &'static str {
    match command {
        AppCommand::Install(_) => "install",
        AppCommand::Uninstall(_) => "uninstall",
        AppCommand::ClearData(_) => "clear-data",
        AppCommand::Launch(_) => "launch",
        AppCommand::ForceStop(_) => "force-stop",
        AppCommand::Kill(_) => "kill",
        AppCommand::Status(_) => "status",
        AppCommand::Info(_) => "info",
        AppCommand::List(_) => "list",
        AppCommand::FilePush(_) => "file-push",
        AppCommand::FilePull(_) => "file-pull",
        AppCommand::DataBackup(_) => "data-backup",
        AppCommand::DataRestore(_) => "data-restore",
    }
}

fn ui_name(command: &UiCommand) -> &'static str {
    match command {
        UiCommand::Dump(_) => "dump",
        UiCommand::Inspect(_) => "inspect",
        UiCommand::Observe(_) => "observe",
        UiCommand::Diff(_) => "diff",
        UiCommand::Find(_) => "find",
        UiCommand::Tap(_) => "tap",
        UiCommand::LongPress(_) => "long-press",
        UiCommand::Text(_) => "text",
        UiCommand::Clear(_) => "clear",
        UiCommand::Wait(_) => "wait",
        UiCommand::Assert(_) => "assert",
        UiCommand::Settle(_) => "settle",
        UiCommand::Scroll(_) => "scroll",
        UiCommand::ScrollTo(_) => "scroll-to",
        UiCommand::Swipe(_) => "swipe",
        UiCommand::Back(_) => "back",
        UiCommand::Home => "home",
        UiCommand::Keyevent(_) => "keyevent",
        UiCommand::HideKeyboard => "hide-keyboard",
        UiCommand::Focused => "focused",
    }
}

fn capture_name(command: &CaptureCommand) -> String {
    match command {
        CaptureCommand::Screenshot(_) => "capture.screenshot".to_string(),
        CaptureCommand::Frame(_) => "capture.frame".to_string(),
        CaptureCommand::Record { command } => format!(
            "capture.record.{}",
            match command {
                crate::cli::RecordCommand::Start(_) => "start",
                crate::cli::RecordCommand::Stop => "stop",
                crate::cli::RecordCommand::Status => "status",
            }
        ),
    }
}

fn diagnostics_name(command: &DiagnosticsCommand) -> &'static str {
    match command {
        DiagnosticsCommand::Collect(_) => "collect",
        DiagnosticsCommand::App { .. } => "app",
        DiagnosticsCommand::Logcat { .. } => "logcat",
        DiagnosticsCommand::Crashes { .. } => "crashes",
        DiagnosticsCommand::Inspect(_) => "inspect",
        DiagnosticsCommand::Performance(_) => "performance",
    }
}

fn fixture_name(command: &FixtureCommand) -> &'static str {
    match command {
        FixtureCommand::List => "list",
        FixtureCommand::Validate { .. } => "validate",
        FixtureCommand::Run(_) => "run",
    }
}

fn run_name(command: &RunCommand) -> &'static str {
    match command {
        RunCommand::Start(_) => "start",
        RunCommand::End(_) => "end",
        RunCommand::Status => "status",
        RunCommand::Exec(_) => "exec",
    }
}

fn maintenance_name(command: &crate::cli::MaintenanceCommand) -> &'static str {
    match command {
        crate::cli::MaintenanceCommand::Summary => "summary",
        crate::cli::MaintenanceCommand::Clean(_) => "clean",
    }
}

fn report_name(command: &ReportCommand) -> &'static str {
    match command {
        ReportCommand::List { .. } => "list",
        ReportCommand::Show { .. } => "show",
        ReportCommand::Path { .. } => "path",
        ReportCommand::Prune(_) => "prune",
        ReportCommand::Reconcile(_) => "reconcile",
        ReportCommand::Compare(_) => "compare",
        ReportCommand::Flaky(_) => "flaky",
        ReportCommand::Explain(_) => "explain",
        ReportCommand::History(_) => "history",
        ReportCommand::Recurrences(_) => "recurrences",
        ReportCommand::Bundle(_) => "bundle",
        ReportCommand::Import { .. } => "import",
    }
}

trait NestedName {
    fn nested_name(&self) -> &'static str;
}

fn nested_name<T: NestedName>(value: &T) -> &'static str {
    value.nested_name()
}

impl NestedName for crate::cli::NotificationsCommand {
    fn nested_name(&self) -> &'static str {
        match self {
            Self::List { .. } => "list",
            Self::Wait(_) => "wait",
            Self::Assert(_) => "assert",
            Self::Channels { .. } => "channels",
            Self::Tap(_) => "tap",
            Self::Clear(_) => "clear",
        }
    }
}

impl NestedName for crate::cli::JobsCommand {
    fn nested_name(&self) -> &'static str {
        match self {
            Self::List { .. } => "list",
            Self::Run { .. } => "run",
            Self::Cancel { .. } => "cancel",
            Self::Wait(_) => "wait",
            Self::Assert(_) => "assert",
        }
    }
}

impl NestedName for crate::cli::AlarmsCommand {
    fn nested_name(&self) -> &'static str {
        match self {
            Self::Next { .. } => "next",
            Self::List { .. } => "list",
            Self::Wait { .. } => "wait",
        }
    }
}

impl NestedName for crate::cli::PermissionsCommand {
    fn nested_name(&self) -> &'static str {
        match self {
            Self::Check { .. } => "check",
            Self::Grant { .. } => "grant",
            Self::Revoke { .. } => "revoke",
        }
    }
}

impl NestedName for crate::cli::AppOpsCommand {
    fn nested_name(&self) -> &'static str {
        match self {
            Self::Get { .. } => "get",
            Self::Set { .. } => "set",
            Self::Reset { .. } => "reset",
        }
    }
}

impl NestedName for crate::cli::DozeCommand {
    fn nested_name(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Force => "force",
            Self::Unforce => "unforce",
        }
    }
}

impl NestedName for crate::cli::StandbyCommand {
    fn nested_name(&self) -> &'static str {
        match self {
            Self::Get { .. } => "get",
            Self::Set { .. } => "set",
        }
    }
}

impl NestedName for crate::cli::BatteryCommand {
    fn nested_name(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Unplug => "unplug",
            Self::Reset => "reset",
        }
    }
}

impl NestedName for crate::cli::NetworkCommand {
    fn nested_name(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Airplane { .. } => "airplane",
            Self::Wifi { .. } => "wifi",
            Self::Data { .. } => "data",
        }
    }
}

impl NestedName for crate::cli::TimeCommand {
    fn nested_name(&self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Set(_) => "set",
            Self::Restore => "restore",
        }
    }
}
