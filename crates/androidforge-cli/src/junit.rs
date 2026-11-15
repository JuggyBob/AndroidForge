use std::path::Path;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::ArtifactRef;
use androidforge_core::run::RunContext;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JunitCase {
    pub suite: String,
    pub classname: Option<String>,
    pub name: String,
    pub status: String,
    pub time_seconds: Option<f64>,
    pub failure: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JunitSummary {
    pub passed: bool,
    pub suites: usize,
    pub tests: usize,
    pub failures: usize,
    pub errors: usize,
    pub skipped: usize,
    pub cases: Vec<JunitCase>,
    pub artifact: ArtifactRef,
}

pub fn import_junit(run: &mut RunContext, path: &Path) -> Result<JunitSummary> {
    if !path.is_file() {
        return Err(ForgeError::new(
            ErrorCode::JunitInvalid,
            format!("JUnit XML {} was not found", path.display()),
        ));
    }
    let text = std::fs::read_to_string(path).map_err(|error| {
        ForgeError::new(
            ErrorCode::JunitInvalid,
            format!("could not read JUnit XML {}: {error}", path.display()),
        )
    })?;
    let document = roxmltree::Document::parse(&text).map_err(|error| {
        ForgeError::new(
            ErrorCode::JunitInvalid,
            format!("could not parse JUnit XML {}: {error}", path.display()),
        )
    })?;
    let root = document.root_element();
    let suite_nodes = if root.tag_name().name() == "testsuites" {
        root.children()
            .filter(|node| node.is_element() && node.tag_name().name() == "testsuite")
            .collect::<Vec<_>>()
    } else if root.tag_name().name() == "testsuite" {
        vec![root]
    } else {
        return Err(ForgeError::new(
            ErrorCode::JunitInvalid,
            format!(
                "JUnit XML root is <{}>; expected <testsuites> or <testsuite>",
                root.tag_name().name()
            ),
        ));
    };
    if suite_nodes.is_empty() {
        return Err(ForgeError::new(
            ErrorCode::JunitInvalid,
            "JUnit XML contains no testsuite elements",
        ));
    }
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "junit.xml".to_string());
    let artifact = run.write_text_artifact(
        &format!("junit/{file_name}"),
        "junit_xml",
        Some("Imported JUnit XML"),
        &text,
    )?;
    let mut summary = JunitSummary {
        passed: true,
        suites: suite_nodes.len(),
        tests: 0,
        failures: 0,
        errors: 0,
        skipped: 0,
        cases: Vec::new(),
        artifact: artifact.clone(),
    };
    let mut artifacts = vec![artifact];
    for suite in suite_nodes {
        let suite_name = suite
            .attribute("name")
            .unwrap_or("unnamed suite")
            .to_string();
        let mut suite_failures = 0usize;
        let mut suite_errors = 0usize;
        let mut suite_skipped = 0usize;
        let mut suite_tests = 0usize;
        let mut failed_names = Vec::new();
        let mut cases = Vec::new();
        for case in suite
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "testcase")
        {
            suite_tests += 1;
            let name = case.attribute("name").unwrap_or("unnamed test").to_string();
            let classname = case.attribute("classname").map(str::to_string);
            let time_seconds = case
                .attribute("time")
                .and_then(|value| value.parse::<f64>().ok());
            let failure_node = case.children().find(|node| {
                node.is_element()
                    && matches!(node.tag_name().name(), "failure" | "error" | "skipped")
            });
            let status = failure_node
                .map(|node| node.tag_name().name().to_string())
                .unwrap_or_else(|| "passed".to_string());
            let failure = failure_node.map(|node| {
                let message = node.attribute("message").unwrap_or("");
                let text = node.text().unwrap_or("");
                if message.is_empty() {
                    text.trim().to_string()
                } else {
                    format!("{message}: {}", text.trim())
                }
            });
            match status.as_str() {
                "failure" => {
                    suite_failures += 1;
                    failed_names.push(name.clone());
                }
                "error" => {
                    suite_errors += 1;
                    failed_names.push(name.clone());
                }
                "skipped" => suite_skipped += 1,
                _ => {}
            }
            cases.push(JunitCase {
                suite: suite_name.clone(),
                classname,
                name,
                status,
                time_seconds,
                failure,
            });
        }
        summary.tests += suite_tests;
        summary.failures += suite_failures;
        summary.errors += suite_errors;
        summary.skipped += suite_skipped;
        summary.cases.extend(cases);
        let step = run.start_step(
            format!("junit:{suite_name}"),
            format!("report import junit {}", file_name),
        )?;
        if suite_failures + suite_errors > 0 {
            summary.passed = false;
            let error = ForgeError::new(
                ErrorCode::UiAssertionFailed,
                format!(
                    "JUnit suite {suite_name} reported {} failure(s) and {} error(s)",
                    suite_failures, suite_errors
                ),
            )
            .with_context_string("suite", suite_name.clone())
            .with_context(
                "failed_tests",
                serde_json::to_value(&failed_names).unwrap_or(serde_json::Value::Null),
            )
            .with_artifacts(artifacts.clone());
            run.finish_step_failure(step, &error, artifacts.clone())?;
        } else {
            run.finish_step_success(
                step,
                serde_json::json!({
                    "suite": suite_name,
                    "tests": suite_tests,
                    "failures": suite_failures,
                    "errors": suite_errors,
                    "skipped": suite_skipped,
                }),
                artifacts.clone(),
            )?;
        }
    }
    let _ = &mut artifacts;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use androidforge_core::redact::Redactor;
    use androidforge_core::run::{RunOptions, RunStatus};

    #[test]
    fn imports_junit_suites() {
        let temp = tempfile::tempdir().unwrap();
        let xml = temp.path().join("tests.xml");
        std::fs::write(
            &xml,
            r#"<?xml version="1.0"?>
<testsuites>
  <testsuite name="unit" tests="2">
    <testcase classname="A" name="passes"/>
    <testcase classname="A" name="fails"><failure message="boom">stack</failure></testcase>
  </testsuite>
</testsuites>
"#,
        )
        .unwrap();
        let mut run = RunContext::create(
            RunOptions::new(
                temp.path().join("artifacts"),
                temp.path(),
                "Example",
                "report.import.junit",
            ),
            Redactor::disabled(),
        )
        .unwrap();
        let summary = import_junit(&mut run, &xml).unwrap();
        assert!(!summary.passed);
        assert_eq!(summary.tests, 2);
        assert_eq!(summary.failures, 1);
        run.finalize(RunStatus::Failed, 7).unwrap();
        assert!(run.report_path().is_file());
    }
}
