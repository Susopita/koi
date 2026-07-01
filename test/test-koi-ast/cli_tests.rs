use std::fs;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

const BIN: &str = env!("CARGO_BIN_EXE_koi-ast");
const AST_JSON: &str = "/tmp/ast.json";

/// `koi-ast` always writes to the fixed path `/tmp/ast.json`, so any test
/// that inspects that file has to run exclusive of every other such test
/// (cargo's default test harness runs tests from one binary concurrently on
/// separate threads).
fn ast_json_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn scratch_file(name: &str, contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "koi-ast-cli-test-{}-{}.carp",
        std::process::id(),
        name
    ));
    fs::write(&path, contents).expect("failed to write scratch .carp file");
    path
}

#[test]
fn no_arguments_prints_usage_and_exits_nonzero() {
    let output = Command::new(BIN).output().expect("failed to run koi-ast");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
}

#[test]
fn too_many_arguments_prints_usage_and_exits_nonzero() {
    let output = Command::new(BIN)
        .arg("a.carp")
        .arg("b.carp")
        .output()
        .expect("failed to run koi-ast");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
}

#[test]
fn missing_input_file_reports_lexer_prefixed_error() {
    let output = Command::new(BIN)
        .arg("/nonexistent/path/does-not-exist.carp")
        .output()
        .expect("failed to run koi-ast");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("[lexer]"));
}

#[test]
fn valid_program_exits_zero_and_writes_schema_valid_json() {
    let _guard = ast_json_lock().lock().unwrap();
    let _ = fs::remove_file(AST_JSON);

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let input = format!("{manifest_dir}/../test_programs/add.carp");

    let output = Command::new(BIN)
        .arg(&input)
        .output()
        .expect("failed to run koi-ast");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("AST complete"));

    let json = fs::read_to_string(AST_JSON).expect("koi-ast should have written /tmp/ast.json");
    let value: serde_json::Value = serde_json::from_str(&json).expect("output must be valid JSON");
    assert_eq!(value["nodeType"], "program");
}

#[test]
fn parse_error_is_prefixed_and_exits_nonzero() {
    let path = scratch_file("parse-error", "(defn add [x y]\n  (+ x y)");
    let output = Command::new(BIN)
        .arg(&path)
        .output()
        .expect("failed to run koi-ast");
    let _ = fs::remove_file(&path);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("[parser]"));
}

#[test]
fn scope_error_is_prefixed_exits_nonzero_and_does_not_write_json() {
    let _guard = ast_json_lock().lock().unwrap();
    let _ = fs::remove_file(AST_JSON);

    let path = scratch_file("scope-error", "(defn add [x y]\n  (+ x z))");
    let output = Command::new(BIN)
        .arg(&path)
        .output()
        .expect("failed to run koi-ast");
    let _ = fs::remove_file(&path);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("[scope]"), "unexpected stderr: {stderr}");
    assert!(
        !std::path::Path::new(AST_JSON).exists(),
        "must not write ast.json on a scope error"
    );
}
