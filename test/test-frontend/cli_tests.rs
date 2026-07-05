use std::fs;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_koi");

fn build_cmd() -> Command {
    let mut cmd = Command::new(BIN);
    cmd.arg("build");
    cmd
}

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
fn no_subcommand_prints_usage_and_exits_nonzero() {
    let output = Command::new(BIN)
        .output()
        .expect("failed to run koi");
    assert!(!output.status.success());
    // clap prints help to stderr (error condition)
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Usage:") || stderr.contains("usage"),
        "expected usage message in stderr, got: {stderr}"
    );
}

#[test]
fn missing_input_file_reports_io_error() {
    let output = build_cmd()
        .arg("/nonexistent/path/does-not-exist.carp")
        .output()
        .expect("failed to run koi");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("[io]"), "expected [io] prefix, got: {stderr}");
}

#[test]
fn valid_program_with_dump_ast_exits_zero_and_prints_valid_json() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let input = format!("{manifest_dir}/test/casos_prueba_carp/add.carp");

    let output = build_cmd()
        .arg("--dump-ast")
        .arg(&input)
        .output()
        .expect("failed to run koi");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("dump-ast output must be valid JSON");
    assert_eq!(value["nodeType"], "program");
}

#[test]
fn parse_error_is_prefixed_and_exits_nonzero() {
    let path = scratch_file("parse-error", "(defn add [x y]\n  (+ x y)");
    let output = build_cmd()
        .arg(&path)
        .output()
        .expect("failed to run koi");
    let _ = fs::remove_file(&path);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("[parser]"));
}

#[test]
fn scope_error_is_prefixed_and_exits_nonzero() {
    let path = scratch_file("scope-error", "(defn add [x y]\n  (+ x z))");
    let output = build_cmd()
        .arg(&path)
        .output()
        .expect("failed to run koi");
    let _ = fs::remove_file(&path);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("[scope]"), "unexpected stderr: {stderr}");
}
