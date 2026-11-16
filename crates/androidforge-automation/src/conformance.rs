use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use androidforge_core::config::ForgeConfig;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::{ArtifactRef, BuildIdentity};
use androidforge_core::paths::ForgePaths;
use androidforge_core::run::RunContext;
use androidforge_core::session::Session;
use androidforge_core::util::relative_to;
use androidforge_ui::UiManager;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fixture::{FixtureRunOptions, FixtureStatus, run_fixture};
use crate::maestro::run_maestro;

const CONFORMANCE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConformanceManifest {
    #[serde(default = "default_version")]
    pub version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_fixture: Option<String>,
    pub flow: ConformanceFlow,
    #[serde(default)]
    pub expect: ConformanceExpectation,
    #[serde(skip)]
    pub source_path: Option<String>,
}

fn default_version() -> u32 {
    CONFORMANCE_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConformanceFlow {
    Script {
        command: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
    },
    Maestro {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
    },
    Fixture {
        name: String,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConformanceExpectation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub json_paths: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceCheck {
    pub name: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceResult {
    pub name: String,
    pub path: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<ConformanceCheck>,
    /// Named checks emitted by the external flow itself (for example a plain
    /// Python script's final JSON line). They are evidence; the flow exit code
    /// remains the authority for pass/fail.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flow_checks: Vec<ConformanceCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub build: BuildIdentity,
    pub duration_ms: u64,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_conformance(
    config: &ForgeConfig,
    session: &Session,
    app: &androidforge_app::AppManager,
    ui: &UiManager,
    paths: &ForgePaths,
    run: &mut RunContext,
    flow_override: Option<&Path>,
    confirmed: bool,
    trace: bool,
) -> Result<ConformanceResult> {
    let path = resolve_conformance_path(config, flow_override)?;
    let manifest = load_conformance(&path)?;
    let started = Instant::now();
    let step = run.start_step(
        format!("conformance:{}", manifest.name),
        format!("conformance {}", manifest.name),
    )?;
    let mut artifacts = Vec::new();
    let mut warnings = Vec::new();

    if let Some(fixture) = &manifest.setup_fixture {
        let mut options = FixtureRunOptions::new(fixture.clone());
        options.confirmed = confirmed;
        options.trace = trace;
        match run_fixture(config, session, app, ui, paths, run, &options).await {
            Ok(result) => {
                artifacts.extend(result.artifacts);
                warnings.extend(result.warnings);
                if result.status != FixtureStatus::Success {
                    let error = ForgeError::new(
                        ErrorCode::FixtureFailed,
                        format!("conformance setup fixture '{}' did not succeed", fixture),
                    )
                    .with_context_string("fixture_status", format!("{:?}", result.status))
                    .with_artifacts(artifacts.clone());
                    run.finish_step_failure(step, &error, artifacts)?;
                    return Err(error);
                }
            }
            Err(error) => {
                run.finish_step_failure(step, &error, artifacts)?;
                return Err(error);
            }
        }
    }

    let (flow_exit_code, flow_success, mut flow_artifacts, flow_warnings, flow_checks) =
        match &manifest.flow {
            ConformanceFlow::Script {
                command,
                timeout_secs,
            } => {
                let command = command
                    .iter()
                    .map(|arg| resolve_command_arg(arg, config))
                    .collect::<Result<Vec<_>>>()?;
                let mut options = crate::external::ExternalRunOptions::new(
                    command,
                    format!("conformance-{}", manifest.name),
                );
                options.timeout = Duration::from_secs(timeout_secs.unwrap_or(600));
                options.capture_logcat = true;
                options.capture_screen = true;
                options.trace = trace;
                options.package = config.package().map(str::to_string);
                let result = crate::external::run_external(
                    &app.adb,
                    app,
                    ui,
                    session,
                    run,
                    &config.paths.project_root,
                    &options,
                )
                .await?;
                let flow_checks = script_checks(&run.dir, result.stdout_artifact.as_deref());
                (
                    result.exit_code,
                    result.success,
                    result.artifacts,
                    result.warnings,
                    flow_checks,
                )
            }
            ConformanceFlow::Maestro { path, timeout_secs } => {
                let flow_path = resolve_local_path(path, config);
                let result = run_maestro(
                    config,
                    &app.adb,
                    session,
                    run,
                    &flow_path,
                    Duration::from_secs(timeout_secs.unwrap_or(900)),
                    trace,
                )
                .await?;
                (
                    result.exit_code,
                    result.success,
                    result.artifacts,
                    result.warnings,
                    Vec::new(),
                )
            }
            ConformanceFlow::Fixture { name } => {
                let mut options = FixtureRunOptions::new(name.clone());
                options.confirmed = confirmed;
                options.trace = trace;
                let result = run_fixture(config, session, app, ui, paths, run, &options).await?;
                (
                    Some(if result.status == FixtureStatus::Success {
                        0
                    } else {
                        1
                    }),
                    result.status == FixtureStatus::Success,
                    result.artifacts,
                    result.warnings,
                    Vec::new(),
                )
            }
        };
    artifacts.append(&mut flow_artifacts);
    warnings.extend(flow_warnings);

    let mut checks = Vec::new();
    if let Some(expected_status) = &manifest.expect.status {
        let passed = match expected_status.as_str() {
            "success" | "passed" => flow_success,
            "failure" | "failed" => !flow_success,
            _ => false,
        };
        checks.push(ConformanceCheck {
            name: "status".to_string(),
            passed,
            expected: Some(expected_status.clone()),
            actual: Some(if flow_success { "success" } else { "failure" }.to_string()),
            detail: None,
        });
    }
    if let Some(expected_code) = manifest.expect.exit_code {
        checks.push(ConformanceCheck {
            name: "exit_code".to_string(),
            passed: flow_exit_code == Some(expected_code),
            expected: Some(expected_code.to_string()),
            actual: flow_exit_code.map(|code| code.to_string()),
            detail: None,
        });
    }
    for pattern in &manifest.expect.artifacts {
        let matched = find_artifact(&run.dir, pattern);
        checks.push(ConformanceCheck {
            name: format!("artifact:{pattern}"),
            passed: matched.is_some(),
            expected: Some(pattern.clone()),
            actual: matched.clone(),
            detail: None,
        });
    }
    if !manifest.expect.json_paths.is_empty() {
        let report = serde_json::to_value(&run.report)?;
        for (path, expected) in &manifest.expect.json_paths {
            let actual = json_path(&report, path);
            checks.push(ConformanceCheck {
                name: format!("json:{path}"),
                passed: actual.as_ref() == Some(expected),
                expected: Some(expected.to_string()),
                actual: actual.map(|value| value.to_string()),
                detail: None,
            });
        }
    }
    if flow_checks.iter().any(|check| !check.passed) {
        warnings.push(
            "the external flow reported failed named checks; inspect flow_checks and the flow output"
                .to_string(),
        );
    }
    let passed = flow_success && checks.iter().all(|check| check.passed);
    let build = run.report.build.clone();
    let result = ConformanceResult {
        name: manifest.name.clone(),
        path: manifest.source_path.clone().unwrap_or_default(),
        passed,
        checks: checks.clone(),
        flow_checks: flow_checks.clone(),
        flow_exit_code,
        artifacts: artifacts.clone(),
        warnings: warnings.clone(),
        build: build.clone(),
        duration_ms: started.elapsed().as_millis() as u64,
    };
    if !passed {
        let failed = checks
            .iter()
            .filter(|check| !check.passed)
            .map(|check| check.name.clone())
            .collect::<Vec<_>>();
        let error = ForgeError::new(
            ErrorCode::UiAssertionFailed,
            format!("conformance flow '{}' failed", manifest.name),
        )
        .with_context(
            "checks",
            serde_json::to_value(&checks).unwrap_or(Value::Null),
        )
        .with_context_string("failed_checks", failed.join(", "))
        .with_artifacts(artifacts.clone())
        .with_suggestion("Inspect the run's artifacts and `report.json`.")
        .session(session.id.clone());
        run.finish_step_failure(step, &error, artifacts)?;
        return Err(error);
    }
    run.finish_step_success(
        step,
        serde_json::json!({
            "name": manifest.name,
            "flow_exit_code": flow_exit_code,
            "checks": checks,
            "flow_checks": flow_checks,
            "build": build,
            "status": "passed",
        }),
        artifacts,
    )?;
    Ok(result)
}

/// Read named checks from the external flow's final JSON object. This keeps a
/// plain Python/Java harness machine-readable without making its exit code any
/// less authoritative.
fn script_checks(run_dir: &Path, stdout_artifact: Option<&str>) -> Vec<ConformanceCheck> {
    let Some(relative) = stdout_artifact else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(run_dir.join(relative)) else {
        return Vec::new();
    };
    for line in text.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(checks) = value.get("checks").and_then(Value::as_array) else {
            continue;
        };
        return checks
            .iter()
            .filter_map(|check| {
                let name = check.get("name")?.as_str()?.to_string();
                let passed = check.get("passed")?.as_bool()?;
                Some(ConformanceCheck {
                    name,
                    passed,
                    expected: check.get("expected").map(value_text),
                    actual: check.get("actual").map(value_text),
                    detail: check.get("detail").cloned(),
                })
            })
            .collect();
    }
    Vec::new()
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn resolve_conformance_path(config: &ForgeConfig, override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            config.paths.project_root.join(path)
        };
        if path.is_file() {
            return Ok(path);
        }
        if path.is_dir() {
            for candidate in ["conformance.toml", "flow.toml", "release-smoke.toml"] {
                let candidate = path.join(candidate);
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
        return Err(ForgeError::new(
            ErrorCode::ConfigInvalid,
            format!("conformance flow {} was not found", path.display()),
        )
        .with_suggestion(
            "Pass a .toml conformance manifest or a directory containing conformance.toml.",
        ));
    }
    if let Some(path) = config.conformance_path(None)
        && path.is_file()
    {
        return Ok(path);
    }
    for candidate in [
        ".androidforge/conformance/release-smoke.toml",
        ".androidforge/conformance/conformance.toml",
        "conformance/release-smoke.toml",
    ] {
        let path = config.paths.project_root.join(candidate);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(ForgeError::new(
        ErrorCode::ConfigInvalid,
        "no conformance flow was configured",
    )
    .with_suggestion("Set [conformance].flow in .androidforge/project.toml.")
    .with_suggestion("Or pass --flow <path>."))
}

fn load_conformance(path: &Path) -> Result<ConformanceManifest> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        ForgeError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "could not read conformance manifest {}: {error}",
                path.display()
            ),
        )
    })?;
    let mut manifest: ConformanceManifest = toml::from_str(&text).map_err(|error| {
        ForgeError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "could not parse conformance manifest {}: {error}",
                path.display()
            ),
        )
    })?;
    if manifest.version != CONFORMANCE_VERSION {
        return Err(ForgeError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "unsupported conformance version {}; expected {CONFORMANCE_VERSION}",
                manifest.version
            ),
        ));
    }
    match &manifest.flow {
        ConformanceFlow::Script { command, .. } if command.is_empty() => {
            return Err(ForgeError::new(
                ErrorCode::ConfigInvalid,
                "conformance script flow has an empty command",
            ));
        }
        _ => {}
    }
    manifest.source_path = Some(path.to_string_lossy().into_owned());
    Ok(manifest)
}

