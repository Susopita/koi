use std::path::Path;
use std::process::Command;

use crate::frontend::ast::ASTNode;
use crate::frontend::parser::Parser;
use crate::frontend::scanner::Scanner;
use crate::frontend::scope::ScopeAnalyzer;
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

/// Runs the frontend: scan → parse → scope analysis.
pub fn run_frontend(input: &str) -> Result<ASTNode, BuildError> {
    let scanner = Scanner::new(input);
    let mut parser = Parser::new(scanner);

    let program = parser
        .parse_program()
        .map_err(|e| BuildError {
            phase: "parser".into(),
            message: e,
        })?;

    let mut scope_analyzer = ScopeAnalyzer::new();
    scope_analyzer
        .analyze(&program)
        .map_err(|errors| BuildError {
            phase: "scope".into(),
            message: errors.join("; "),
        })?;

    Ok(program)
}

/// Runs the middle-end: type inference → unification → monomorphization →
/// lambda lifting → IR generation.
pub fn run_middle_end(program: &ASTNode) -> Result<IRProgram, BuildError> {
    crate::middle_end::pipeline::compile(program).map_err(|e| BuildError {
        phase: "middle_end".into(),
        message: e,
    })
}

/// Runs the backend: optimization → codegen → peephole → write output.s →
/// compile + link to executable.
pub fn run_backend(program: IRProgram, src_path: &Path) -> Result<(), BuildError> {
    let assembly = crate::backend::compile_ir_to_assembly(&program).map_err(|e| BuildError {
        phase: "backend".into(),
        message: e.message,
    })?;

    // Write output.s (always produced regardless of whether we can
    // subsequently assemble/link it on this host).
    std::fs::write("output.s", &assembly).map_err(|e| BuildError {
        phase: "io".into(),
        message: format!("failed to write output.s: {e}"),
    })?;

    // Attempt to assemble and link.  On non-x86-64 hosts (e.g. Apple
    // Silicon) gcc cannot assemble the x86-64 output, so this step is
    // best-effort: failure is a warning, not a hard error.
    let exe_name = src_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");

    match assemble_and_link("output.s", exe_name) {
        Ok(()) => {}
        Err(msg) => {
            eprintln!("[koi] warning: {msg} (output.s was written, but no binary was produced)");
        }
    }

    Ok(())
}

/// Attempt to assemble output.s with a C compiler and link it into an
/// executable.  Returns `Ok(())` on success or an error message on failure
/// (caller decides whether to treat it as fatal).
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
        return Err("assembly failed (x86-64 cross-assembly may not be available on this host)".into());
    }

    let mut link_cmd = Command::new(&cc);
    if cc.ends_with("zig") || cc == "zig" {
        link_cmd.arg("cc");
    }
    let status = link_cmd
        .arg(&obj)
        .arg("-o")
        .arg(exe_name)
        .status()
        .map_err(|e| format!("failed to run linker ({cc}): {e}"))?;
    if !status.success() {
        return Err("linking failed".into());
    }

    let _ = std::fs::remove_file(&obj);
    Ok(())
}

fn find_c_compiler() -> String {
    for candidate in ["cc", "gcc", "clang"] {
        if Command::new(candidate).arg("--version").output().is_ok() {
            return candidate.to_string();
        }
    }
    "cc".to_string()
}
