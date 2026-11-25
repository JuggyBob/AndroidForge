//! Public-API smoke tests for the accessibility audit primitives.
//!
//! These use a synthetic hierarchy string and in-memory pixels only: no
//! device, emulator or adb access is involved.

use androidforge_ui::a11y::{
    A11yMeasurement, A11yOptions, A11yReport, A11yRule, Severity, audit_hierarchy,
    audit_region_contrast, check_contrast, contrast_finding,
};
use androidforge_ui::{Image, Rgb, UiHierarchy, UiNode};

const HIERARCHY: &str = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" text="" class="android.widget.FrameLayout" package="com.example" clickable="false" displayed="true" bounds="[0,0][1080,2400]">
    <node index="0" text="OK" class="android.widget.Button" package="com.example" clickable="true" displayed="true" bounds="[100,200][148,244]" />
    <node index="1" text="" class="android.widget.ImageButton" package="com.example" clickable="true" displayed="true" bounds="[200,200][232,232]" />
  </node>
</hierarchy>"#;

fn sample_image() -> Image {
    let pixels = [
        255u8, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
    ];
    Image::from_rgba(2, 2, pixels.to_vec()).unwrap()
}

#[test]
fn xml_hierarchy_audit_finds_missing_labels_and_small_targets() {
    let hierarchy = UiHierarchy::parse(HIERARCHY).unwrap();
    let report = audit_hierarchy(&hierarchy, A11yOptions::default());

    assert_eq!(report.nodes_examined, 3);
    // The button has visible text "OK" (a valid accessible name); only the
    // unlabelled ImageButton is reported.
    assert_eq!(report.count(A11yRule::MissingContentDescription), 1);
    assert_eq!(report.count(A11yRule::TinyTouchTarget), 2);
    assert_eq!(report.count(A11yRule::DuplicateLabel), 0);
    assert!(report.count_by_severity(Severity::Error) >= 1);
    assert!(
        report
            .findings
            .iter()
            .all(|finding| !finding.message.is_empty())
    );
    assert!(
        report
            .findings
            .iter()
            .all(|finding| !finding.node_path.is_empty())
    );
}

#[test]
fn report_round_trips_through_serde_json() {
    let hierarchy = UiHierarchy::parse(HIERARCHY).unwrap();
    let report = audit_hierarchy(&hierarchy, A11yOptions::default());
    let json = serde_json::to_string(&report).unwrap();
    let decoded: A11yReport = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, report);

    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(value["findings"].is_array());
    assert_eq!(value["nodes_examined"], 3);
    assert_eq!(value["options"]["min_contrast_ratio"], 4.5);
}

#[test]
fn contrast_helpers_work_with_bounds_derived_regions() {
    use androidforge_ui::a11y::region_from_bounds;

    let image = sample_image();
    let region = region_from_bounds(
        androidforge_ui::Rect {
            left: 0,
            top: 0,
            right: 2,
            bottom: 2,
        },
        &image,
    )
    .unwrap();
    let threshold = A11yOptions::default().contrast_threshold(false);
    let check = audit_region_contrast(&image, region, Rgb::new(120, 120, 120), threshold).unwrap();
    assert_eq!(check.background_hex, "#FFFFFF");
    assert_eq!(check.threshold, 4.5);
    assert!(!check.passed);

    let hierarchy = UiHierarchy::parse(HIERARCHY).unwrap();
    let node: &UiNode = &hierarchy.nodes[1];
    let finding = contrast_finding(node, &check, false).unwrap();
    assert_eq!(finding.rule, A11yRule::LowContrast);
    assert_eq!(finding.node_path, node.path);
    assert!(matches!(
        finding.measured,
        A11yMeasurement::LowContrast { .. }
    ));
}

#[test]
fn check_contrast_reports_wcag_reference_values() {
    let check = check_contrast(Rgb::new(0, 0, 0), Rgb::new(255, 255, 255), 4.5);
    assert!((check.ratio - 21.0).abs() < 1e-9);
    assert!(check.passed);
    let identical = check_contrast(Rgb::new(40, 60, 80), Rgb::new(40, 60, 80), 4.5);
    assert!((identical.ratio - 1.0).abs() < 1e-12);
    assert!(!identical.passed);
}