fn resolve_command_arg(value: &str, config: &ForgeConfig) -> Result<String> {
    let mut value = value.replace(
        "${PROJECT_ROOT}",
        &config.paths.project_root.to_string_lossy(),
    );
    let regex = regex::Regex::new(r"\$\{ENV:([A-Za-z_][A-Za-z0-9_]*)\}").expect("env regex");
    let mut output = String::new();
    let mut last = 0;
    for captures in regex.captures_iter(&value) {
        let whole = captures.get(0).unwrap();
        output.push_str(&value[last..whole.start()]);
        let name = captures.get(1).unwrap().as_str();
        output.push_str(&std::env::var(name).map_err(|_| {
            ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!("conformance command references missing environment variable {name}"),
            )
        })?);
        last = whole.end();
    }
    output.push_str(&value[last..]);
    value = output;
    Ok(value)
}

fn resolve_local_path(value: &str, config: &ForgeConfig) -> PathBuf {
    let value = value.replace(
        "${PROJECT_ROOT}",
        &config.paths.project_root.to_string_lossy(),
    );
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        config.paths.project_root.join(path)
    }
}

fn find_artifact(root: &Path, pattern: &str) -> Option<String> {
    let pattern = pattern.replace('\\', "/");
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    files.into_iter().find(|path| glob_match(&pattern, path))
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else if path.is_file() {
            files.push(relative_to(&path, root));
        }
    }
}

