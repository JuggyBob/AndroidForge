//! Complete CLI command catalog shared by the desktop tool database and the
//! generated MCP tool surface.
//!
//! The catalog is derived from clap itself, so a command that exists in the
//! parser cannot silently disappear from the documentation surface. Curated
//! examples are additive; the fallback is the command's own `--help` usage.

use clap::CommandFactory;
use clap::builder::ArgAction;
use serde::{Deserialize, Serialize};

use crate::cli::Cli;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogArg {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short: Option<char>,
    pub positional: bool,
    pub required: bool,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub possible_values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogCommand {
    pub path: String,
    pub tool_name: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub usage: String,
    pub safety: String,
    pub arguments: Vec<CatalogArg>,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub schema_version: u32,
    pub generated_by: String,
    pub commands: Vec<CatalogCommand>,
}

pub fn catalog() -> Catalog {
    let root = Cli::command();
    let mut commands = Vec::new();
    for subcommand in root.get_subcommands() {
        if is_internal(subcommand) {
            continue;
        }
        walk(
            subcommand,
            vec![subcommand.get_name().to_string()],
            &mut commands,
        );
    }
    commands.sort_by(|left, right| left.path.cmp(&right.path));
    Catalog {
        schema_version: 1,
        generated_by: format!("androidforge {}", env!("CARGO_PKG_VERSION")),
        commands,
    }
}

pub fn command_catalog() -> Vec<CatalogCommand> {
    catalog().commands
}

fn walk(command: &clap::Command, path: Vec<String>, out: &mut Vec<CatalogCommand>) {
    let children = command
        .get_subcommands()
        .filter(|child| !is_internal(child))
        .collect::<Vec<_>>();
    if children.is_empty() {
        out.push(leaf(command, path));
        return;
    }
    for child in children {
        let mut next = path.clone();
        next.push(child.get_name().to_string());
        walk(child, next, out);
    }
}

fn is_internal(command: &clap::Command) -> bool {
    command.get_name() == "help" || command.is_hide_set()
}

fn leaf(command: &clap::Command, path: Vec<String>) -> CatalogCommand {
    let joined = path.join(" ");
    let summary = command
        .get_about()
        .map(ToString::to_string)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("Run `androidforge {joined} --help`."));
    let description = command
        .get_long_about()
        .map(ToString::to_string)
        .filter(|value| !value.trim().is_empty());
    let usage = command
        .clone()
        .render_usage()
        .to_string()
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let arguments = command
        .get_arguments()
        .filter(|arg| arg.get_id().as_str() != "help" && arg.get_id().as_str() != "version")
        .map(catalog_arg)
        .collect::<Vec<_>>();
    let safety = safety(&path).to_string();
    let examples = examples(&joined, &arguments);
    CatalogCommand {
        tool_name: tool_name(&path),
        path: joined,
        summary,
        description,
        usage: if usage.is_empty() {
            format!("androidforge {}", path.join(" "))
        } else {
            usage
        },
        safety,
        arguments,
        examples,
    }
}

fn catalog_arg(arg: &clap::Arg) -> CatalogArg {
    let possible_values = arg
        .get_possible_values()
        .iter()
        .map(|value| value.get_name().to_string())
        .collect::<Vec<_>>();
    let action = arg.get_action();
    let kind = if matches!(action, ArgAction::SetTrue | ArgAction::Count) {
        "boolean"
    } else if !possible_values.is_empty() {
        "enum"
    } else if matches!(action, ArgAction::Append)
        || arg
            .get_num_args()
            .is_some_and(|range| range.max_values() > 1)
    {
        "array"
    } else {
        "string"
    };
    CatalogArg {
        id: arg.get_id().to_string(),
        long: arg.get_long().map(str::to_string),
        short: arg.get_short(),
        positional: arg.is_positional(),
        required: arg.is_required_set(),
        kind: kind.to_string(),
        value_name: arg
            .get_value_names()
            .and_then(|names| names.first())
            .map(ToString::to_string),
        help: arg.get_help().map(ToString::to_string),
        possible_values,
        default: arg
            .get_default_values()
            .first()
            .map(|value| value.to_string_lossy().into_owned()),
    }
}

fn tool_name(path: &[String]) -> String {
    let mut name = String::from("androidforge");
    for segment in path {
        name.push('_');
        for character in segment.chars() {
            if character.is_ascii_alphanumeric() {
                name.push(character.to_ascii_lowercase());
            } else {
                name.push('_');
            }
        }
    }
    while name.contains("__") {
        name = name.replace("__", "_");
    }
    name
}

