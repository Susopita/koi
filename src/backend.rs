pub mod abi;
pub mod codegen;
pub mod optimizer;
pub mod peephole;
pub mod register_allocator;

use serde::Serialize;

use crate::middle_end::ir::IRProgram;
use codegen::X86Generator;
use optimizer::Optimizer;
use peephole::Peephole;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ErrorLocation {
    pub file: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompileError {
    pub phase: String,
    pub severity: String,
    pub message: String,
    pub location: ErrorLocation,
}

impl CompileError {
    pub fn new(phase: &str, message: impl Into<String>) -> Self {
        CompileError {
            phase: phase.to_string(),
            severity: "error".to_string(),
            message: message.into(),
            location: ErrorLocation {
                file: String::new(),
                line: 0,
                column: 0,
            },
        }
    }

    pub fn io(path: &str, error: impl std::fmt::Display) -> Self {
        CompileError {
            phase: "io".to_string(),
            severity: "error".to_string(),
            message: format!("{path}: {error}"),
            location: ErrorLocation {
                file: path.to_string(),
                line: 0,
                column: 0,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Compile entry points
// ---------------------------------------------------------------------------

/// Compile an in-memory [`IRProgram`] into assembly text, applying
/// optimizations and peephole passes along the way.
pub fn compile_ir_to_assembly(program: &IRProgram) -> Result<String, CompileError> {
    let mut program = program.clone();
    Optimizer::optimize_program(&mut program);
    let asm = X86Generator::new()
        .generate(&program)
        .map_err(|e| CompileError::new("codegen", e))?;
    Ok(Peephole::optimize(&asm))
}

/// Convenience wrapper — parse IR from a JSON string, then compile.
/// Kept for backward compatibility with existing tests.
pub fn compile_ir_json_to_assembly(ir_json: &str) -> Result<String, CompileError> {
    let program: IRProgram =
        serde_json::from_str(ir_json).map_err(|e| CompileError::new("ir_parser", e.to_string()))?;
    if program.functions.is_empty() {
        return Err(CompileError::new(
            "ir_parser",
            "IR program does not contain any functions",
        ));
    }
    compile_ir_to_assembly(&program)
}

/// Read IR from a file and write the resulting assembly to another file.
pub fn compile_ir_file_to_output(input: &str, output: &str) -> Result<(), CompileError> {
    let ir_json =
        std::fs::read_to_string(input).map_err(|e| CompileError::io(input, e))?;
    let assembly = compile_ir_json_to_assembly(&ir_json)?;
    std::fs::write(output, assembly).map_err(|e| CompileError::io(output, e))
}
