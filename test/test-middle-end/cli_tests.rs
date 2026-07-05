use std::fs;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_koi");

fn scratch_file(name: &str, contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "koi-cli-test-{}-{}.carp",
        std::process::id(),
        name
    ));
    fs::write(&path, contents).expect("failed to write scratch .carp file");
    path
}

#[test]
fn valid_build_check_exits_zero() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let input = format!("{manifest_dir}/test/casos_prueba_carp/add.carp");

    let output = Command::new(BIN)
        .arg("build")
        .arg("--check")
        .arg(&input)
        .output()
        .expect("failed to run koi");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // In --check mode, output is JSON to stdout.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("check output must be valid JSON");
    assert_eq!(value["success"], true, "check should succeed: {value}");
}

#[test]
fn check_with_undeclared_variable_reports_error_and_exits_nonzero() {
    let path = scratch_file("undeclared", "(defn f [] z)");
    let output = Command::new(BIN)
        .arg("build")
        .arg("--check")
        .arg(&path)
        .output()
        .expect("failed to run koi");
    let _ = fs::remove_file(&path);

    assert!(!output.status.success());
    // In --check mode, diagnostics are JSON on stdout.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("output must be valid JSON in --check mode");
    assert_eq!(value["success"], false, "expected failure, got {value}");
    let diags = value["diagnostics"].as_array().expect("diagnostics must be an array");
    assert!(!diags.is_empty(), "expected at least one diagnostic");
    let phases: Vec<&str> = diags
        .iter()
        .filter_map(|d| d["phase"].as_str())
        .collect();
    assert!(
        phases.iter().any(|p| *p == "scope" || *p == "inference"),
        "expected scope or inference phase, got {phases:?}"
    );
}
