use std::fs;
use std::path::{Path, PathBuf};

use androidforge_diagnostics::{
    FramesDeltaStatus, FramesThresholds, compare_frames_reports, compare_frames_reports_default,
    frames_report_from_gfxinfo, parse_frames_gfxinfo,
};
use serde_json::Value;

fn schema_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("schema")
}

fn validate_frames(value: &Value) {
    let bytes = fs::read(schema_dir().join("frames.schema.json")).expect("frames schema");
    let schema: Value = serde_json::from_slice(&bytes).expect("parse frames schema");
    let validator = jsonschema::validator_for(&schema).expect("compile frames schema");
    validator
        .validate(value)
        .unwrap_or_else(|error| panic!("frames schema rejected {value:#}: {error}"));
}

fn typical_gfxinfo() -> &'static str {
    "Applications Graphics Acceleration Info:\n\
     Uptime: 123456 Realtime: 123456\n\
     ** Graphics info for pid 1234 [com.example] **\n\
     Stats since: 3411175000000ns\n\
     Total frames rendered: 120\n\
     Janky frames: 9 (7.50%)\n\
     Janky frames legacy: 5 (4.17%)\n\
     50th percentile: 8ms\n\
     90th percentile: 16ms\n\
     95th percentile: 24ms\n\
     99th percentile: 40ms\n\
     Max: 72ms\n\
     Number Missed Vsync: 3\n\
     Number Slow UI thread: 2\n\
     Number Frame deadline missed: 1\n\
     Histogram:\n\
       0-16ms: 100 frames (83.33%)\n"
}

#[test]
fn frames_report_payload_validates_against_schema() {
    let mut report = frames_report_from_gfxinfo(
        typical_gfxinfo(),
        "com.example",
        "emulator-5554",
        "2026-01-01T00:00:00.000Z",
        30,
    );
    let baseline = report.clone();
    report.comparison = Some(compare_frames_reports_default(&baseline, &report));
    report.baseline_source = Some("runs/old/frames.json".to_string());
    let value = serde_json::to_value(&report).expect("serialise frames report");
    validate_frames(&value);
}

#[test]
fn explicit_thresholds_classify_regressions_and_the_payload_validates() {
    let baseline = frames_report_from_gfxinfo(
        typical_gfxinfo(),
        "com.example",
        "emulator-5554",
        "2026-01-01T00:00:00.000Z",
        30,
    );
    let slower = typical_gfxinfo()
        .replace("Janky frames: 9 (7.50%)", "Janky frames: 18 (15.00%)")
        .replace("99th percentile: 40ms", "99th percentile: 48ms")
        .replace("Max: 72ms", "Max: 80ms");
    let current = frames_report_from_gfxinfo(
        &slower,
        "com.example",
        "emulator-5554",
        "2026-01-02T00:00:00.000Z",
        30,
    );

    let strict = compare_frames_reports_default(&baseline, &current);
    assert!(strict.comparable);
    assert!(strict.regression);
    assert_eq!(strict.deltas.len(), 3);
    assert!(
        strict
            .deltas
            .iter()
            .any(|delta| delta.status == FramesDeltaStatus::Regression)
    );

    let tolerant = compare_frames_reports(
        &baseline,
        &current,
        FramesThresholds {
            jank_percent_pp: 8.0,
            p99_ms: 10.0,
            max_ms: 10.0,
        },
    );
    assert!(tolerant.comparable);
    assert!(!tolerant.regression, "{tolerant:?}");

    let mut report = current;
    report.baseline_source = Some("runs/old/frames/report.json".to_string());
    report.comparison = Some(strict);
    validate_frames(&serde_json::to_value(&report).expect("serialise frames report"));
}

#[test]
fn missing_fields_payload_validates_against_schema() {
    let sample = parse_frames_gfxinfo("No process found for com.example\n");
    assert_eq!(sample.sample_status, "unavailable");
    let report = frames_report_from_gfxinfo(
        "No process found for com.example\n",
        "com.example",
        "emulator-5554",
        "2026-01-01T00:00:00.000Z",
        30,
    );
    let value = serde_json::to_value(&report).expect("serialise frames report");
    validate_frames(&value);
    assert_eq!(value["sample"]["janky"], Value::Null);
}
