//! Architecture-specific backends.
//!
//! Each subdirectory (`x86_64`, `arm64`, `riscv`) implements a
//! [`TargetBackend`] that translates the architecture-agnostic [`IRProgram`]
//! into final assembly text.
//!
//! The [`compile_ir_to_assembly`] function dispatches to the right backend
//! based on the [`TargetArch`] argument.

use std::fmt;

use crate::middle_end::ir::IRProgram;

// ---------------------------------------------------------------------------
// Public sub-modules (architecture-agnostic)
// ---------------------------------------------------------------------------

pub mod optimizer; // IR-level optimiser (shared by all targets)

// ---------------------------------------------------------------------------
// Target architecture enum
// ---------------------------------------------------------------------------

/// Supported target architectures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetArch {
    X8664,
    Arm64,
    RiscV,
}

impl TargetArch {
    /// Parse from a CLI string.
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "x86_64" | "x86-64" | "amd64" => Ok(TargetArch::X8664),
            "arm64" | "aarch64" => Ok(TargetArch::Arm64),
            "riscv" | "riscv64" => Ok(TargetArch::RiscV),
            other => Err(format!(
                "unsupported target '{other}'; expected x86_64, arm64, or riscv"
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TargetArch::X8664 => "x86_64",
            TargetArch::Arm64 => "arm64",
            TargetArch::RiscV => "riscv",
        }
    }
}

impl Default for TargetArch {
    fn default() -> Self {
        TargetArch::X8664
    }
}

impl fmt::Display for TargetArch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Target backend trait
// ---------------------------------------------------------------------------

/// Every target backend must implement this trait.
pub trait TargetBackend {
    /// The human-readable name (e.g. `"x86_64"`).
    fn name(&self) -> &'static str;

    /// Translate an IR program into assembly text.
    fn generate_code(&self, program: &IRProgram) -> Result<String, CompileError>;
}

// ---------------------------------------------------------------------------
// Backend modules (one per architecture)
// ---------------------------------------------------------------------------

pub mod x86_64;
pub mod arm64;
pub mod riscv;

use std::sync::OnceLock;

/// Retrieve the singleton backend for a given architecture.
pub fn backend_for(arch: TargetArch) -> &'static dyn TargetBackend {
    match arch {
        TargetArch::X8664 => {
            static X86: OnceLock<x86_64::Backend> = OnceLock::new();
            X86.get_or_init(|| x86_64::Backend::new())
        }
        TargetArch::Arm64 => {
            static ARM: OnceLock<arm64::Backend> = OnceLock::new();
            ARM.get_or_init(|| arm64::Backend::new())
        }
        TargetArch::RiscV => {
            static RV: OnceLock<riscv::Backend> = OnceLock::new();
            RV.get_or_init(|| riscv::Backend::new())
        }
    }
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CompileError {
    pub phase: String,
    pub severity: String,
    pub message: String,
}

impl CompileError {
    pub fn new(phase: &str, message: impl Into<String>) -> Self {
        CompileError {
            phase: phase.to_string(),
            severity: "error".to_string(),
            message: message.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Main entry points
// ---------------------------------------------------------------------------

/// Compile an IR program into assembly for the given target architecture.
pub fn compile_ir_to_assembly(
    program: &IRProgram,
    arch: TargetArch,
) -> Result<String, CompileError> {
    backend_for(arch).generate_code(program)
}

/// Compile from a JSON IR string into assembly for the given target.
pub fn compile_ir_json_to_assembly(ir_json: &str, arch: TargetArch) -> Result<String, CompileError> {
    let program: IRProgram =
        serde_json::from_str(ir_json).map_err(|e| CompileError::new("ir_parser", e.to_string()))?;
    if program.functions.is_empty() {
        return Err(CompileError::new(
            "ir_parser",
            "IR program does not contain any functions",
        ));
    }
    compile_ir_to_assembly(&program, arch)
}
