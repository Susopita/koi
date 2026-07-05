pub mod abi;
pub mod codegen;
pub mod register_allocator;
pub mod peephole;

use crate::backend::TargetBackend;
use crate::backend::CompileError;
use crate::middle_end::ir::IRProgram;
use codegen::X86Generator;
use crate::backend::optimizer::Optimizer;
use peephole::Peephole;

/// x86_64 code generation backend.
pub struct Backend;

impl Backend {
    pub fn new() -> Self {
        Backend
    }
}

impl TargetBackend for Backend {
    fn name(&self) -> &'static str {
        "x86_64"
    }

    fn generate_code(&self, program: &IRProgram) -> Result<String, CompileError> {
        let mut program = program.clone();
        Optimizer::optimize_program(&mut program);
        let asm = X86Generator::new()
            .generate(&program)
            .map_err(|e| CompileError::new("codegen", e))?;
        Ok(Peephole::optimize(&asm))
    }
}
