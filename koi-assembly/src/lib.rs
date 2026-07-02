pub mod abi;
pub mod codegen;
pub mod ir_parser;
pub mod optimizer;
pub mod register_allocator;

use codegen::X86Generator;
use ir_parser::IRParser;
use optimizer::Optimizer;
use serde::Serialize;

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
                file: "/tmp/ir.json".to_string(),
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

pub fn compile_ir_json_to_assembly(ir_json: &str) -> Result<String, CompileError> {
    let mut program = IRParser::parse_json(ir_json).map_err(|e| CompileError::new("ir_parser", e))?;
    Optimizer::optimize_program(&mut program);
    X86Generator::new().generate(&program).map_err(|e| CompileError::new("codegen", e))
}

pub fn compile_ir_file_to_output(input: &str, output: &str) -> Result<(), CompileError> {
    let ir_json = std::fs::read_to_string(input).map_err(|e| CompileError::io(input, e))?;
    let assembly = compile_ir_json_to_assembly(&ir_json)?;
    std::fs::write(output, assembly).map_err(|e| CompileError::io(output, e))
}

