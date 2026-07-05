pub mod abi;
pub mod peephole;
pub mod instruction_select;
pub mod optimizer;
pub mod regalloc;

use crate::backend::TargetBackend;
use crate::backend::CompileError;
use crate::middle_end::ir::IRProgram;
use crate::backend::optimizer::Optimizer;
use self::instruction_select::{select_instructions, emit_assembly};
use self::optimizer::optimise_selected;
use self::regalloc::allocate_and_frame;

/// RISC-V code generation backend.
pub struct Backend;

impl Backend {
    pub fn new() -> Self {
        Backend
    }
}

impl TargetBackend for Backend {
    fn name(&self) -> &'static str {
        "riscv"
    }

    fn generate_code(&self, program: &IRProgram) -> Result<String, CompileError> {
        let mut program = program.clone();
        Optimizer::optimize_program(&mut program);

        // Phase 1: Instruction selection (Maximal Munch + constant folding).
        let mut selected = self::instruction_select::select_instructions(&program);

        // Phase 2: Post-selection optimisations (strength reduction, memory LVN).
        self::optimizer::optimise_selected(&mut selected);

        // Phase 3: Register allocation + prologue/epilogue.
        self::regalloc::allocate_and_frame(&mut selected);

        // Phase 4: Emit assembly text.
        let asm = self::instruction_select::emit_assembly(&selected);
        Ok(self::peephole::optimize(&asm))
    }
}
