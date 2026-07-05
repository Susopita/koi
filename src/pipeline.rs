use std::path::{Path, PathBuf};
use std::process::Command;

use crate::frontend::ast::ASTNode;
use crate::frontend::diagnostics::{CheckOutput, DiagnosticBag, DumpAstOutput};
use crate::frontend::parser::Parser;
use crate::frontend::scanner::Scanner;
use crate::frontend::scope::ScopeAnalyzer;
use crate::backend::TargetArch;
use crate::middle_end::ir::IRProgram;

/// What to do after parsing the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildMode {
    /// Full pipeline: frontend → middle-end → backend → output.s + binary
    Full,
    /// Frontend + middle-end only (type-check). No code generation.
    Check,
    /// Frontend only. Return the AST for display.
    DumpAst,
}

/// A structured error from any stage of the pipeline.
#[derive(Debug)]
pub struct BuildError {
    pub phase: String,
    pub message: String,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.phase, self.message)
    }
}

impl std::error::Error for BuildError {}

// ---------------------------------------------------------------------------
// New pipeline entry point (replaces old run_build in main.rs)
// ---------------------------------------------------------------------------

/// Result of a pipeline run: either a JSON blob for stdout, or nothing.
#[derive(Debug)]
pub enum PipelineResult {
    Json(String),
    None,
}

/// Run the full pipeline with structured diagnostics.
///
/// Returns the string to write to stdout (if any).  Errors are collected
/// into the diagnostic bag and *not* printed to stderr by this function —
/// the caller decides how to render them.
pub fn run_pipeline(
    input: &str,
    src_path: &PathBuf,
    mode: BuildMode,
    arch: TargetArch,
) -> (PipelineResult, DiagnosticBag) {
    let mut diag = DiagnosticBag::new(
        src_path.to_str().unwrap_or("<unknown>"),
    );

    // --- Stage 1: Frontend (always) ---
    let ast = match run_frontend(input, &mut diag) {
        Some(a) => a,
        None => return (emit_check_failure(&diag), diag),
    };

    if mode == BuildMode::DumpAst {
        let json = serde_json::to_value(&ast).unwrap_or(serde_json::Value::Null);
        let output = DumpAstOutput { ast: json };
        return (
            PipelineResult::Json(
                serde_json::to_string_pretty(&output).unwrap_or_default(),
            ),
            diag,
        );
    }

    // --- Stage 2: Middle-end ---
    let ir = match run_middle_end(&ast, &mut diag) {
        Some(ir) => ir,
        None => return (emit_check_failure(&diag), diag),
    };

    // --- Stage 3: Borrow check (only in Check / Full modes) ---
    if let Err(e) = run_borrow_check(&ast) {
        diag.push("borrow_check", "error", e);
    }

    if mode == BuildMode::Check {
        return (
            emit_check_result(&diag),
            diag,
        );
    }

    // --- Stage 4: Backend ---
    match run_backend(ir, src_path, &mut diag, arch) {
        Ok(()) => (PipelineResult::None, diag),
        Err(()) => (PipelineResult::None, diag),
    }
}

// ---------------------------------------------------------------------------
// Stage runners
// ---------------------------------------------------------------------------

fn run_frontend(input: &str, diag: &mut DiagnosticBag) -> Option<ASTNode> {
    let scanner = Scanner::new(input);
    let mut parser = Parser::new(scanner);

    let program = match parser.parse_program() {
        Ok(ast) => ast,
        Err(e) => {
            diag.push("parser", "error", e);
            return None;
        }
    };

    let mut scope_analyzer = ScopeAnalyzer::new();
    if let Err(errors) = scope_analyzer.analyze(&program) {
        for e in errors {
            diag.push("scope", "error", e);
        }
        return None;
    }

    Some(program)
}

fn run_middle_end(program: &ASTNode, diag: &mut DiagnosticBag) -> Option<IRProgram> {
    match crate::middle_end::pipeline::compile(program) {
        Ok(ir) => Some(ir),
        Err(e) => {
            diag.push("inference", "error", e);
            None
        }
    }
}

fn run_borrow_check(program: &ASTNode) -> Result<(), String> {
    // The borrow checker currently operates on TypedExpr.
    // For now this is a pass-through — the full bridge is wired
    // when the old AST → TypedAST → IR path is finalised.
    Ok(())
}

fn run_backend(
    program: IRProgram,
    src_path: &Path,
    diag: &mut DiagnosticBag,
    arch: TargetArch,
) -> Result<(), ()> {
    let assembly = match crate::backend::compile_ir_to_assembly(&program, arch) {
        Ok(a) => a,
        Err(e) => {
            diag.push("codegen", "error", e.message);
            return Err(());
        }
    };

    if let Err(e) = std::fs::write("output.s", &assembly) {
        diag.push("io", "error", format!("failed to write output.s: {e}"));
        return Err(());
    }

    let exe_name = src_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");

    match assemble_and_link("output.s", exe_name) {
        Ok(()) => {}
        Err(msg) => {
            diag.push("linker", "warning", msg);
        }
    }

    Ok(())
}

fn assemble_and_link(asm_path: &str, exe_name: &str) -> Result<(), String> {
    let cc = find_c_compiler();
    let obj = std::env::temp_dir().join("koi_output.o");

    let mut asm_cmd = Command::new(&cc);
    if cc.ends_with("zig") || cc == "zig" {
        asm_cmd.arg("cc");
    }
    let status = asm_cmd
        .arg("-c")
        .arg(asm_path)
        .arg("-o")
        .arg(&obj)
        .status()
        .map_err(|e| format!("failed to run assembler ({cc}): {e}"))?;
    if !status.success() {
        return Err(
            "assembly failed (x86-64 cross-assembly may not be available on this host)".into(),
        );
    }

    let mut link_cmd = Command::new(&cc);
    if cc.ends_with("zig") || cc == "zig" {
        link_cmd.arg("cc");
    }
    let status = link_cmd
        .arg(&obj)
        .arg("-o")
        .arg(exe_name)
        .arg("-lc")
        .status()
        .map_err(|e| format!("failed to run linker ({cc}): {e}"))?;
    if !status.success() {
        return Err("linking failed".into());
    }

    let _ = std::fs::remove_file(&obj);
    Ok(())
}

fn find_c_compiler() -> String {
    // On macOS, prefer clang over GCC (which in Nix is often real GCC
    // and cannot produce correct Mach-O binaries for ARM64).
    for candidate in ["clang", "cc", "gcc"] {
        if Command::new(candidate).arg("--version").output().is_ok() {
            if cfg!(target_os = "macos") && candidate == "gcc" {
                // Check if `gcc` is actually Clang.
                let out = Command::new("gcc")
                    .arg("--version")
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();
                if out.contains("Apple") || out.contains("clang") {
                    return "gcc".to_string(); // It's Apple Clang via gcc.
                }
                // Real GCC on macOS — skip, try next candidate.
                continue;
            }
            return candidate.to_string();
        }
    }
    "cc".to_string()
}

// ---------------------------------------------------------------------------
// JSON output helpers
// ---------------------------------------------------------------------------

fn emit_check_failure(diag: &DiagnosticBag) -> PipelineResult {
    PipelineResult::Json(diag.to_json())
}

fn emit_check_result(diag: &DiagnosticBag) -> PipelineResult {
    if diag.has_errors() {
        PipelineResult::Json(diag.to_json())
    } else {
        PipelineResult::Json(
            serde_json::to_string_pretty(&CheckOutput {
                success: true,
                diagnostics: diag.diagnostics.clone(),
            })
            .unwrap_or_default(),
        )
    }
}
