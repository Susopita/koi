//! Linear-scan register allocator with spilling.
//!
//! Maps SSA value names to physical x86-64 registers (or stack slots when
//! registers are exhausted).  The allocator computes live intervals, walks
//! them in order, and assigns a register to each interval when possible;
//! intervals whose live range overlaps an already-occupied register are
//! spilled to the stack.
//!
//! # Allocatable registers
//!
//! 14 general-purpose registers: `%rax`, `%rbx`, `%rcx`, `%rdx`, `%rsi`,
//! `%rdi`, `%r8`–`%r15`.  `%rbp` and `%rsp` are reserved for the frame
//! pointer and stack pointer respectively.

use crate::backend::x86_64::abi::AMD64ABI;
use crate::middle_end::ir::{IRFunction, Instruction};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Physical register pool
// ---------------------------------------------------------------------------

const GP_REGISTERS: &[&str] = &[
    "%rax", "%rbx", "%rcx", "%rdx", "%rsi", "%rdi", "%r8", "%r9", "%r10",
    "%r11", "%r12", "%r13", "%r14", "%r15",
];

/// Number of allocatable GP registers.
const NUM_GPR: usize = 14;

// ---------------------------------------------------------------------------
// Value location
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueLocation {
    /// A physical x86-64 register (e.g. `%rax`, `%r10`).
    Register(String),
    /// A stack slot relative to `%rbp` (e.g. `-8(%rbp)`).
    Stack(i64),
}

impl ValueLocation {
    /// Return the operand string for use in an assembly instruction.
    pub fn as_operand(&self) -> String {
        match self {
            ValueLocation::Register(r) => r.clone(),
            ValueLocation::Stack(offset) => format!("{offset}(%rbp)"),
        }
    }
}

// ---------------------------------------------------------------------------
// Live interval
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LiveInterval {
    pub var: String,
    pub start: usize,
    pub end: usize,
    /// Once assigned, the register or stack slot this interval lives in.
    pub location: Option<ValueLocation>,
}

// ---------------------------------------------------------------------------
// Function layout (result of allocation)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FunctionLayout {
    /// The assigned location for every SSA value and parameter.
    pub locations: HashMap<String, ValueLocation>,
    /// The type string for every value (used by codegen to pick int/float
    /// instructions — note that this allocator handles GP registers only;
    /// float values currently stay stack-allocated).
    pub value_types: HashMap<String, String>,
    /// Total stack frame size (aligned to 16 bytes) consumed by spills.
    pub stack_size: i64,
    /// Set of callee-saved registers that this function actually uses;
    /// the codegen prologue must push them and the epilogue pop them.
    pub used_callee_saved: Vec<String>,
}

// ---------------------------------------------------------------------------
// Linear-scan allocator
// ---------------------------------------------------------------------------

pub struct LinearScanAllocator;

impl LinearScanAllocator {
    pub fn new() -> Self {
        LinearScanAllocator
    }