fn safety(path: &[String]) -> &'static str {
    let joined = path.join(" ");
    if joined == "adb" || joined.starts_with("run exec") {
        return "unsafe";
    }
    if joined == "mcp serve" || joined == "network mock serve" {
        return "long-running";
    }
    const DESTRUCTIVE: &[&str] = &[
        "app uninstall",
        "app clear-data",
        "app data import",
        "app data-restore",
        "app data reset",
        "app data seed",
        "snapshot load",
        "snapshot delete",
        "permissions grant",
        "permissions revoke",
        "appops set",
        "appops reset",
        "doze force",
        "battery unplug",
        "battery reset",
        "time set",
        "time restore",
        "notifications clear",
        "jobs run",
        "jobs cancel",
        "session stop",
        "session release",
        "session restart",
        "session gc",
        "session prune",
        "maintenance clean",
        "report import",
        "report prune",
        "avd create",
        "devices tag add",
        "devices tag remove",
        "devices tag clear",
        "fixture run",
        // Declarative state writes/removes app-private files through run-as.
        "fixture apply",
        "fixture reset",
        "conformance run",
        "hook run",
        "tools uninstall",
        "tools install",
        "init",
    ];
    if DESTRUCTIVE.iter().any(|prefix| joined.starts_with(prefix)) {
        return "destructive";
    }
    const MUTATING: &[&str] = &[
        "agent test",
        "agent matrix",
        // An allowlisted scenario dispatches audited app/UI/fixture operations.
        "scenario run",
        "app install",
        "app launch",
        "app force-stop",
        "app kill",
        "app broadcast",
        "app service",
        "app file-push",
        "network airplane",
        "network wifi",
        "network data",
        "network proxy set",
        "network proxy clear",
        "network profile apply",
        "network restore",
        "network reverse add",
        "network reverse remove",
        "network reverse clear",
        "doze unforce",
        "emulator start",
        "emulator stop",
        "session start",
        "session ensure",
        "session attach",
        "session claim",
        "devices prepare",
        "devices restore",
        "devices scenario",
        "devices shell",
        "devices input tap",
        "ui tap",
        "ui long-press",
        "ui drag",
        "ui text",
        "ui clear",
        "ui swipe",
        "ui scroll",
        "ui scroll-to",
        "ui back",
        "ui home",
        "ui keyevent",
        "ui hide-keyboard",
        "capture record start",
        "capture record stop",
        // Launches the target package by design. Adds no physical mutation flag:
        // the existing scoped app-control consent rules still apply.
        "capture launch",
        // Startup measurement launches the app (`am start -W`, and `am force-stop`
        // with --cold), so the same scoped app-control consent rules apply.
        // `diagnostics frames` stays read-only: it only reads gfxinfo counters.
        "diagnostics performance",
        "notifications tap",
        "ui dismiss-overlays",
        "snapshot save",
        "standby set",
        "test maestro",
        "test instrumentation",
    ];
    if MUTATING.iter().any(|prefix| joined.starts_with(prefix)) {
        return "device-mutation";
    }
    // Named baselines write only to the local, run-scoped store; a11y audits
    // and contrast checks only read the hierarchy and screenshot. They stay
    // read-only even when a fresh screenshot is captured from a device.
    // `build` executes a project-configured host command and writes a bounded
    // local log artifact. Neither is device mutation: build alone resolves no
    // session and contacts no ADB device. Its install/launch flags reuse the
    // already-classified app paths and their physical gates, and the curated
    // MCP build tool deliberately omits those flags.
    "read-only"
}

