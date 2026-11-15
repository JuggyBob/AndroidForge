use std::fs;
use std::path::Path;

const FORBIDDEN: &[&str] = &[
    "IronPulse",
    "ironpulse",
    "DiscountHunter",
    "discounthunter",
    "CalcSuite",
    "calcsuite",
    "com.aistudio.fitness.ironpulse",
    "com.discounthunter.android",
    "com.calcsuite",
];

#[test]
fn core_crates_contain_no_project_specific_branches_or_names() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let crates = workspace.join("crates");
    let mut violations = Vec::new();
    for entry in fs::read_dir(&crates).unwrap().flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let src = entry.path().join("src");
        if !src.is_dir() {
            continue;
        }
        scan(&src, &mut violations);
    }
    assert!(
        violations.is_empty(),
        "project-specific terms leaked into core crates:\n{}",
        violations.join("\n")
    );
}

fn scan(directory: &Path, violations: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan(&path, violations);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for token in FORBIDDEN {
                if text.contains(token) {
                    violations.push(format!(
                        "{} contains forbidden token {token}",
                        path.display()
                    ));
                }
            }
        }
    }
}