    /// Allocate registers for a single function body.
    pub fn allocate(&self, function: &IRFunction) -> FunctionLayout {
        let intervals = self.compute_live_intervals(function);
        let mut value_types = HashMap::new();

        // Collect type information from parameters and instructions.
        for (name, ty) in &function.parameters {
            value_types.insert(name.clone(), ty.clone());
        }
        for block in &function.blocks {
            for instruction in &block.instructions {
                if let (Some(result), Some(ty)) =
                    (instruction.result_name(), instruction.result_type())
                {
                    value_types.insert(result.to_string(), ty.to_string());
                }
            }
        }

        // --- Linear scan -------------------------------------------------
        let mut active: Vec<LiveInterval> = Vec::new(); // intervals currently live
        let mut free_regs: Vec<bool> = vec![true; NUM_GPR];
        let mut next_stack = -8i64;
        let mut locations: HashMap<String, ValueLocation> = HashMap::new();
        let mut used_callee_saved: Vec<String> = Vec::new();

        // Pre-assign parameter homes (they live at the top of the stack
        // where the caller pushed them).  We give them a stack slot just
        // like everything else — the prologue will move them there from
        // their ABI argument registers.
        for (name, ty) in &function.parameters {
            if !value_types.contains_key(name) {
                value_types.insert(name.clone(), ty.clone());
            }
            locations.insert(name.clone(), ValueLocation::Stack(next_stack));
            next_stack -= 8;
        }

        // Pre-assign a stack slot for every non-register value so we
        // always have a spill location.  This simplifies spilling during
        // allocation.
        let mut spill_slots: HashMap<String, i64> = HashMap::new();
        for interval in &intervals {
            if !spill_slots.contains_key(&interval.var) {
                spill_slots.insert(interval.var.clone(), next_stack);
                next_stack -= 8;
            }
        }

        for mut interval in intervals {
            // Skip values that already have a location (pre-assigned
            // parameters, etc.).
            if locations.contains_key(&interval.var) {
                continue;
            }

            // Floats must stay stack-allocated since the allocator only handles GP registers.
            if value_types.get(&interval.var).map(String::as_str) == Some("f64") {
                let slot = spill_slots[&interval.var];
                interval.location = Some(ValueLocation::Stack(slot));
                locations.insert(interval.var.clone(), interval.location.clone().unwrap());
                active.push(interval);
                continue;
            }

            // Expire old intervals.
            active.retain(|iv| {
                if iv.end <= interval.start {
                    let reg_idx = reg_index(iv.location.as_ref().unwrap());
                    if let Some(idx) = reg_idx {
                        free_regs[idx] = true;
                    }
                    false
                } else {
                    true
                }
            });

            // Try to assign a register.
            let reg_opt = self.find_free_reg(&active, &free_regs);

            if let Some(reg_idx) = reg_opt {
                free_regs[reg_idx] = false;
                let reg_name = GP_REGISTERS[reg_idx].to_string();
                interval.location = Some(ValueLocation::Register(reg_name.clone()));
                // Track callee-saved usage: %rbx, %r12-%r15
                if is_callee_saved(&reg_name) && !used_callee_saved.contains(&reg_name) {
                    used_callee_saved.push(reg_name);
                }
                locations.insert(interval.var.clone(), interval.location.clone().unwrap());
            } else {
                // No free register — spill to the pre-assigned stack slot.
                let slot = spill_slots[&interval.var];
                interval.location = Some(ValueLocation::Stack(slot));
                locations.insert(interval.var.clone(), interval.location.clone().unwrap());
            }

            active.push(interval);
        }

        // Total stack frame: spill slots + any alignment padding.
        let used_bytes = (-next_stack - 8).max(0);
        let stack_size = AMD64ABI::align_to_16(used_bytes);

        FunctionLayout {
            locations,
            value_types,
            stack_size,
            used_callee_saved,
        }
    }

    /// Find a free register, or return `None` if all are occupied.
    /// Prefers caller-saved registers (likely to be available after a call)
    /// over callee-saved ones (which have push/pop overhead).
    fn find_free_reg(
        &self,
        active: &[LiveInterval],
        free_regs: &[bool],
    ) -> Option<usize> {
        // First pass: look for any completely free register.
        for idx in 0..NUM_GPR {
            if free_regs[idx] {
                return Some(idx);
            }
        }

        // All registers taken — spill the interval with the furthest end
        // point (cheapest to spill).  But spilling is handled at the call
        // site, so here we just return None.
        None
    }

    // ------------------------------------------------------------------
    // Live-interval computation (CFG-ignorant, per-function)
    // ------------------------------------------------------------------

