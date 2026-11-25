//! Compact and brief output profiles for agent-facing commands.
//!
//! Full output remains the default and keeps every existing field. The compact
//! profile preserves decisions, stable handles, counts, unknown reasons and
//! evidence pointers while removing repetition and capping lists. Brief is the
//! smaller decision-oriented projection used by `agent brief` and `--brief`.

use androidforge_core::config::{AgentProfile, AgentSection, ForgeConfig};
use androidforge_core::error::ForgeError;
use androidforge_core::model::ArtifactRef;
use androidforge_core::output::CommandSuccess;
use androidforge_core::paths::ForgePaths;
use serde_json::{Map, Value, json};

use crate::cli::{AgentCommand, Cli, Command, OutputProfile};

#[derive(Debug, Clone, Copy)]
pub struct CompactLimits {
    pub max_items: usize,
    pub max_text_chars: usize,
}

impl Default for CompactLimits {
    fn default() -> Self {
        Self {
            max_items: 40,
            max_text_chars: 240,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OutputPlan {
    pub profile: OutputProfile,
    pub limits: CompactLimits,
}

impl OutputPlan {
    pub fn is_compact(self) -> bool {
        self.profile.is_compact()
    }

    pub fn brief(self) -> bool {
        self.profile == OutputProfile::Brief
    }
}

pub fn resolve(cli: &Cli) -> OutputPlan {
    let config = agent_section(cli);
    let forced_brief = matches!(
        &cli.command,
        Command::Agent {
            command: AgentCommand::Brief
        }
    );
    let profile = if forced_brief || cli.brief {
        OutputProfile::Brief
    } else if cli.compact {
        OutputProfile::Compact
    } else if let Some(profile) = cli.output_profile {
        profile
    } else {
        match config.as_ref().map(|section| section.profile) {
            Some(AgentProfile::Compact) => OutputProfile::Compact,
            Some(AgentProfile::Brief) => OutputProfile::Brief,
            Some(AgentProfile::Full) | None => OutputProfile::Full,
        }
    };
    let configured = config.unwrap_or_default();
    let max_items = cli.max_items.unwrap_or(configured.max_items).clamp(1, 500);
    let max_text_chars = cli
        .max_text_chars
        .unwrap_or(configured.max_text_chars)
        .clamp(16, 10_000);
    OutputPlan {
        profile,
        limits: CompactLimits {
            max_items,
            max_text_chars,
        },
    }
}

fn agent_section(cli: &Cli) -> Option<AgentSection> {
    let paths = ForgePaths::discover(None, cli.project.clone()).ok()?;
    let config = ForgeConfig::load(&paths).ok()?;
    Some(config.file.agent)
}

/// Apply the selected profile to a successful command result.
pub fn apply_success(success: &mut CommandSuccess, plan: OutputPlan) {
    if !plan.is_compact() {
        success.summary = None;
        success.next.clear();
        return;
    }
    success.summary = Some(summary_for(success, plan.limits.max_text_chars));
    success.next = next_commands(success);
    compact_artifacts(&mut success.artifacts, plan);
    success.result = project_result(&success.command, success.result.clone(), plan);
}

/// Bound error context and suggestions without hiding the failure code.
pub fn apply_error(error: &mut ForgeError, plan: OutputPlan) {
    if !plan.is_compact() {
        return;
    }
    let brief = plan.brief();
    let max_text = plan.limits.max_text_chars;
    let body = error.body_mut();
    body.suggestions.truncate(if brief { 2 } else { 3 });
    for suggestion in &mut body.suggestions {
        *suggestion = truncate(suggestion, max_text.max(80));
    }
    body.artifacts.truncate(if brief { 3 } else { 5 });
    for artifact in &mut body.artifacts {
        artifact.sha256 = None;
        if brief {
            artifact.size_bytes = None;
            artifact.description = None;
        }
    }
    body.cause = body
        .cause
        .as_ref()
        .map(|cause| truncate(cause, max_text.max(80)));
    let mut keys = body.context.keys().cloned().collect::<Vec<_>>();
    if keys.len() > 8 {
        keys.sort_by_key(|key| context_priority(key));
        let keep = keys
            .into_iter()
            .take(8)
            .collect::<std::collections::BTreeSet<_>>();
        body.context.retain(|key, _| keep.contains(key));
    }
    for (key, value) in &mut body.context {
        shrink_context_value(key, value, plan);
    }
}

fn context_priority(key: &str) -> u8 {
    match key {
        "current" | "observed" | "path" | "ref" | "token" | "code" | "stage" => 0,
        "matches" | "fields" | "failure" | "reason" => 1,
        "report" | "junit" | "external_result" => 2,
        _ => 3,
    }
}

fn shrink_context_value(key: &str, value: &mut Value, plan: OutputPlan) {
    let brief = plan.brief();
    let max_text = plan.limits.max_text_chars;
    match key {
        "report" => {
            if let Some(report) = value.as_object() {
                let steps = report
                    .get("steps")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let failed = steps
                    .iter()
                    .filter(|step| step.get("status").and_then(Value::as_str) == Some("failed"))
                    .count();
                *value = json!({
                    "status": report.get("status").cloned().unwrap_or(Value::Null),
                    "failed_steps": failed,
                    "total_steps": steps.len(),
                });
            }
        }
        "junit" => {
            if let Some(junit) = value.as_object() {
                *value = json!({
                    "tests": junit.get("tests").cloned().unwrap_or(Value::Null),
                    "failures": junit.get("failures").cloned().unwrap_or(Value::Null),
                    "errors": junit.get("errors").cloned().unwrap_or(Value::Null),
                    "skipped": junit.get("skipped").cloned().unwrap_or(Value::Null),
                });
            }
        }
        "external_result" => {
            if let Some(result) = value.as_object() {
                *value = json!({
                    "exit_code": result.get("exit_code").cloned().unwrap_or(Value::Null),
                    "timed_out": result.get("timed_out").cloned().unwrap_or(Value::Null),
                    "duration_ms": result.get("duration_ms").cloned().unwrap_or(Value::Null),
                });
            }
        }
        "current" | "observed" => {
            if let Some(node) = value.as_object_mut() {
                retain_node_keys(node);
                for field in ["text", "content_desc", "hint", "resource_id"] {
                    truncate_field(node, field, max_text);
                }
            }
        }
        "matches" => {
            if let Some(matches) = value.as_array_mut() {
                matches.truncate(if brief { 3 } else { 5 });
                for node in matches {
                    compact_node(node, brief, max_text);
                }
            }
        }
        _ => shrink_value(value, brief, max_text),
    }
}

fn shrink_value(value: &mut Value, brief: bool, max_text: usize) {
    match value {
        Value::String(text) => {
            if text.len() > max_text.max(200) * 2 {
                *text = truncate(text, max_text.max(200));
            }
        }
        Value::Array(items) => {
            items.truncate(if brief { 3 } else { 6 });
            for item in items {
                shrink_value(item, brief, max_text);
            }
        }
        Value::Object(object) => {
            for (key, item) in object {
                if matches!(key.as_str(), "text" | "content_desc" | "hint" | "message") {
                    if let Some(text) = item.as_str() {
                        *item = Value::String(truncate(text, max_text));
                    }
                } else {
                    shrink_value(item, brief, max_text);
                }
            }
        }
        _ => {}
    }
}

fn compact_artifacts(artifacts: &mut Vec<ArtifactRef>, plan: OutputPlan) {
    artifacts.truncate(if plan.brief() { 5 } else { 10 });
    for artifact in artifacts {
        artifact.sha256 = None;
        if plan.brief() {
            artifact.size_bytes = None;
            artifact.description = None;
        }
    }
}

fn summary_for(success: &CommandSuccess, max_text: usize) -> String {
    if success.command == "schema.list"
        && let Some(count) = success
            .result
            .get("schemas")
            .and_then(Value::as_array)
            .map(Vec::len)
    {
        return format!("{count} schema(s) available");
    }
    if success.command == "ui.observe"
        && let Some(result) = success.result.as_object()
    {
        let nodes = result
            .get("hierarchy")
            .and_then(|hierarchy| hierarchy.get("node_count"))
            .and_then(Value::as_u64)
            .or_else(|| result.get("actionable_total").and_then(Value::as_u64))
            .unwrap_or(0);
        let actions = result
            .get("actionable_total")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let foreground = result.get("foreground");
        let package = foreground
            .and_then(|foreground| foreground.get("package"))
            .and_then(Value::as_str);
        let activity = foreground
            .and_then(|foreground| foreground.get("activity"))
            .and_then(Value::as_str)
            .map(|activity| activity.rsplit('.').next().unwrap_or(activity));
        let mut summary = format!("{nodes} node(s)");
        if let Some(activity) = activity {
            summary.push_str(&format!(" · {activity}"));
        } else if let Some(package) = package {
            summary.push_str(&format!(" · {package}"));
        }
        summary.push_str(&format!(" · {actions} action(s)"));
        return truncate(&summary, max_text);
    }
    if success.command == "diagnostics.performance"
        && let Some(result) = success.result.as_object()
    {
        let package = result
            .get("package")
            .and_then(Value::as_str)
            .unwrap_or("app");
        let startup = result
            .get("startup")
            .and_then(|startup| startup.get("total_time_ms"))
            .and_then(Value::as_u64)
            .map(|value| format!("startup {value}ms"))
            .unwrap_or_else(|| "startup unknown".to_string());
        let jank = result
            .get("jank")
            .map(|jank| {
                if jank.get("sample_status").and_then(Value::as_str) == Some("insufficient") {
                    let frames = jank
                        .get("total_frames")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    return format!("jank insufficient sample ({frames} frame(s))");
                }
                match jank.get("janky_percent").and_then(Value::as_f64) {
                    Some(percent) => {
                        let total = jank.get("total_frames").and_then(Value::as_u64);
                        let janky = jank.get("janky_frames").and_then(Value::as_u64);
                        match (janky, total) {
                            (Some(janky), Some(total)) => {
                                format!("jank {percent:.1}% ({janky}/{total} frames)")
                            }
                            _ => format!("jank {percent:.1}%"),
                        }
                    }
                    None => match jank.get("sample_status").and_then(Value::as_str) {
                        Some(status) if status != "unknown" => format!("jank {status}"),
                        _ => "jank unknown".to_string(),
                    },
                }
            })
            .unwrap_or_else(|| "jank unknown".to_string());
        let pss = result
            .get("memory")
            .and_then(|memory| memory.get("total_pss_kb"))
            .and_then(Value::as_u64)
            .map(|value| format!("PSS {value} KiB"))
            .unwrap_or_else(|| "memory unknown".to_string());
        let cpu = result
            .get("cpu")
            .map(
                |cpu| match cpu.get("total_percent").and_then(Value::as_f64) {
                    Some(value) => format!("CPU {value:.1}%"),
                    None => match cpu.get("status").and_then(Value::as_str) {
                        Some(status) if status != "unknown" => format!("CPU {status}"),
                        _ => "CPU unknown".to_string(),
                    },
                },
            )
            .unwrap_or_else(|| "CPU unknown".to_string());
        return truncate(
            &format!("{package}: {startup}, {jank}, {pss}, {cpu}"),
            max_text,
        );
    }
    if success.command == "app.install"
        && let Some(result) = success.result.as_object()
    {
        let package = result
            .get("build")
            .and_then(|build| build.get("package"))
            .and_then(Value::as_str)
            .or_else(|| {
                result
                    .get("apk")
                    .and_then(|apk| apk.get("package"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("app");
        if result.get("skipped").and_then(Value::as_bool) == Some(true) {
            let reason = result
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("already current");
            return truncate(&format!("Install skipped: {reason}"), max_text);
        }
        let version = result
            .get("build")
            .and_then(|build| build.get("version_name"))
            .and_then(Value::as_str)
            .unwrap_or("version unknown");
        let splits = result
            .get("split_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        return truncate(
            &format!("Installed {package} {version} ({splits} split(s))"),
            max_text,
        );
    }
    if success.command == "apk.diff"
        && let Some(result) = success.result.as_object()
    {
        let overall = result
            .get("overall")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let safe = result
            .get("safe_to_skip_install")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        return truncate(
            &format!("APK diff: {overall} (safe to skip: {safe})"),
            max_text,
        );
    }
    if success.command == "report.explain"
        && let Some(diagnosis) = success.result.get("diagnosis").and_then(Value::as_object)
        && let Some(summary) = diagnosis.get("summary").and_then(Value::as_str)
    {
        let code = diagnosis
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("failure");
        return truncate(&format!("{summary} [{code}]"), max_text);
    }
    if success.status != "success"
        && let Some(failure) = success.result.get("failure").and_then(Value::as_object)
    {
        let stage = failure
            .get("stage")
            .and_then(Value::as_str)
            .unwrap_or("pipeline");
        let message = failure
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("the command failed");
        return truncate(&format!("{stage}: {message}"), max_text);
    }
    let first = success
        .human
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .filter(|line| !line.eq_ignore_ascii_case("artifacts:"))
        .unwrap_or_else(|| {
            if success.status == "success" {
                "command completed"
            } else {
                "command did not complete"
            }
        });
    truncate(first, max_text)
}

fn next_commands(success: &CommandSuccess) -> Vec<String> {
    let mut next = Vec::new();
    if let Some(actions) = success.result.get("next_actions").and_then(Value::as_array) {
        for action in actions.iter().take(3) {
            if let Some(argv) = action
                .get("command")
                .and_then(|command| command.get("argv"))
                .and_then(Value::as_array)
            {
                let command = argv
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ");
                if !command.is_empty() && !next.contains(&command) {
                    next.push(command);
                }
            }
        }
    }
    if next.is_empty()
        && let Some(items) = success.result.get("next").and_then(Value::as_array)
    {
        for item in items.iter().take(3) {
            if let Some(command) = item.as_str()
                && !next.iter().any(|existing| existing == command)
            {
                next.push(command.to_string());
            }
        }
    }
    if next.is_empty()
        && success.command == "ui.observe"
        && let Some(token) = success.result.get("token").and_then(Value::as_str)
    {
        let session = success
            .session_id
            .as_ref()
            .map(|session| format!(" --session {session}"))
            .unwrap_or_default();
        if let Some(actionable) = success.result.get("actionable").and_then(Value::as_array) {
            if let Some(reference) = actionable
                .iter()
                .find(|node| {
                    node.get("ref").is_some()
                        && node.get("clickable").and_then(Value::as_bool) == Some(true)
                })
                .and_then(|node| node.get("ref"))
                .and_then(Value::as_str)
            {
                next.push(format!(
                    "androidforge ui tap --ref {reference} --expect-token {token}{session}"
                ));
            } else if let Some(reference) = actionable
                .iter()
                .find(|node| {
                    node.get("ref").is_some()
                        && node.get("scrollable").and_then(Value::as_bool) == Some(true)
                })
                .and_then(|node| node.get("ref"))
                .and_then(Value::as_str)
            {
                next.push(format!(
                    "androidforge ui scroll --ref {reference} --expect-token {token} --direction down{session}"
                ));
            }
        }
    }
    next.truncate(3);
    next
}

fn project_result(command: &str, mut value: Value, plan: OutputPlan) -> Value {
    match command {
        "ui.observe" => {
            compact_observation(&mut value, plan);
            value
        }
        "ui.diff" => {
            compact_diff(&mut value, plan);
            value
        }
        "ui.tap" | "ui.long-press" | "ui.text" | "ui.clear" | "ui.scroll" => {
            compact_action(&mut value, plan);
            value
        }
        "context" | "agent.brief" => {
            compact_context(&mut value, plan);
            value
        }
        "agent.test" => {
            compact_agent_test(&mut value, plan);
            value
        }
        "report.explain" => {
            compact_explain(&mut value, plan);
            value
        }
        "diagnostics.performance" => {
            compact_performance(&mut value, plan);
            value
        }
        "app.install" => {
            compact_install(&mut value, plan);
            value
        }
        "apk.diff" => {
            compact_apk_diff(&mut value, plan);
            value
        }
        "apk.inspect" | "apk.resolve" => {
            compact_apk(&mut value, plan);
            value
        }
        _ => {
            generic_compact(&mut value, plan);
            value
        }
    }
}

fn compact_observation(value: &mut Value, plan: OutputPlan) {
    let brief = plan.brief();
    let max_text = plan.limits.max_text_chars;
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for key in [
        "id",
        "observed_at",
        "session_id",
        "device_serial",
        "raw_xml_path",
        "nodes",
    ] {
        object.remove(key);
    }
    if brief {
        object.remove("display");
        object.remove("hierarchy");
    }
    if let Some(focused) = object.get_mut("focused") {
        compact_node(focused, brief, max_text);
    }
    let item_limit = if brief {
        plan.limits.max_items.min(15)
    } else {
        plan.limits.max_items
    };
    let (total, truncated) = compact_node_list(object, "actionable", item_limit, brief, max_text);
    object
        .entry("actionable_total")
        .or_insert_with(|| json!(total));
    if truncated {
        object.insert("actionable_truncated".to_string(), json!(true));
    }
    let context_limit = if brief {
        item_limit.min(4)
    } else {
        item_limit.min(8)
    };
    let (total, truncated) = compact_node_list(object, "context", context_limit, brief, max_text);
    object
        .entry("context_total")
        .or_insert_with(|| json!(total));
    if truncated {
        object.insert("context_truncated".to_string(), json!(true));
    }
    let (_, truncated) = compact_node_list(
        object,
        "scrollable",
        if brief {
            plan.limits.max_items.min(3)
        } else {
            plan.limits.max_items.min(5)
        },
        brief,
        max_text,
    );
    if truncated {
        object.insert("scrollable_truncated".to_string(), json!(true));
    }
    if let Some(text) = object.get_mut("visible_text").and_then(Value::as_array_mut) {
        let total = text.len();
        let limit = if brief {
            plan.limits.max_items.min(8)
        } else {
            plan.limits.max_items.min(20)
        };
        text.truncate(limit);
        for item in text.iter_mut() {
            if let Some(value) = item.as_str() {
                *item = Value::String(truncate(value, max_text));
            }
        }
        if total > limit {
            object.insert("visible_text_total".to_string(), json!(total));
            object.insert("visible_text_truncated".to_string(), json!(true));
        }
    }
}

fn compact_node_list(
    object: &mut Map<String, Value>,
    key: &str,
    limit: usize,
    brief: bool,
    max_text: usize,
) -> (usize, bool) {
    let Some(items) = object.get_mut(key).and_then(Value::as_array_mut) else {
        return (0, false);
    };
    let total = items.len();
    items.truncate(limit);
    for item in items {
        compact_node(item, brief, max_text);
    }
    (total, total > limit)
}

fn compact_node(node: &mut Value, brief: bool, max_text: usize) {
    let Some(object) = node.as_object_mut() else {
        return;
    };
    for key in ["class", "package", "displayed"] {
        object.remove(key);
    }
    for key in ["text", "content_desc", "hint", "resource_id"] {
        truncate_field(object, key, max_text);
    }
    if brief {
        object.remove("hint");
    }
}

fn retain_node_keys(object: &mut Map<String, Value>) {
    object.retain(|key, _| {
        matches!(
            key.as_str(),
            "path"
                | "text"
                | "content_desc"
                | "hint"
                | "resource_id"
                | "class"
                | "bounds"
                | "clickable"
                | "enabled"
                | "focused"
                | "scrollable"
                | "editable"
                | "checked"
                | "ref"
        )
    });
}

fn compact_diff(value: &mut Value, plan: OutputPlan) {
    let brief = plan.brief();
    let max_text = plan.limits.max_text_chars;
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let limit = if brief {
        plan.limits.max_items.min(3)
    } else {
        plan.limits.max_items.min(5)
    };
    if let Some(changes) = object.get_mut("changes").and_then(Value::as_object_mut) {
        for key in ["added", "removed"] {
            if let Some(items) = changes.get_mut(key).and_then(Value::as_array_mut) {
                let total = items.len();
                items.truncate(limit);
                for item in items.iter_mut() {
                    compact_node(item, brief, max_text);
                }
                if total > limit {
                    changes.insert("truncated".to_string(), json!(true));
                }
            }
        }
        if let Some(items) = changes.get_mut("modified").and_then(Value::as_array_mut) {
            let total = items.len();
            items.truncate(limit);
            for item in items.iter_mut() {
                if let Some(object) = item.as_object_mut() {
                    truncate_field(object, "path", max_text);
                    if let Some(changes) = object.get_mut("changes").and_then(Value::as_array_mut) {
                        changes.truncate(4);
                    }
                }
            }
            if total > limit {
                changes.insert("truncated".to_string(), json!(true));
            }
        }
    }
}

fn compact_action(value: &mut Value, plan: OutputPlan) {
    let brief = plan.brief();
    let max_text = plan.limits.max_text_chars;
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(receipt) = object.get_mut("receipt").and_then(Value::as_object_mut) {
        compact_receipt(receipt, plan);
    }
    if let Some(matched) = object.get_mut("matched") {
        compact_node(matched, brief, max_text);
    }
    if let Some(ancestor) = object.get_mut("ancestor") {
        compact_node(ancestor, brief, max_text);
    }
    for key in ["element_ref", "observation_token", "resolved_node"] {
        if brief && key == "resolved_node" {
            object.remove(key);
        } else if let Some(node) = object.get_mut(key) {
            compact_node(node, brief, max_text);
        }
    }
}

fn compact_receipt(receipt: &mut Map<String, Value>, plan: OutputPlan) {
    let brief = plan.brief();
    let max_text = plan.limits.max_text_chars;
    receipt.remove("target");
    receipt.remove("action_result");
    if brief {
        receipt.remove("ended_at");
    }
    if brief {
        receipt.remove("selector");
        receipt.remove("effect_evidence");
    }
    if let Some(reference) = receipt.get_mut("reference").and_then(Value::as_object_mut) {
        reference.remove("resolved_node");
        truncate_field(reference, "expected_token", max_text.max(80));
    }
    if let Some(changes) = receipt.get_mut("changes").and_then(Value::as_object_mut) {
        let limit = if brief {
            plan.limits.max_items.min(3)
        } else {
            plan.limits.max_items.min(5)
        };
        for key in ["added", "removed"] {
            if let Some(items) = changes.get_mut(key).and_then(Value::as_array_mut) {
                items.truncate(limit);
                for item in items {
                    compact_node(item, brief, max_text);
                }
            }
        }
        if let Some(items) = changes.get_mut("modified").and_then(Value::as_array_mut) {
            items.truncate(limit);
        }
    }
    if let Some(evidence) = receipt
        .get_mut("effect_evidence")
        .and_then(Value::as_array_mut)
    {
        evidence.truncate(if brief { 1 } else { 3 });
        for item in evidence {
            if let Some(text) = item.as_str() {
                *item = Value::String(truncate(text, max_text));
            }
        }
    }
    if let Some(warnings) = receipt.get_mut("warnings").and_then(Value::as_array_mut) {
        warnings.truncate(plan.limits.max_items.min(3));
        for warning in warnings {
            if let Some(text) = warning.as_str() {
                *warning = Value::String(truncate(text, max_text));
            }
        }
    }
    if let Some(postcondition) = receipt
        .get_mut("postcondition")
        .and_then(Value::as_object_mut)
        && let Some(conditions) = postcondition
            .get_mut("conditions")
            .and_then(Value::as_array_mut)
    {
        conditions.truncate(4);
    }
}

fn compact_context(value: &mut Value, plan: OutputPlan) {
    let brief = plan.brief();
    let max_text = plan.limits.max_text_chars;
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.remove("duration_ms");
    object.remove("environment");
    if let Some(project) = object.get_mut("project") {
        if brief {
            retain_available(project, &["name", "package"], true);
        } else {
            retain_available(project, &["name", "package", "root"], false);
        }
    }
    if let Some(session) = object.get_mut("session") {
        retain_available(session, &["id", "owner", "state", "serial"], true);
    }
    if let Some(device) = object.get_mut("device") {
        retain_available(
            device,
            &["serial", "state", "model", "api_level", "architecture"],
            true,
        );
    }
    if let Some(app) = object.get_mut("app") {
        retain_available(
            app,
            &[
                "package",
                "installed",
                "version_name",
                "version_code",
                "foreground",
                "process_exists",
            ],
            true,
        );
    }
    if let Some(last_run) = object.get_mut("last_run") {
        retain_available(last_run, &["run_id", "status"], true);
    }
    if let Some(screen) = object.get_mut("screen") {
        if screen.get("token").is_some() {
            compact_observation(screen, plan);
        } else if let Some(screen) = screen.as_object_mut() {
            retain_available_map(screen, &["available", "reason", "code"], true);
        }
    }
    if let Some(actions) = object.get_mut("next_actions").and_then(Value::as_array_mut) {
        actions.truncate(3);
    }
    if let Some(warnings) = object.get_mut("warnings").and_then(Value::as_array_mut) {
        warnings.truncate(3);
        for warning in warnings {
            if let Some(text) = warning.as_str() {
                *warning = Value::String(truncate(text, max_text));
            }
        }
    }
}

fn compact_agent_test(value: &mut Value, plan: OutputPlan) {
    let brief = plan.brief();
    let max_text = plan.limits.max_text_chars;
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for key in ["session", "device", "resolved_apk"] {
        object.remove(key);
    }
    if let Some(project) = object.get_mut("project").and_then(Value::as_object_mut) {
        project.retain(|key, _| matches!(key.as_str(), "name" | "package" | "root"));
    }
    if let Some(apk) = object.get_mut("apk") {
        retain_keys(
            apk,
            &[
                "package",
                "version_name",
                "version_code",
                "sha256",
                "artifact_kind",
                "split_count",
                "archive_sha256",
            ],
        );
    }
    if let Some(stages) = object.get_mut("stages").and_then(Value::as_array_mut) {
        let total = stages.len();
        let failed = stages
            .iter()
            .filter(|stage| stage.get("status").and_then(Value::as_str) == Some("failed"))
            .count();
        for stage in stages.iter_mut() {
            if brief && let Some(stage) = stage.as_object_mut() {
                // Keep the schema-required field but do not repeat long argv
                // arrays in the smallest profile.
                stage.insert("command_argv".to_string(), json!([]));
            }
            compact_stage(stage, max_text);
        }
        if brief {
            object.insert("stages_total".to_string(), json!(total));
            object.insert("stages_failed".to_string(), json!(failed));
        }
    }
    object.remove("artifacts");
    if let Some(actions) = object.get_mut("next_actions").and_then(Value::as_array_mut) {
        actions.truncate(if brief { 2 } else { 3 });
    }
    if let Some(warnings) = object.get_mut("warnings").and_then(Value::as_array_mut) {
        warnings.truncate(3);
        for warning in warnings {
            if let Some(text) = warning.as_str() {
                *warning = Value::String(truncate(text, max_text));
            }
        }
    }
}

fn compact_stage(stage: &mut Value, max_text: usize) {
    let Some(object) = stage.as_object_mut() else {
        return;
    };
    let failed = object.get("status").and_then(Value::as_str) == Some("failed");
    if let Some(details) = object.get_mut("details").and_then(Value::as_object_mut) {
        details.retain(|key, _| {
            failed
                || matches!(
                    key.as_str(),
                    "reason"
                        | "package"
                        | "version_name"
                        | "version_code"
                        | "sha256"
                        | "artifact_kind"
                        | "split_count"
                        | "archive_sha256"
                        | "matches"
                        | "proof"
                        | "run_id"
                        | "fixture"
                        | "flow"
                )
        });
        for (key, value) in details {
            if matches!(key.as_str(), "reason" | "proof" | "package")
                && let Some(text) = value.as_str()
            {
                *value = Value::String(truncate(text, max_text));
            } else if let Some(items) = value.as_array_mut() {
                items.truncate(5);
            }
        }
    }
}

fn compact_explain(value: &mut Value, plan: OutputPlan) {
    let max_text = plan.limits.max_text_chars;
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for key in ["evidence", "alternatives", "uncertainty"] {
        object.remove(key);
    }
    if let Some(diagnosis) = object.get_mut("diagnosis") {
        retain_keys(
            diagnosis,
            &[
                "category",
                "code",
                "confidence",
                "summary",
                "failed_operation",
            ],
        );
        if let Some(diagnosis) = diagnosis.as_object_mut() {
            for key in ["summary", "failed_operation"] {
                truncate_field(diagnosis, key, max_text);
            }
        }
    }
    if let Some(recurrence) = object.get_mut("recurrence") {
        retain_keys(
            recurrence,
            &[
                "status",
                "occurrence_count",
                "currently_resolved",
                "last_seen",
            ],
        );
    }
    if let Some(actions) = object.get_mut("next_actions").and_then(Value::as_array_mut) {
        actions.truncate(3);
    }
}

fn compact_performance(value: &mut Value, plan: OutputPlan) {
    let brief = plan.brief();
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert("commands".to_string(), json!([]));
    if brief {
        object.remove("foreground");
        object.remove("activity");
    }
    if let Some(startup) = object.get_mut("startup").and_then(Value::as_object_mut) {
        startup.remove("raw_output");
        if brief {
            startup.remove("status");
            startup.remove("activity");
            startup.remove("wait_time_ms");
            startup.remove("this_time_ms");
        }
    }
    if let Some(memory) = object.get_mut("memory").and_then(Value::as_object_mut) {
        memory.remove("pid");
        if brief {
            memory.remove("total_rss_kb");
            memory.remove("java_heap_pss_kb");
            memory.remove("native_heap_pss_kb");
        }
    }
    if let Some(jank) = object.get_mut("jank").and_then(Value::as_object_mut)
        && brief
    {
        let insufficient =
            jank.get("sample_status").and_then(Value::as_str) == Some("insufficient");
        // Keep the frame count for insufficient samples so brief output can
        // say "insufficient sample (N frame(s))" instead of a bogus percentage.
        if !insufficient {
            jank.remove("total_frames");
            jank.remove("janky_frames");
        }
        for key in [
            "percentile_50_ms",
            "percentile_95_ms",
            "missed_vsync",
            "window_start_ms",
            "window_end_ms",
            "frame_deadline_ms",
        ] {
            jank.remove(key);
        }
    }
    if let Some(cpu) = object.get_mut("cpu").and_then(Value::as_object_mut)
        && brief
    {
        for key in [
            "process",
            "user_percent",
            "kernel_percent",
            "window_start",
            "window_end",
        ] {
            cpu.remove(key);
        }
    }
    if let Some(warnings) = object.get_mut("warnings").and_then(Value::as_array_mut) {
        warnings.truncate(3);
    }
}

fn compact_install(value: &mut Value, plan: OutputPlan) {
    let brief = plan.brief();
    let max_text = plan.limits.max_text_chars;
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(apk) = object.get_mut("apk") {
        retain_keys(
            apk,
            &[
                "package",
                "version_name",
                "version_code",
                "sha256",
                "path",
                "signature",
            ],
        );
        if let Some(apk) = apk.as_object_mut() {
            if brief {
                apk.remove("path");
            }
            if let Some(signature) = apk.get_mut("signature").and_then(Value::as_object_mut) {
                retain_keys_map(
                    signature,
                    &["cert_sha256", "debug_signed", "verified", "signed"],
                );
            }
        }
    }
    if let Some(build) = object.get_mut("build").and_then(Value::as_object_mut) {
        for key in ["installed_apk_path", "notes"] {
            if brief {
                build.remove(key);
            }
        }
        if let Some(notes) = build.get_mut("notes").and_then(Value::as_array_mut) {
            notes.truncate(3);
        }
    }
    if let Some(installed) = object.get_mut("installed").and_then(Value::as_object_mut) {
        retain_keys_map(
            installed,
            &["package", "version_name", "version_code", "debuggable"],
        );
    }
    if let Some(splits) = object.get_mut("splits").and_then(Value::as_array_mut) {
        if brief {
            splits.clear();
        } else {
            for split in splits {
                retain_keys(split, &["name", "sha256", "size_bytes"]);
            }
        }
    }
    if let Some(diff) = object.get_mut("diff").and_then(Value::as_object_mut) {
        compact_diff_object(diff, plan);
    }
    if let Some(diagnosis) = object
        .get_mut("signature_diagnosis")
        .and_then(Value::as_object_mut)
    {
        if brief {
            object.remove("signature_diagnosis");
        } else {
            diagnosis.remove("notes");
            if let Some(suggestions) = diagnosis
                .get_mut("suggestions")
                .and_then(Value::as_array_mut)
            {
                suggestions.truncate(2);
            }
            truncate_field(diagnosis, "message", max_text);
            truncate_field(diagnosis, "install_output", max_text);
        }
    }
    if let Some(warnings) = object.get_mut("warnings").and_then(Value::as_array_mut) {
        warnings.truncate(plan.limits.max_items.min(3));
        for warning in warnings {
            if let Some(text) = warning.as_str() {
                *warning = Value::String(truncate(text, max_text));
            }
        }
    }
}

fn compact_diff_object(diff: &mut Map<String, Value>, plan: OutputPlan) {
    diff.remove("candidate");
    diff.remove("installed");
    diff.remove("candidate_splits");
    compact_diff_fields(diff, plan);
}

fn compact_diff_fields(diff: &mut Map<String, Value>, plan: OutputPlan) {
    let brief = plan.brief();
    if let Some(fields) = diff.get_mut("fields").and_then(Value::as_array_mut) {
        fields.truncate(if brief { 6 } else { 20 });
        for field in fields {
            let Some(object) = field.as_object_mut() else {
                continue;
            };
            let status = object.get("status").and_then(Value::as_str).unwrap_or("");
            if status == "same" || brief {
                object.remove("candidate");
                object.remove("installed");
                object.remove("note");
            }
        }
    }
    if let Some(warnings) = diff.get_mut("warnings").and_then(Value::as_array_mut) {
        warnings.truncate(3);
    }
}

fn compact_apk_diff(value: &mut Value, plan: OutputPlan) {
    let brief = plan.brief();
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.remove("candidate_artifact");
    object.remove("candidate_splits");
    compact_diff_fields(object, plan);
    if let Some(candidate) = object.get_mut("candidate").and_then(Value::as_object_mut) {
        candidate.retain(|key, _| {
            matches!(
                key.as_str(),
                "package" | "version_name" | "version_code" | "sha256"
            )
        });
    }
    if let Some(installed) = object.get_mut("installed").and_then(Value::as_object_mut) {
        installed.retain(|key, _| {
            matches!(
                key.as_str(),
                "package"
                    | "remote_paths"
                    | "base_remote_path"
                    | "base_apk"
                    | "split_count"
                    | "version_name"
                    | "version_code"
                    | "splits"
            )
        });
        if let Some(base) = installed.get_mut("base_apk").and_then(Value::as_object_mut) {
            base.retain(|key, _| {
                matches!(
                    key.as_str(),
                    "package" | "version_name" | "version_code" | "sha256"
                )
            });
        }
        if let Some(splits) = installed.get_mut("splits").and_then(Value::as_array_mut) {
            if brief {
                splits.clear();
            } else {
                for split in splits {
                    if let Some(split) = split.as_object_mut() {
                        split.retain(|key, _| matches!(key.as_str(), "name" | "sha256"));
                    }
                }
            }
        }
    }
}

fn compact_apk(value: &mut Value, plan: OutputPlan) {
    let brief = plan.brief();
    let max_text = plan.limits.max_text_chars;
    if value.get("selected").is_some() {
        compact_apk_resolve(value, plan);
        return;
    }
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.remove("architectures");
    object.remove("label");
    object.remove("notes");
    if let Some(signature) = object.get_mut("signature").and_then(Value::as_object_mut) {
        retain_keys_map(
            signature,
            &["cert_sha256", "debug_signed", "verified", "signed"],
        );
    }
    if let Some(artifact) = object.get_mut("artifact").and_then(Value::as_object_mut) {
        artifact.remove("base");
        if brief {
            artifact.remove("splits");
        } else if let Some(splits) = artifact.get_mut("splits").and_then(Value::as_array_mut) {
            for split in splits {
                retain_keys(split, &["name", "sha256", "size_bytes"]);
            }
        }
        if let Some(warnings) = artifact.get_mut("warnings").and_then(Value::as_array_mut) {
            warnings.truncate(3);
            for warning in warnings {
                if let Some(text) = warning.as_str() {
                    *warning = Value::String(truncate(text, max_text));
                }
            }
        }
    }
}

fn compact_apk_resolve(value: &mut Value, plan: OutputPlan) {
    let brief = plan.brief();
    let max_text = plan.limits.max_text_chars;
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if brief {
        object.remove("patterns");
        object.remove("considered");
        object.remove("rejected");
    } else if let Some(rejected) = object.get_mut("rejected").and_then(Value::as_array_mut) {
        rejected.truncate(5);
        for item in rejected {
            if let Some(item) = item.as_object_mut() {
                truncate_field(item, "reason", max_text);
            }
        }
    }
    if let Some(selected) = object.get_mut("selected").and_then(Value::as_object_mut) {
        selected.remove("architectures");
        selected.remove("label");
        selected.remove("notes");
        if let Some(signature) = selected.get_mut("signature").and_then(Value::as_object_mut) {
            retain_keys_map(
                signature,
                &["cert_sha256", "debug_signed", "verified", "signed"],
            );
        }
        if brief {
            selected.remove("splits");
        } else if let Some(splits) = selected.get_mut("splits").and_then(Value::as_array_mut) {
            for split in splits {
                retain_keys(split, &["name", "sha256", "size_bytes"]);
            }
        }
    }
}

fn generic_compact(value: &mut Value, plan: OutputPlan) {
    let brief = plan.brief();
    let max_text = plan.limits.max_text_chars;
    let limit = if brief {
        plan.limits.max_items.min(8)
    } else {
        plan.limits.max_items
    };
    generic_compact_value(value, brief, max_text, limit);
}

fn generic_compact_value(value: &mut Value, brief: bool, max_text: usize, limit: usize) {
    match value {
        Value::Object(object) => {
            object.remove("nodes");
            object.remove("raw_xml_path");
            if brief {
                object.remove("xml");
            }
            for key in [
                "actionable",
                "context",
                "visible_text",
                "stages",
                "fields",
                "warnings",
                "suggestions",
                "next_actions",
                "artifacts",
                "splits",
                "matches",
                "evidence",
                "alternatives",
                "considered",
                "rejected",
                "commands",
                "added",
                "removed",
                "modified",
            ] {
                if let Some(items) = object.get_mut(key).and_then(Value::as_array_mut) {
                    let total = items.len();
                    items.truncate(limit);
                    for item in items.iter_mut() {
                        generic_compact_value(item, brief, max_text, limit);
                    }
                    if total > items.len()
                        && !object.contains_key(&format!("{key}_total"))
                        && !object.contains_key(&format!("{key}_truncated"))
                    {
                        object.insert(format!("{key}_truncated"), json!(true));
                    }
                }
            }
            for (key, item) in object.iter_mut() {
                if matches!(
                    key.as_str(),
                    "text"
                        | "content_desc"
                        | "hint"
                        | "message"
                        | "summary"
                        | "reason"
                        | "note"
                        | "description"
                        | "excerpt"
                        | "cause"
                ) {
                    if let Some(text) = item.as_str() {
                        *item = Value::String(truncate(text, max_text));
                    }
                } else {
                    generic_compact_value(item, brief, max_text, limit);
                }
            }
        }
        Value::Array(items) => {
            items.truncate(limit);
            for item in items {
                generic_compact_value(item, brief, max_text, limit);
            }
        }
        _ => {}
    }
}

fn retain_available(value: &mut Value, keys: &[&str], drop_root: bool) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    retain_available_map(object, keys, drop_root);
}

fn retain_available_map(object: &mut Map<String, Value>, keys: &[&str], drop_root: bool) {
    if object.get("available").and_then(Value::as_bool) == Some(false) {
        object.retain(|key, _| matches!(key.as_str(), "available" | "reason" | "code"));
        return;
    }
    object.retain(|key, _| keys.contains(&key.as_str()) || (*key == "available" && !drop_root));
}

fn retain_keys(value: &mut Value, keys: &[&str]) {
    if let Some(object) = value.as_object_mut() {
        object.retain(|key, _| keys.contains(&key.as_str()));
    }
}

fn retain_keys_map(object: &mut Map<String, Value>, keys: &[&str]) {
    object.retain(|key, _| keys.contains(&key.as_str()));
}

fn truncate_field(object: &mut Map<String, Value>, key: &str, max: usize) {
    if let Some(Value::String(text)) = object.get_mut(key) {
        *text = truncate(text, max);
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max).collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(profile: OutputProfile) -> OutputPlan {
        OutputPlan {
            profile,
            limits: CompactLimits {
                max_items: 2,
                max_text_chars: 24,
            },
        }
    }

    #[test]
    fn observation_drops_duplicate_identity_and_caps_lists() {
        let mut value = json!({
            "schema_version": 1,
            "id": "obs-123456789012",
            "token": "obs-123456789012",
            "session_id": "ses-1",
            "device_serial": "emulator-5554",
            "observed_at": "2026-01-01T00:00:00Z",
            "age_ms": 12,
            "stale": false,
            "display": {"width": 1080, "height": 2400, "orientation": "portrait"},
            "foreground": {"package": "com.example"},
            "actionable": [
                {"ref": "el-aaaaaaaaaaaa", "path": "0/1", "depth": 1, "class": "Button", "package": "com.example", "text": "Save", "bounds": {"left": 0, "top": 0, "right": 10, "bottom": 10}, "clickable": true, "enabled": true},
                {"ref": "el-bbbbbbbbbbbb", "path": "0/2", "depth": 1, "class": "Button", "package": "com.example", "text": "Cancel", "bounds": {"left": 0, "top": 20, "right": 10, "bottom": 30}, "clickable": true, "enabled": true},
                {"ref": "el-cccccccccccc", "path": "0/3", "depth": 1, "class": "Button", "package": "com.example", "text": "Delete", "bounds": {"left": 0, "top": 40, "right": 10, "bottom": 50}, "clickable": true, "enabled": true}
            ],
            "actionable_total": 3,
            "context": [],
            "context_total": 0,
            "scrollable": [],
            "visible_text": ["Save", "Cancel", "Delete"],
            "nodes": [{"path": "0/1"}],
            "hierarchy": {"package": "com.example", "node_count": 3},
            "raw_xml_path": "ui/obs.xml"
        });
        compact_observation(&mut value, plan(OutputProfile::Compact));
        assert!(value.get("id").is_none());
        assert!(value.get("nodes").is_none());
        assert!(value.get("raw_xml_path").is_none());
        assert_eq!(value["actionable"].as_array().unwrap().len(), 2);
        assert_eq!(value["actionable_truncated"], true);
        assert_eq!(value["visible_text"].as_array().unwrap().len(), 2);
        assert_eq!(value["visible_text_total"], 3);
        assert!(value["actionable"][0].get("path").is_some());
        assert!(value["actionable"][0].get("class").is_none());
        assert_eq!(value["token"], "obs-123456789012");
    }

    #[test]
    fn brief_observation_drops_hierarchy_and_display() {
        let mut value = json!({
            "token": "obs-1",
            "display": {"orientation": "portrait"},
            "hierarchy": {"node_count": 10},
            "actionable": [],
            "context": [],
            "scrollable": [],
            "visible_text": [],
            "nodes": null,
            "raw_xml_path": "ui/obs.xml"
        });
        compact_observation(&mut value, plan(OutputProfile::Brief));
        assert!(value.get("display").is_none());
        assert!(value.get("hierarchy").is_none());
        assert_eq!(value["token"], "obs-1");
    }

    #[test]
    fn compact_profiles_materially_reduce_representative_payloads() {
        let context_nodes = (0..20)
            .map(|index| {
                json!({
                    "ref": format!("el-{index:012x}"),
                    "role": "text",
                    "path": format!("0/1/{index}"),
                    "depth": 3,
                    "text": format!("Settings item {index}"),
                    "resource_id": format!("com.example:id/item_{index}"),
                    "class": "android.widget.TextView",
                    "package": "com.example",
                    "bounds": {"left": 0, "top": index * 10, "right": 500, "bottom": index * 10 + 8},
                    "clickable": false,
                    "enabled": true
                })
            })
            .collect::<Vec<_>>();
        let observation = json!({
            "schema_version": 1,
            "id": "obs-123456789012",
            "token": "obs-123456789012",
            "session_id": "ses-1",
            "device_serial": "emulator-5554",
            "observed_at": "2026-01-01T00:00:00Z",
            "age_ms": 10,
            "stale": false,
            "display": {"width": 1080, "height": 2400, "orientation": "portrait"},
            "foreground": {"package": "com.example"},
            "actionable": context_nodes,
            "actionable_total": 20,
            "context": [],
            "context_total": 0,
            "scrollable": [],
            "visible_text": (0..20).map(|index| format!("Item {index}")).collect::<Vec<_>>(),
            "nodes": null,
            "hierarchy": {"package": "com.example", "node_count": 40, "compact_node_count": 20},
            "raw_xml_path": "observations/obs.xml"
        });
        let full_bytes = serde_json::to_vec(&observation).unwrap().len();
        let mut compact = observation.clone();
        compact_observation(&mut compact, plan(OutputProfile::Compact));
        let compact_bytes = serde_json::to_vec(&compact).unwrap().len();
        let mut brief = observation;
        compact_observation(&mut brief, plan(OutputProfile::Brief));
        let brief_bytes = serde_json::to_vec(&brief).unwrap().len();
        assert!(
            compact_bytes * 2 < full_bytes,
            "compact observation should be less than half (full={full_bytes}, compact={compact_bytes})"
        );
        assert!(
            brief_bytes * 3 < full_bytes,
            "brief observation should be less than a third (full={full_bytes}, brief={brief_bytes})"
        );

        let performance = json!({
            "schema_version": 1,
            "package": "com.example",
            "serial": "emulator-5554",
            "collected_at": "2026-01-01T00:00:00Z",
            "cold_start_requested": true,
            "settle_ms": 2000,
            "foreground": true,
            "activity": "com.example/.Main",
            "startup": {"requested": true, "cold": true, "status": "ok", "total_time_ms": 500, "wait_time_ms": 520, "this_time_ms": 300, "raw_output": "x".repeat(2000)},
            "jank": {"reset_before_launch": true, "total_frames": 120, "janky_frames": 9, "janky_percent": 7.5, "percentile_50_ms": 8.0, "percentile_90_ms": 16.0, "percentile_95_ms": 24.0, "percentile_99_ms": 40.0, "missed_vsync": 3},
            "memory": {"pid": 123, "total_pss_kb": 456789, "total_rss_kb": 500000, "java_heap_pss_kb": 12345, "native_heap_pss_kb": 23456},
            "cpu": {"process": "com.example", "total_percent": 12.5, "user_percent": 8.0, "kernel_percent": 4.5},
            "commands": [{"name": "startup", "command": "am start -W", "status": "ok", "excerpt": "x".repeat(3000)}],
            "warnings": []
        });
        let full_bytes = serde_json::to_vec(&performance).unwrap().len();
        let mut brief = performance;
        compact_performance(&mut brief, plan(OutputProfile::Brief));
        let brief_bytes = serde_json::to_vec(&brief).unwrap().len();
        assert!(
            brief_bytes * 4 < full_bytes,
            "brief performance should be less than a quarter (full={full_bytes}, brief={brief_bytes})"
        );
    }
    #[test]
    fn summary_prefers_failure_message_over_generic_human_text() {
        let success = CommandSuccess::new(
            "agent.test",
            json!({
                "final_status": "failed",
                "failure": {"stage": "install_apk", "message": "installing requires --install"}
            }),
            "agent test failed\n",
        )
        .with_status("failed");
        assert_eq!(
            summary_for(&success, 100),
            "install_apk: installing requires --install"
        );
    }

    #[test]
    fn next_commands_extracts_argv_and_ignores_duplicates() {
        let mut success = CommandSuccess::new(
            "report.explain",
            json!({
                "next_actions": [
                    {"command": {"argv": ["androidforge", "ui", "observe"]}},
                    {"command": {"argv": ["androidforge", "ui", "observe"]}},
                    {"command": {"argv": ["androidforge", "ui", "tap", "--ref", "el-1"]}}
                ]
            }),
            "",
        );
        success.session_id = Some("ses-1".to_string());
        assert_eq!(
            next_commands(&success),
            vec!["androidforge ui observe", "androidforge ui tap --ref el-1"]
        );
    }

    #[test]
    fn brief_context_keeps_decision_fields_and_drops_environment() {
        let value = json!({
            "schema_version": 1,
            "generated_at": "2026-01-01T00:00:00Z",
            "duration_ms": 10,
            "project": {"available": true, "name": "Example", "package": "com.example", "root": "A:/x"},
            "environment": {"sdk": "..."},
            "session": {"available": true, "id": "ses-1", "owner": "agent", "state": "ready", "serial": "emulator-5554", "build": {}},
            "device": {"available": true, "serial": "emulator-5554", "state": "device", "model": "Pixel", "api_level": 36, "manufacturer": "Google"},
            "app": {"available": true, "package": "com.example", "installed": true, "version_name": "1.0", "version_code": 1, "foreground": true},
            "screen": {"available": true, "token": "obs-1", "actionable": [], "context": [], "scrollable": [], "visible_text": [], "raw_xml_path": "x"},
            "last_run": {"available": true, "run_id": "run-1", "status": "passed"},
            "next_actions": [{"command": {"argv": ["androidforge", "ui", "observe"]}}],
            "warnings": ["w1", "w2", "w3", "w4"]
        });
        let mut brief = value.clone();
        compact_context(
            &mut brief,
            OutputPlan {
                profile: OutputProfile::Brief,
                limits: CompactLimits::default(),
            },
        );
        assert!(brief.get("environment").is_none());
        assert_eq!(brief["next_actions"].as_array().unwrap().len(), 1);
        assert_eq!(brief["project"]["name"], "Example");
        assert_eq!(brief["session"]["id"], "ses-1");
        assert_eq!(brief["device"]["model"], "Pixel");
        assert_eq!(brief["screen"]["token"], "obs-1");
        assert!(brief["screen"].get("raw_xml_path").is_none());
        assert_eq!(brief["warnings"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn error_compaction_bounds_report_and_suggestions() {
        let mut error = ForgeError::new(
            androidforge_core::error::ErrorCode::CommandFailed,
            "failed",
        )
        .with_suggestion("one")
        .with_suggestion("two")
        .with_suggestion("three")
        .with_suggestion("four")
        .with_context(
            "report",
            json!({"status": "failed", "steps": [{"status": "failed"}, {"status": "passed"}]}),
        );
        apply_error(&mut error, plan(OutputProfile::Brief));
        assert_eq!(error.body().suggestions.len(), 2);
        assert_eq!(error.body().context["report"]["failed_steps"], 1);
        assert_eq!(error.body().context["report"]["total_steps"], 2);
    }

    #[test]
    fn agent_test_brief_keeps_failure_and_stage_counts() {
        let mut value = json!({
            "schema_version": 1,
            "pipeline": "agent.test",
            "dry_run": false,
            "generated_at": "2026-01-01T00:00:00Z",
            "project": {"name": "Example", "root": "A:/example"},
            "stages": [
                {"name": "ensure_session", "status": "passed", "duration_ms": 1, "command_argv": [], "details": {}},
                {"name": "install_apk", "status": "failed", "duration_ms": 2, "command_argv": ["androidforge", "app", "install"], "details": {"reason": "needs --install"}}
            ],
            "final_status": "failed",
            "failure": {"stage": "install_apk", "code": "E_INVALID_ARGUMENT", "message": "installing requires --install"},
            "run_id": "run-1",
            "report_path": "run-1/report.json",
            "artifacts": [{"kind": "x", "path": "x"}],
            "warnings": ["w1", "w2", "w3", "w4"],
            "next_actions": [{"command": {"argv": ["androidforge", "agent", "test"]}}]
        });
        compact_agent_test(&mut value, plan(OutputProfile::Brief));
        assert_eq!(value["stages"].as_array().unwrap().len(), 2);
        assert_eq!(value["stages"][1]["name"], "install_apk");
        assert_eq!(value["stages"][1]["status"], "failed");
        assert_eq!(
            value["stages"][1]["command_argv"].as_array().unwrap().len(),
            0,
            "brief stages keep the schema-required argv field empty"
        );
        assert_eq!(value["stages_total"], 2);
        assert_eq!(value["stages_failed"], 1);
        assert_eq!(value["failure"]["stage"], "install_apk");
        assert_eq!(value["final_status"], "failed");
        assert_eq!(value["next_actions"].as_array().unwrap().len(), 1);
        assert!(value.get("artifacts").is_none());
        assert!(value.get("project").is_some());
    }

    #[test]
    fn receipt_compaction_keeps_required_identity_and_effect() {
        let mut value = json!({
            "receipt": {
                "schema_version": 1,
                "receipt_id": "rcpt-abcdefabcdef",
                "action": "tap",
                "session_id": "ses-1",
                "device_serial": "emulator-5554",
                "started_at": "2026-01-01T00:00:00Z",
                "ended_at": "2026-01-01T00:00:01Z",
                "duration_ms": 900,
                "target_resolved": true,
                "dispatched": true,
                "dispatch_state": "dispatched",
                "effect": "content_changed",
                "completeness": "complete",
                "target": {"path": "0/1", "text": "Save"},
                "reference": {"ref": "el-aaaaaaaaaaaa", "expected_token": "obs-1", "resolved_node": {"path": "0/1"}, "validation": "validated"},
                "before": {"token": "obs-1", "observed_at": "2026-01-01T00:00:00Z"},
                "after": {"token": "obs-2", "observed_at": "2026-01-01T00:00:01Z"},
                "changes": {"added": [], "removed": [], "modified": [], "added_total": 0, "removed_total": 0, "modified_total": 0, "truncated": false, "focus_changed": false, "package_changed": false, "activity_changed": false},
                "warnings": ["w1", "w2", "w3", "w4"],
                "action_result": {"matched": {"path": "0/1"}}
            }
        });
        compact_action(&mut value, plan(OutputProfile::Brief));
        let receipt = &value["receipt"];
        assert_eq!(receipt["receipt_id"], "rcpt-abcdefabcdef");
        assert_eq!(receipt["session_id"], "ses-1");
        assert_eq!(receipt["device_serial"], "emulator-5554");
        assert_eq!(receipt["effect"], "content_changed");
        assert_eq!(receipt["before"]["observed_at"], "2026-01-01T00:00:00Z");
        assert!(receipt.get("target").is_none());
        assert!(receipt.get("action_result").is_none());
        assert_eq!(receipt["warnings"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn performance_brief_keeps_primary_metrics_and_unknown_reasons() {
        let mut value = json!({
            "schema_version": 1,
            "package": "com.example",
            "serial": "emulator-5554",
            "collected_at": "2026-01-01T00:00:00Z",
            "cold_start_requested": true,
            "settle_ms": 2000,
            "startup": {"requested": true, "cold": true, "status": "ok", "total_time_ms": 555, "wait_time_ms": 557, "raw_output": "x"},
            "jank": {"reset_before_launch": true, "total_frames": 25, "janky_frames": 7, "janky_percent": 28.0, "percentile_50_ms": 17.0},
            "memory": {"pid": 123, "total_pss_kb": 57177, "total_rss_kb": 190900, "unknown_reason": null},
            "cpu": {"process": "pid 123", "total_percent": 0.0, "user_percent": 0.0, "kernel_percent": 0.0, "unknown_reason": null},
            "commands": [{"name": "startup", "command": "am start", "status": "ok"}],
            "warnings": []
        });
        compact_performance(&mut value, plan(OutputProfile::Brief));
        assert!(value["commands"].as_array().unwrap().is_empty());
        assert_eq!(value["startup"]["total_time_ms"], 555);
        assert!(value["startup"].get("wait_time_ms").is_none());
        assert_eq!(value["jank"]["janky_percent"], 28.0);
        assert_eq!(value["memory"]["total_pss_kb"], 57177);
        assert!(value["memory"].get("total_rss_kb").is_none());
        assert_eq!(value["cpu"]["total_percent"], 0.0);
    }

    #[test]
    fn performance_summary_reports_insufficient_sample_not_hundred_percent() {
        let success = CommandSuccess::new(
            "diagnostics.performance",
            json!({
                "package": "com.example",
                "startup": {"requested": true, "cold": false, "total_time_ms": 100},
                "jank": {
                    "reset_before_launch": true,
                    "total_frames": 1,
                    "janky_frames": 1,
                    "janky_percent": null,
                    "sample_status": "insufficient",
                    "comparable": false,
                    "comparison_reason": "insufficient sample (1 frame(s)); at least 30 are required"
                },
                "memory": {"total_pss_kb": 1000},
                "cpu": {"status": "unavailable", "total_percent": null}
            }),
            "",
        );
        let summary = summary_for(&success, 240);
        assert!(
            summary.contains("insufficient sample (1 frame(s))"),
            "summary: {summary}"
        );
        assert!(!summary.contains("100.0%"), "summary: {summary}");
        assert!(summary.contains("CPU unavailable"), "summary: {summary}");
    }

    #[test]
    fn performance_brief_keeps_quality_status_and_insufficient_frame_count() {
        let mut value = json!({
            "schema_version": 1,
            "package": "com.example",
            "screen_on": true,
            "startup": {"requested": true, "cold": false, "total_time_ms": 100},
            "jank": {
                "reset_before_launch": true,
                "total_frames": 1,
                "janky_frames": 1,
                "janky_percent": null,
                "sample_status": "insufficient",
                "comparable": false,
                "comparison_reason": "insufficient sample (1 frame(s)); at least 30 are required",
                "window_start_ms": 10,
                "window_end_ms": 20,
                "frame_deadline_ms": 16.67
            },
            "memory": {"pid": 123, "total_pss_kb": 1000},
            "cpu": {
                "status": "stale",
                "reason": "cpuinfo window is stale",
                "total_percent": null,
                "process": "com.example",
                "window_start": 10,
                "window_end": 20,
                "sample_pid": 123
            },
            "sample_quality": {
                "jank": {"status": "insufficient", "reason": "insufficient sample (1 frame(s))"},
                "cpu": {"status": "stale", "reason": "cpuinfo window is stale"}
            },
            "commands": [],
            "warnings": []
        });
        compact_performance(&mut value, plan(OutputProfile::Brief));
        assert_eq!(value["jank"]["sample_status"], "insufficient");
        assert_eq!(value["jank"]["total_frames"], 1);
        assert!(value["jank"].get("window_start_ms").is_none());
        assert_eq!(value["cpu"]["status"], "stale");
        assert_eq!(value["cpu"]["reason"], "cpuinfo window is stale");
        assert!(value["cpu"].get("window_start").is_none());
        assert_eq!(value["sample_quality"]["jank"]["status"], "insufficient");
        assert_eq!(value["screen_on"], true);
    }

    #[test]
    fn apk_diff_compaction_keeps_installed_identity_fields() {
        let mut value = json!({
            "schema_version": 1,
            "candidate": {"package": "com.example", "version_name": "1.0", "version_code": 1, "sha256": "abc", "min_sdk": 24},
            "installed": {
                "package": "com.example",
                "remote_paths": ["/data/app/base.apk", "/data/app/split.apk"],
                "base_remote_path": "/data/app/base.apk",
                "base_apk": {"package": "com.example", "version_name": "1.0", "version_code": 1, "sha256": "abc", "min_sdk": 24},
                "split_count": 1,
                "splits": [{"name": "split.apk", "remote_path": "/data/app/split.apk", "sha256": "def", "size_bytes": 10}],
                "warnings": []
            },
            "candidate_splits": [{"name": "split.apk", "path": "C:/tmp/split.apk", "sha256": "def", "size_bytes": 10}],
            "candidate_artifact": {"kind": "split_archive"},
            "fields": [{"field": "package", "status": "same"}, {"field": "split_apks", "status": "different", "note": "x"}],
            "summary": "different",
            "overall": "different",
            "safe_to_skip_install": false,
            "warnings": []
        });
        compact_apk_diff(&mut value, plan(OutputProfile::Compact));
        assert!(value.get("candidate_artifact").is_none());
        assert!(value.get("candidate_splits").is_none());
        assert_eq!(value["installed"]["package"], "com.example");
        assert_eq!(value["installed"]["split_count"], 1);
        assert_eq!(value["installed"]["splits"][0]["name"], "split.apk");
        assert!(value["installed"]["splits"][0].get("remote_path").is_none());
        assert_eq!(value["fields"][1]["status"], "different");
    }

    #[test]
    fn explain_compaction_keeps_required_diagnosis_fields() {
        let mut value = json!({
            "schema_version": 1,
            "generated_at": "2026-01-01T00:00:00Z",
            "run_id": "run-1",
            "project": "Example",
            "project_root": "A:/example",
            "scenario": "conformance:smoke",
            "status": "failed",
            "diagnosis": {"category": "assertion", "code": "E_UI_ASSERTION_FAILED", "confidence": "high", "summary": "text not found", "failed_operation": "ui.wait"},
            "alternatives": [{"category": "timeout"}],
            "evidence": [{"source": "x", "description": "y"}],
            "next_actions": [{"command": {"argv": ["androidforge", "ui", "observe"]}}],
            "fingerprint": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "fingerprint_version": 1,
            "artifacts": [],
            "uncertainty": ["x"],
            "run_dir": "A:/runs/run-1"
        });
        compact_explain(&mut value, plan(OutputProfile::Brief));
        assert!(value.get("evidence").is_none());
        assert!(value.get("alternatives").is_none());
        assert!(value.get("uncertainty").is_none());
        assert_eq!(value["diagnosis"]["category"], "assertion");
        assert_eq!(value["fingerprint_version"], 1);
        assert_eq!(value["project_root"], "A:/example");
        assert_eq!(value["next_actions"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn compact_install_keeps_identity_and_drops_split_paths() {
        let mut value = json!({
            "skipped": false,
            "split_count": 1,
            "artifact_kind": "split_archive",
            "archive_sha256": "abc",
            "apk": {"package": "com.example", "version_name": "1.0", "version_code": 1, "sha256": "def", "min_sdk": 24, "signature": {"cert_sha256": "ABCD", "debug_signed": true, "verified": true, "errors": ["x"]}},
            "build": {"package": "com.example", "apk_path": "A:/bundle.apks", "apk_sha256": "def", "installed_apk_sha256": "def", "notes": ["one", "two", "three", "four"]},
            "installed": {"package": "com.example", "version_name": "1.0", "version_code": 1, "data_dir": "/data/user/0/x", "flags": ["DEBUGGABLE"]},
            "splits": [{"name": "config.en.apk", "path": "C:/tmp/split.apk", "sha256": "123", "size_bytes": 10}],
            "warnings": ["one", "two", "three", "four"]
        });
        compact_install(&mut value, plan(OutputProfile::Compact));
        assert_eq!(value["split_count"], 1);
        assert_eq!(value["apk"]["package"], "com.example");
        assert_eq!(value["apk"]["signature"]["cert_sha256"], "ABCD");
        assert!(value["apk"].get("min_sdk").is_none());
        assert!(value["installed"].get("data_dir").is_none());
        assert!(value["splits"][0].get("path").is_none());
        assert_eq!(value["warnings"].as_array().unwrap().len(), 2);
    }
}
