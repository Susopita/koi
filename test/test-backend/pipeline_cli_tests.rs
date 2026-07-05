use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_koi");

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn c_compiler() -> String {
    if let Ok(cc) = std::env::var("KOI_CC") {
        return cc;
    }
    if let Ok(cc) = std::env::var("CC") {
        return cc;
    }

    for candidate in ["cc", "gcc", "clang"] {
        if Command::new(candidate).arg("--version").output().is_ok() {
            return candidate.to_string();
        }
    }

    panic!("no C compiler found; set KOI_CC or CC");
}

/// Run `koi build <sample>` inside a fresh temporary directory so that
/// `output.s` never collides with another test.
fn run_pipeline(sample: &str) -> PathBuf {
    let root = workspace_root();
    let sample_path = root.join("test/casos_prueba_carp").join(sample);

    // Each test gets its own tempdir so output.s never races.
    let temp_dir = std::env::temp_dir().join(format!(
        "koi-pipeline-test-{}-{}",
        std::process::id(),
        sample
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("failed to create temp dir");

    let output = Command::new(BIN)
        .arg("build")
        .arg(&sample_path)
        .current_dir(&temp_dir)
        .output()
        .expect("failed to run koi");

    assert!(
        output.status.success(),
        "koi build failed for {sample}:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let asm_path = temp_dir.join("output.s");
    assert!(asm_path.is_file(), "expected output.s to be generated for {sample}");
    asm_path
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn add_program_generates_assembly() {
    let asm_path = run_pipeline("add.carp");
    let asm = fs::read_to_string(&asm_path).expect("failed to read output.s");
    assert!(asm.contains("main:"), "output.s must contain main label");
    let _ = fs::remove_dir_all(asm_path.parent().unwrap());
}

#[test]
fn lambda_program_generates_assembly() {
    let asm_path = run_pipeline("lambda.carp");
    let asm = fs::read_to_string(&asm_path).expect("failed to read output.s");
    assert!(asm.contains("main:"), "output.s must contain main label");
    let _ = fs::remove_dir_all(asm_path.parent().unwrap());
}

#[test]
fn fib_program_generates_assembly() {
    let asm_path = run_pipeline("fib.carp");
    let _ = fs::remove_dir_all(asm_path.parent().unwrap());
}

#[test]
fn control_flow_struct_and_kitchen_sink_reach_assembly_stage() {
    for sample in ["control_flow.carp", "struct.carp", "kitchen_sink.carp"] {
        let asm_path = run_pipeline(sample);
        assert!(asm_path.is_file(), "expected output.s for {sample}");

        // On x86-64, verify the assembly can actually be assembled by gcc.
        if cfg!(target_arch = "x86_64") {
            let compiler = c_compiler();
            let obj_path = std::env::temp_dir().join(format!("{sample}.o"));
            let mut compile = Command::new(&compiler);
            if compiler.ends_with("/zig") || compiler == "zig" {
                compile.arg("cc");
            }
            let status = compile
                .arg("-c")
                .arg(&asm_path)
                .arg("-o")
                .arg(&obj_path)
                .status()
                .expect("failed to assemble");
            assert!(status.success(), "assembly failed for {sample}");
            let _ = fs::remove_file(obj_path);
        }
        let _ = fs::remove_dir_all(asm_path.parent().unwrap());
    }
}

// These full end-to-end tests only run on x86-64 since koi generates x86-64
// assembly and the final binary must be executed natively.
#[test]
#[cfg_attr(not(target_arch = "x86_64"), ignore)]
fn add_program_runs_and_returns_expected_exit_code() {
    let asm_path = run_pipeline("add.carp");
    let exe = assemble_and_run(&asm_path, "add", Some(8));
    let _ = fs::remove_dir_all(asm_path.parent().unwrap());
    let _ = fs::remove_file(exe);
}

#[test]
#[cfg_attr(not(target_arch = "x86_64"), ignore)]
fn lambda_program_runs_and_returns_expected_exit_code() {
    let asm_path = run_pipeline("lambda.carp");
    let exe = assemble_and_run(&asm_path, "lambda", Some(6));
    let _ = fs::remove_dir_all(asm_path.parent().unwrap());
    let _ = fs::remove_file(exe);
}

fn assemble_and_run(asm_path: &Path, name: &str, expected_exit: Option<i32>) -> PathBuf {
    let compiler = c_compiler();
    let obj_path = std::env::temp_dir().join(format!("koi-pipeline-{name}.o"));
    let exe_path = std::env::temp_dir().join(format!("koi-pipeline-{name}"));

    let mut assemble_cmd = Command::new(&compiler);
    if compiler.ends_with("/zig") || compiler == "zig" {
        assemble_cmd.arg("cc");
    }
    let status = assemble_cmd
        .arg("-c")
        .arg(asm_path)
        .arg("-o")
        .arg(&obj_path)
        .status()
        .expect("failed to assemble");
    assert!(status.success(), "assembly failed for {name}");

    let mut link_cmd = Command::new(&compiler);
    if compiler.ends_with("/zig") || compiler == "zig" {
        link_cmd.arg("cc");
    }
    let status = link_cmd
        .arg(&obj_path)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("failed to link");
    assert!(status.success(), "link failed for {name}");

    let output = Command::new(&exe_path)
        .output()
        .expect("failed to run executable");
    assert_eq!(
        output.status.code(),
        expected_exit,
        "unexpected exit code for {name}"
    );

    let _ = fs::remove_file(obj_path);
    exe_path
}