fn glob_match(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase().chars().collect::<Vec<_>>();
    let value = value
        .replace('\\', "/")
        .to_ascii_lowercase()
        .chars()
        .collect::<Vec<_>>();
    let mut memo = vec![vec![None; value.len() + 1]; pattern.len() + 1];
    fn inner(
        pattern: &[char],
        value: &[char],
        pi: usize,
        vi: usize,
        memo: &mut Vec<Vec<Option<bool>>>,
    ) -> bool {
        if let Some(cached) = memo[pi][vi] {
            return cached;
        }
        let result = if pi == pattern.len() {
            vi == value.len()
        } else if pattern[pi] == '*' {
            if pattern.get(pi + 1) == Some(&'*') {
                inner(pattern, value, pi + 2, vi, memo)
                    || (vi < value.len() && inner(pattern, value, pi, vi + 1, memo))
            } else {
                inner(pattern, value, pi + 1, vi, memo)
                    || (vi < value.len()
                        && value[vi] != '/'
                        && inner(pattern, value, pi, vi + 1, memo))
            }
        } else if pattern[pi] == '?' {
            vi < value.len() && value[vi] != '/' && inner(pattern, value, pi + 1, vi + 1, memo)
        } else {
            vi < value.len()
                && pattern[pi] == value[vi]
                && inner(pattern, value, pi + 1, vi + 1, memo)
        };
        memo[pi][vi] = Some(result);
        result
    }
    inner(&pattern, &value, 0, 0, &mut memo)
}

