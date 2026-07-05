pub mod abi;
pub mod peephole;
pub mod instruction_select;
pub mod materializer;
pub mod register_allocator;
pub mod scheduler;

use crate::backend::TargetBackend;
use crate::backend::CompileError;
use crate::middle_end::ir::IRProgram;
use crate::backend::optimizer::Optimizer;
use self::instruction_select::{select_instructions, emit_assembly};
use self::materializer::materialize_constants;
use self::register_allocator::allocate_registers;

/// ARM64 code generation backend.
pub struct Backend;

impl Backend {
    pub fn new() -> Self {
        Backend
    }
}

impl TargetBackend for Backend {
    fn name(&self) -> &'static str {
        "arm64"
    }

    fn generate_code(&self, program: &IRProgram) -> Result<String, CompileError> {
        let mut program = program.clone();
        Optimizer::optimize_program(&mut program);

        // Phase 1: Instruction selection (Maximal Munch + if-conversion).
        let mut selected = self::instruction_select::select_instructions(&program);

        // Phase 2: Greedy constant materialization.
        self::materializer::materialize_constants(&mut selected);

        // Phase 3: Register allocation (graph coloring + coalescing).
        self::register_allocator::allocate_registers(&mut selected);

        // Phase 4: List scheduling (reorder ops to hide latencies).
        self::scheduler::schedule_functions(&mut selected);

        // Phase 5: Emit assembly text.
        let asm = self::instruction_select::emit_assembly(&selected);
        Ok(self::peephole::optimize(&asm))
    }
}