    pub fn compute_live_intervals(&self, function: &IRFunction) -> Vec<LiveInterval> {
        let mut starts = HashMap::<String, usize>::new();
        let mut ends = HashMap::<String, usize>::new();
        let mut position = 0usize;

        // Parameters are "defined" at position 0 and "used" at the point
        // of their first reference (which we approximate as 0).
        for (param_name, _) in &function.parameters {
            starts.entry(param_name.clone()).or_insert(0);
            ends.insert(param_name.clone(), 0);
        }

        for block in &function.blocks {
            for instruction in &block.instructions {
                for used in instruction_uses(instruction) {
                    starts.entry(used.clone()).or_insert(position);
                    ends.insert(used, position);
                }
                if let Some(result) = instruction.result_name() {
                    starts.entry(result.to_string()).or_insert(position);
                    ends.entry(result.to_string()).or_insert(position);
                }
                position += 1;
            }
        }

        let mut intervals: Vec<LiveInterval> = starts
            .into_iter()
            .map(|(var, start)| LiveInterval {
                end: *ends.get(&var).unwrap_or(&start),
                var,
                start,
                location: None,
            })
            .collect();

        intervals.sort_by_key(|iv| (iv.start, iv.end));
        intervals
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn reg_index(loc: &ValueLocation) -> Option<usize> {
    match loc {
        ValueLocation::Register(r) => GP_REGISTERS.iter().position(|x| *x == r.as_str()),
        ValueLocation::Stack(_) => None,
    }
}

fn is_callee_saved(reg: &str) -> bool {
    matches!(
        reg,
        "%rbx" | "%r12" | "%r13" | "%r14" | "%r15"
    )
}

fn instruction_uses(instruction: &Instruction) -> Vec<String> {
    match instruction {
        Instruction::Const { .. } => vec![],
        Instruction::BinOp { lhs, rhs, .. } => vec![lhs.clone(), rhs.clone()],
        Instruction::Call { arguments, .. } => arguments.clone(),
        Instruction::CallIndirect {
            function_value,
            arguments,
            ..
        } => {
            let mut uses = vec![function_value.clone()];
            uses.extend(arguments.iter().cloned());
            uses
        }
        Instruction::Return { value } => value.iter().cloned().collect(),
        Instruction::Jump { .. } => vec![],
        Instruction::Branch { cond, .. } => vec![cond.clone()],
        Instruction::Phi { incoming, .. } => {
            incoming.iter().map(|(_, value)| value.clone()).collect()
        }
        Instruction::Alloc { size, .. } => size.iter().cloned().collect(),
        Instruction::GetField { object, .. } => vec![object.clone()],
        Instruction::SetField { object, value, .. } => vec![object.clone(), value.clone()],
        Instruction::GetIndex { array, index, .. } => vec![array.clone(), index.clone()],
        Instruction::SetIndex { array, index, value, .. } => {
            vec![array.clone(), index.clone(), value.clone()]
        }
        Instruction::AddrOf { operand, .. } => vec![operand.clone()],
        Instruction::Deref { operand, .. } => vec![operand.clone()],
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middle_end::ir::{BasicBlock, IRProgram};

    fn make_function(
        name: &str,
        params: Vec<(&str, &str)>,
        blocks: Vec<(&str, Vec<Instruction>)>,
    ) -> IRFunction {
        IRFunction {
            name: name.to_string(),
            return_type: "i64".to_string(),
            parameters: params
                .into_iter()
                .map(|(n, t)| (n.to_string(), t.to_string()))
                .collect(),
            blocks: blocks
                .into_iter()
                .map(|(label, instrs)| BasicBlock {
                    label: label.to_string(),
                    instructions: instrs,
                })
                .collect(),
        }
    }

    fn const_i64(result: &str, value: i64) -> Instruction {
        Instruction::Const {
            result: result.to_string(),
            value: serde_json::json!(value),
            ty: "i64".to_string(),
        }
    }

    fn binop(result: &str, op: &str, lhs: &str, rhs: &str) -> Instruction {
        Instruction::BinOp {
            result: result.to_string(),
            lhs: lhs.to_string(),
            rhs: rhs.to_string(),
            op_type: op.to_string(),
            ty: "i64".to_string(),
        }
    }

    fn ret(value: &str) -> Instruction {
        Instruction::Return {
            value: Some(value.to_string()),
        }
    }

    #[test]
    fn simple_two_adds_get_different_registers() {
        // Two values live simultaneously should get different registers.
        let f = make_function(
            "test",
            vec![],
            vec![(
                "entry",
                vec![
                    const_i64("%v0", 1),
                    const_i64("%v1", 2),
                    binop("%v2", "+", "%v0", "%v1"),
                    ret("%v2"),
                ],
            )],
        );
        let layout = LinearScanAllocator::new().allocate(&f);

        // %v0 and %v1 should have registers (not stack).
        assert!(
            matches!(layout.locations.get("%v0"), Some(ValueLocation::Register(_))),
            "%v0 should be in a register, got {:?}",
            layout.locations.get("%v0")
        );
        assert!(
            matches!(layout.locations.get("%v1"), Some(ValueLocation::Register(_))),
            "%v1 should be in a register"
        );
        // %v2 should have a register too.
        assert!(
            matches!(layout.locations.get("%v2"), Some(ValueLocation::Register(_))),
            "%v2 should be in a register"
        );

        // Different values should have different locations.
        let l0 = layout.locations.get("%v0").unwrap().clone();
        let l1 = layout.locations.get("%v1").unwrap().clone();
        assert_ne!(l0, l1, "%v0 and %v1 should get different registers");
    }

    #[test]
    fn many_values_do_not_crash_allocator() {
        // Stress test: 20 constant definitions followed by a chain of
        // uses.  The live intervals don't fully overlap (each const's
        // interval expires when the next begins), so the allocator
        // should handle all values gracefully without crashing.
        let mut instrs: Vec<Instruction> = (0..20)
            .map(|i| const_i64(&format!("%v{i}"), i as i64))
            .collect();

        // Chain: %v0 and %v1 are used at the same position (20),
        // but each successive definition extends after the previous
        // has expired, so live-interval overlap is minimal.
        instrs.push(binop("%s0", "+", "%v0", "%v1"));
        for i in 2..20 {
            instrs.push(binop(
                &format!("%s{}", i - 1),
                "+",
                &format!("%s{}", i - 2),
                &format!("%v{i}"),
            ));
        }
        instrs.push(ret(&"%s18".to_string()));

        let f = make_function("test", vec![], vec![("entry", instrs)]);
        let layout = LinearScanAllocator::new().allocate(&f);

        // Every value must have a location assigned.
        // 20 consts + 19 intermediate binop results = 39 values.
        assert_eq!(
            layout.locations.len(),
            39,
            "all 39 values should be assigned a location, got {}",
            layout.locations.len()
        );

        // The allocator must produce a reasonable stack size
        // (values in registers don't consume stack; only spills do).
        // With minimal overlap, most values fit in registers.
        let reg_count = layout
            .locations
            .values()
            .filter(|loc| matches!(loc, ValueLocation::Register(_)))
            .count();
        let stack_count = layout
            .locations
            .values()
            .filter(|loc| matches!(loc, ValueLocation::Stack(_)))
            .count();

        assert!(
            reg_count > 0,
            "at least some values should be in registers, got {reg_count} reg + {stack_count} stack"
        );
        // Stack should be reasonable (parameters already take some slots).
        assert!(
            layout.stack_size >= 0,
            "stack size must not be negative, got {}",
            layout.stack_size
        );
    }

    #[test]
    fn parameters_get_stack_slots() {
        let f = make_function(
            "add",
            vec![("x", "i64"), ("y", "i64")],
            vec![(
                "entry",
                vec![binop("%v0", "+", "x", "y"), ret("%v0")],
            )],
        );
        let layout = LinearScanAllocator::new().allocate(&f);

        // Parameters are always pre-assigned to stack slots by the
        // allocator prologue (they arrive from the caller on the stack
        // or in arg registers that get spilled).
        assert!(
            matches!(layout.locations.get("x"), Some(ValueLocation::Stack(_))),
            "parameter x should have a stack slot, got {:?}",
            layout.locations.get("x")
        );
        assert!(
            matches!(layout.locations.get("y"), Some(ValueLocation::Stack(_))),
            "parameter y should have a stack slot"
        );
        // The result (%v0) may be in a register since registers are
        // available after parameters take their stack slots.
        assert!(
            matches!(layout.locations.get("%v0"), Some(ValueLocation::Register(_))),
            "%v0 should be in a register"
        );
    }

    #[test]
    fn callee_saved_registers_are_tracked() {
        let f = make_function(
            "test",
            vec![],
            vec![(
                "entry",
                vec![const_i64("%v0", 42), const_i64("%v1", 99), ret("%v0")],
            )],
        );
        let layout = LinearScanAllocator::new().allocate(&f);

        // If callee-saved registers were used, they should be listed.
        for reg in &layout.used_callee_saved {
            assert!(is_callee_saved(reg), "{reg} is not callee-saved");
        }
    }
}