fn json_path(root: &Value, path: &str) -> Option<Value> {
    let path = path.strip_prefix("report.").unwrap_or(path);
    let mut current = root;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_artifact_paths() {
        assert!(glob_match("screenshots/*.png", "screenshots/start.png"));
        assert!(!glob_match("screenshots/*.png", "logs/start.png"));
        assert!(glob_match("logs/**/logcat*.txt", "logs/a/b/logcat-1.txt"));
        assert!(glob_match("report.json", "report.json"));
    }

    #[test]
    fn json_path_reads_nested_values() {
        let value = serde_json::json!({"status": "passed", "build": {"package": "com.example"}});
        assert_eq!(
            json_path(&value, "status"),
            Some(Value::String("passed".to_string()))
        );
        assert_eq!(
            json_path(&value, "build.package"),
            Some(Value::String("com.example".to_string()))
        );
    }

    #[test]
    fn external_flow_checks_are_parsed_from_the_last_json_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("logs")).unwrap();
        std::fs::write(
            dir.path().join("logs/stdout.txt"),
            "starting flow\n{\"status\":\"success\",\"checks\":[{\"name\":\"app foreground\",\"passed\":true,\"detail\":{\"activity\":\"MainActivity\"}}]}\n",
        )
        .unwrap();
        let checks = script_checks(dir.path(), Some("logs/stdout.txt"));
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "app foreground");
        assert!(checks[0].passed);
        assert!(checks[0].detail.is_some());
    }
}
