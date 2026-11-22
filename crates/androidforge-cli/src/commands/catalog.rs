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
        "fixture run",
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
    ];
    if MUTATING.iter().any(|prefix| joined.starts_with(prefix)) {
        return "device-mutation";
    }
    "read-only"
}

fn examples(path: &str, arguments: &[CatalogArg]) -> Vec<String> {
    let specific: &[&str] = match path {
        "doctor" => &[
            "androidforge doctor --json",
            "androidforge doctor fix --plan --json",
        ],
        "context" => &[
            "androidforge context --json",
            "androidforge ctx --brief --json",
        ],
        "agent brief" => &["androidforge agent brief --json"],
        "agent test" => &["androidforge agent test --apk .\\app-debug.apk --dry-run --json"],
        "session ensure" => &[
            "androidforge session ensure --serial emulator-5554 --owner MyAgent --json",
            "androidforge session ensure --avd Pixel_8_API_35 --headless --wait --timeout 10m --json",
        ],
        "session list" => &[
            "androidforge session list --mine --active --json",
            "androidforge session gc --reclaim --dry-run --json",
        ],
        "app install" => &[
            "androidforge app install .\\app-debug.apk --session <ID> --json",
            "androidforge app install .\\base.apk --split .\\config.arm64_v8a.apk --only-if-changed --session <ID> --json",
        ],
        "app verify-installed" => {
            &["androidforge app verify-installed --apk .\\app-debug.apk --session <ID> --json"]
        }
        "app data export" => {
            &["androidforge app data export com.example.app .\\data.tar --session <ID> --json"]
        }
        "app data import" => &[
            "androidforge app data import com.example.app .\\data.tar --yes --session <ID> --json",
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
        ],
        "network mock serve" => &[
            "androidforge network mock serve --port 8080 --route 'GET /health=200' --reverse --max-requests 2 --session <ID> --json",
        ],
        "capture screenshot" => &[
            "androidforge capture screenshot --session <ID> --json",
            "androidforge capture screenshot --compare .\\baseline.png --region 0,120,1080,900 --max-diff-ratio 0.002 --session <ID> --json",
        ],
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
    fn dangerous_commands_are_classified() {
        let catalog = command_catalog();
        for expected in [
            ("app uninstall", "destructive"),
            ("app data import", "destructive"),
            ("app data-restore", "destructive"),
            ("report prune", "destructive"),
            ("ui tap", "device-mutation"),
            ("doze unforce", "device-mutation"),
            ("tools install", "destructive"),
        ] {
            let command = catalog
                .iter()
                .find(|command| command.path == expected.0)
                .unwrap_or_else(|| panic!("missing {}", expected.0));
            assert_eq!(command.safety, expected.1, "{}", expected.0);
        }
    }

    impl CatalogCommand {
        fn example_is_empty(&self) -> bool {
            self.examples.is_empty()
        }
    }
}