fn examples(path: &str, arguments: &[CatalogArg]) -> Vec<String> {
    let specific: &[&str] = match path {
        "doctor" => &[
            "androidforge doctor --json",
            "androidforge doctor fix --plan --json",
            "androidforge doctor --capacity --json",
        ],
        "avd images" => &["androidforge avd images --json"],
        "avd create" => &[
            "androidforge avd create Pixel_8_API_35 --image \"system-images;android-35;google_apis;x86_64\" --device pixel_7 --dry-run --json",
            "androidforge avd create Pixel_8_API_35 --image \"system-images;android-35;google_apis;x86_64\" --device pixel_7 --yes --json",
        ],
        "ports list" => &["androidforge ports list --json"],
        "artifacts scan-secrets" => &["androidforge artifacts scan-secrets --project . --json"],
        "devices shell" => &[
            "androidforge devices shell --read-only --session <ID> --json -- dumpsys gfxinfo com.example",
            "androidforge devices shell --read-only --allow \"pm list\" --session <ID> --json -- pm list packages",
            "androidforge device shell --read-only --session <ID> --json -- getprop ro.build.version.release",
        ],
        "devices input tap" => &[
            "androidforge devices input tap 540 1200 --session <ID> --json",
            "androidforge device input tap 540 1200 --session <ID> --dry-run --json",
        ],
        "devices scenario" => &[
            "androidforge devices scenario --night yes --session <ID> --json",
            "androidforge devices scenario --network offline --dry-run --json",
        ],
        "devices tag add" => {
            &["androidforge devices tag add --serial emulator-5554 --tag nightly --json"]
        }
        "devices tag list" => &["androidforge devices tag list --json"],
        "context" => &[
            "androidforge context --json",
            "androidforge ctx --brief --json",
        ],
        "agent brief" => &["androidforge agent brief --json"],
        "agent test" => &["androidforge agent test --apk .\\app-debug.apk --dry-run --json"],
        "agent matrix" => &[
            "androidforge agent matrix --avd Pixel_8_API_35 --avd Pixel_7_API_34 --flow conformance --dry-run --json",
            "androidforge agent matrix --device tag:nightly --flow conformance --dry-run --json",
        ],
        "session ensure" => &[
            "androidforge session ensure --serial emulator-5554 --owner MyAgent --json",
            "androidforge session ensure --avd Pixel_8_API_35 --headless --wait --timeout 10m --json",
        ],
        "session list" => &[
            "androidforge session list --mine --active --json",
            "androidforge session gc --reclaim --dry-run --json",
        ],
        "build" => &[
            "androidforge build --variant debug --json",
            "androidforge build --variant debug --module app --expected-package com.example.app --json",
            "androidforge build --variant debug --install --only-if-changed --json",
        ],
        "app install" => &[
            "androidforge app install .\\app-debug.apk --session <ID> --json",
            "androidforge app install .\\base.apk --split .\\config.arm64_v8a.apk --only-if-changed --session <ID> --json",
            "androidforge app install .\\app-debug.apk --session <ID> --wait-for-confirmation --confirm-text \"Install\" --json",
        ],
        "app verify-installed" => {
            &["androidforge app verify-installed --apk .\\app-debug.apk --session <ID> --json"]
        }
        "app data export" => &[
            "androidforge app data export com.example.app --session <ID> --json",
            "androidforge app data export com.example.app --output .\\data.tar --session <ID> --json",
            "androidforge app data export com.example.app .\\data.tar --allow-in-project --overwrite --session <ID> --json",
        ],
        "app data import" => &[
            "androidforge app data import com.example.app .\\data.tar --yes --session <ID> --json",
        ],
        "maintenance clean" => &[
            "androidforge maintenance clean --runs --keep-tag release --json",
            "androidforge maintenance clean --runs --keep-tag release --keep-last 5 --yes --json",
        ],
        "ui observe" => &["androidforge ui observe --session <ID> --json"],
        "ui tap" => &[
            "androidforge ui tap --text \"Save\" --session <ID> --json",
            "androidforge ui tap --test-tag save_button --capture-before .\\before.png --capture-after .\\after.png --session <ID> --json",
        ],
        "ui drag" => &[
            "androidforge ui drag --from \"text=Item\" --to \"tag=trash\" --duration 800 --session <ID> --json",
        ],
        "ui wait" => &[
            "androidforge ui wait --text \"Saved\" --timeout 20 --session <ID> --json",
            "androidforge ui wait --for-idle --session <ID> --json",
            "androidforge ui wait --text \"Save\" --for-bounds-stable --stable-for 2s --timeout 15 --session <ID> --json",
        ],
        "ui audit a11y" => &[
            "androidforge ui audit a11y --session <ID> --json",
            "androidforge ui audit a11y --session <ID> --contrast --text-rgb #FFFFFF --density 2.75 --json",
        ],
        "ui bounds" => &["androidforge ui bounds --text \"Save\" --session <ID> --json"],
        "ui assert-bounds" => &[
            "androidforge ui assert-bounds --text \"Save\" --bottom-y 300 --tolerance 2 --session <ID> --json",
            "androidforge ui assert-bounds --resource-id com.example:id/save --bottom-y-gt 800 --session <ID> --json",
        ],
        "network mock serve" => &[
            "androidforge network mock serve --port 8080 --route 'GET /health=200' --reverse --max-requests 2 --session <ID> --json",
            "androidforge network mock serve --scenario offline --port 8080 --json",
        ],
        "network mock scenarios" => &["androidforge network mock scenarios --json"],
        "fixture apply" => &["androidforge fixture apply state-demo --session <ID> --json"],
        "fixture reset" => &["androidforge fixture reset state-demo --yes --session <ID> --json"],
        "scenario run" => &[
            "androidforge scenario run smoke.toml --dry-run --json",
            "androidforge scenario run smoke.toml --session <ID> --yes --allow-physical-input --json",
            "androidforge scenario run smoke.json --repeat 3 --yes --json",
        ],
        "scenario validate" => &["androidforge scenario validate smoke.toml --json"],
        "capture screenshot" => &[
            "androidforge capture screenshot --session <ID> --json",
            "androidforge capture screenshot --compare .\\baseline.png --region 0,120,1080,900 --max-diff-ratio 0.002 --session <ID> --json",
        ],
        "capture assert-color" => &[
            "androidforge capture assert-color --rgb #1A73E8 --at 540,1200 --tolerance 8 --session <ID> --json",
            "androidforge capture assert-color --rgb #FFFFFF --region 0,0,1080,200 --majority --session <ID> --json",
        ],
        "capture baseline save" => {
            &["androidforge capture baseline save --name home_screen --session <ID> --json"]
        }
        "capture baseline update" => {
            &["androidforge capture baseline update --name home_screen --session <ID> --json"]
        }
        "capture baseline compare" => &[
            "androidforge capture baseline compare --name home_screen --region 0,120,1080,900 --max-diff-ratio 0.002 --session <ID> --json",
        ],
        "capture baseline list" => &["androidforge capture baseline list --json"],
        "capture contrast" => &[
            "androidforge capture contrast --rgb #FFFFFF --at 540,1200 --session <ID> --json",
            "androidforge capture contrast --rgb #000000 --region 0,0,1080,200 --majority --session <ID> --json",
        ],
        "capture launch" => &[
            "androidforge capture launch com.example.app --session <ID> --expect-first-frame --no-splash --json",
            "androidforge capture launch com.example.app --session <ID> --fps 10 --duration 5s --expected-color #1A73E8 --ui-dump-package --json",
        ],
        "diagnostics frames" => &[
            "androidforge diagnostics frames com.example.app --session <ID> --json",
            "androidforge diagnostics frames com.example.app --session <ID> --reset --duration 5s --baseline .\\runs\\old\\frames\\report.json --max-p99 5 --json",
        ],
        "diagnostics logcat query" => &[
            "androidforge diagnostics logcat query --session <ID> --redact-patterns AIza,sk- --json",
        ],
        "diagnostics logcat watch" => &[
            "androidforge diagnostics logcat watch --session <ID> --redact-patterns ghp_,AKIA --json",
        ],
        "report bundle" => &["androidforge report bundle latest --redact-secrets --json"],
        "tools install" => &["androidforge tools install maestro --yes --json"],
        "tools status" => &["androidforge tools status maestro --json"],
        "tools catalog" => &["androidforge tools catalog --json"],
        _ => &[],
    };
    if specific.is_empty() {
        let mut example = format!("androidforge {path}");
        for argument in arguments.iter().filter(|argument| argument.required) {
            if argument.positional {
                example.push_str(&format!(
                    " <{}>",
                    argument
                        .value_name
                        .clone()
                        .unwrap_or_else(|| argument.id.to_uppercase())
                ));
            } else if let Some(long) = &argument.long {
                example.push_str(&format!(
                    " --{long} <{}>",
                    argument
                        .value_name
                        .clone()
                        .unwrap_or_else(|| argument.id.to_uppercase())
                ));
            }
        }
        example.push_str(" --json");
        vec![example, format!("androidforge {path} --help")]
    } else {
        specific.iter().map(|value| value.to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_expected_leaf_commands() {
        let catalog = command_catalog();
        for expected in [
            "app data export",
            "maintenance clean",
            "app verify-installed",
            "ui tap",
            "ui drag",
            "ui wait",
            "network mock serve",
            "capture screenshot",
            "session list",
            "tools install",
        ] {
            assert!(
                catalog.iter().any(|command| command.path == expected),
                "missing {expected}"
            );
        }
    }

    #[test]
    fn catalog_is_unique_and_well_formed() {
        let mut names = std::collections::BTreeSet::new();
        for command in command_catalog() {
            assert!(
                names.insert(command.tool_name.clone()),
                "duplicate {}",
                command.tool_name
            );
            assert!(
                !command.summary.is_empty(),
                "empty summary for {}",
                command.path
            );
            assert!(
                !command.example_is_empty(),
                "empty examples for {}",
                command.path
            );
            assert!(command.tool_name.starts_with("androidforge_"));
            assert!(!command.tool_name.contains(" "));
        }
    }

    #[test]
    fn app_control_commands_stay_device_mutation() {
        let catalog = command_catalog();
        for path in ["app launch", "app force-stop"] {
            let command = catalog
                .iter()
                .find(|command| command.path == path)
                .unwrap_or_else(|| panic!("missing {path}"));
            assert_eq!(command.safety, "device-mutation", "{path}");
        }
    }

    #[test]
    fn frames_and_color_commands_are_read_only_and_launch_is_mutating() {
        let catalog = command_catalog();
        for (path, safety) in [
            ("diagnostics frames", "read-only"),
            ("capture assert-color", "read-only"),
            ("capture launch", "device-mutation"),
        ] {
            let command = catalog
                .iter()
                .find(|command| command.path == path)
                .unwrap_or_else(|| panic!("missing {path}"));
            assert_eq!(command.safety, safety, "{path}");
            assert!(
                !command.examples.is_empty(),
                "{path} has no curated example"
            );
        }
        let launch = catalog
            .iter()
            .find(|command| command.path == "capture launch")
            .expect("capture launch");
        for flag in [
            "fps",
            "duration",
            "expect-first-frame",
            "no-splash",
            "expected-color",
            "tolerance",
            "ui-dump-package",
        ] {
            assert!(
                launch
                    .arguments
                    .iter()
                    .any(|argument| argument.long.as_deref() == Some(flag)),
                "capture launch is missing --{flag}"
            );
        }
        assert!(
            !launch
                .arguments
                .iter()
                .any(|argument| argument.long.as_deref() == Some("allow-physical-mutation")),
            "capture launch must not advertise a physical mutation flag"
        );
    }

    #[test]
    fn dangerous_commands_are_classified() {
        let catalog = command_catalog();
        for expected in [
            ("app uninstall", "destructive"),
            ("app data import", "destructive"),
            ("app data-restore", "destructive"),
            ("report prune", "destructive"),
            ("ui tap", "device-mutation"),
            ("doze unforce", "device-mutation"),
            ("snapshot save", "device-mutation"),
            ("standby set", "device-mutation"),
            ("test maestro", "device-mutation"),
            ("test instrumentation", "device-mutation"),
            ("agent test", "device-mutation"),
            ("agent matrix", "device-mutation"),
            ("notifications tap", "device-mutation"),
            ("ui dismiss-overlays", "device-mutation"),
            ("tools install", "destructive"),
        ] {
            let command = catalog
                .iter()
                .find(|command| command.path == expected.0)
                .unwrap_or_else(|| panic!("missing {}", expected.0));
            assert_eq!(command.safety, expected.1, "{}", expected.0);
        }
    }

    #[test]
    fn scoped_export_and_retention_flags_keep_their_safety_classes() {
        let catalog = command_catalog();
        for (path, safety) in [
            ("app data export", "read-only"),
            ("maintenance clean", "destructive"),
        ] {
            let command = catalog
                .iter()
                .find(|command| command.path == path)
                .unwrap_or_else(|| panic!("missing {path}"));
            assert_eq!(command.safety, safety, "{path}");
            assert!(
                !command.examples.is_empty(),
                "{path} has no curated example"
            );
        }
        let export = catalog
            .iter()
            .find(|command| command.path == "app data export")
            .expect("app data export");
        for flag in ["output", "allow-in-project", "overwrite"] {
            assert!(
                export
                    .arguments
                    .iter()
                    .any(|argument| argument.long.as_deref() == Some(flag)),
                "app data export is missing --{flag}"
            );
        }
        assert!(
            export
                .arguments
                .iter()
                .any(|argument| argument.id == "path" && !argument.required),
            "the legacy export path must stay optional so the default can be used"
        );
        let clean = catalog
            .iter()
            .find(|command| command.path == "maintenance clean")
            .expect("maintenance clean");
        for flag in ["keep-tag", "older-than", "keep-last"] {
            assert!(
                clean
                    .arguments
                    .iter()
                    .any(|argument| argument.long.as_deref() == Some(flag)),
                "maintenance clean is missing --{flag}"
            );
        }
    }

    #[test]
    fn bounds_commands_are_read_only_and_documented() {
        let catalog = command_catalog();
        for (path, tool_name, flags) in [
            ("ui bounds", "androidforge_ui_bounds", vec![]),
            (
                "ui assert-bounds",
                "androidforge_ui_assert_bounds",
                vec!["bottom-y", "bottom-y-gt", "bottom-y-lt", "tolerance"],
            ),
            (
                "ui wait",
                "androidforge_ui_wait",
                vec!["for-bounds-stable", "stable-for", "timeout"],
            ),
        ] {
            let command = catalog
                .iter()
                .find(|command| command.path == path)
                .unwrap_or_else(|| panic!("missing catalog entry {path}"));
            assert_eq!(command.safety, "read-only", "{path}");
            assert_eq!(command.tool_name, tool_name, "{path}");
            assert!(
                !command.examples.is_empty(),
                "{path} has no curated example"
            );
            for flag in flags {
                assert!(
                    command
                        .arguments
                        .iter()
                        .any(|argument| argument.long.as_deref() == Some(flag)),
                    "{path} is missing --{flag}"
                );
            }
        }
        let assert_bounds = catalog
            .iter()
            .find(|command| command.path == "ui assert-bounds")
            .expect("ui assert-bounds");
        let tolerance = assert_bounds
            .arguments
            .iter()
            .find(|argument| argument.id == "tolerance")
            .expect("tolerance argument");
        assert_eq!(tolerance.default.as_deref(), Some("0"));
    }

    #[test]
    fn scenario_commands_are_classified_and_documented() {
        let catalog = command_catalog();
        let run = catalog
            .iter()
            .find(|command| command.path == "scenario run")
            .expect("scenario run");
        assert_eq!(run.safety, "device-mutation", "scenario run");
        assert_eq!(run.tool_name, "androidforge_scenario_run");
        assert!(
            run.examples
                .iter()
                .any(|example| example.contains("--dry-run")),
            "scenario run needs a dry-run example"
        );
        for flag in ["file", "repeat"] {
            assert!(
                run.arguments.iter().any(|argument| argument.id == flag),
                "scenario run is missing {flag}"
            );
        }
        let validate = catalog
            .iter()
            .find(|command| command.path == "scenario validate")
            .expect("scenario validate");
        assert_eq!(validate.safety, "read-only", "scenario validate");
        assert_eq!(validate.tool_name, "androidforge_scenario_validate");
        assert!(!validate.examples.is_empty());
    }

    #[test]
    fn new_command_entries_are_classified_and_documented() {
        let catalog = command_catalog();
        for path in ["devices shell", "devices input tap"] {
            let command = catalog
                .iter()
                .find(|command| command.path == path)
                .unwrap_or_else(|| panic!("missing catalog entry {path}"));
            assert_eq!(command.safety, "device-mutation", "{path}");
            assert!(
                !command.examples.is_empty(),
                "{path} has no curated example"
            );
        }
        let shell = catalog
            .iter()
            .find(|command| command.path == "devices shell")
            .expect("devices shell");
        assert!(
            shell
                .examples
                .iter()
                .any(|example| example.contains("--read-only")),
            "device shell needs a read-only example"
        );
        let scan = catalog
            .iter()
            .find(|command| command.path == "artifacts scan-secrets")
            .expect("artifacts scan-secrets");
        assert_eq!(scan.safety, "read-only");
        assert_eq!(scan.tool_name, "androidforge_artifacts_scan_secrets");
        let bundle = catalog
            .iter()
            .find(|command| command.path == "report bundle")
            .expect("report bundle");
        assert!(
            bundle
                .arguments
                .iter()
                .any(|argument| argument.long.as_deref() == Some("redact-secrets")),
            "report bundle is missing --redact-secrets"
        );
    }

    impl CatalogCommand {
        fn example_is_empty(&self) -> bool {
            self.examples.is_empty()
        }
    }
}
