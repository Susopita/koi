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
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[scope]") || stderr.contains("[middle_end]"),
        "expected error, got: {stderr}"
    );
}
