//! Linear-scan register allocator for RISC-V + calling-convention prologue.
//!
//! # Register file
//!
//! | Group | Regs | Role |
//! |---|---|---|
//! | `zero` | `x0` | Hard-wired zero |
//! | `ra` | `x1` | Return address (caller-saved) |
//! | `sp` | `x2` | Stack pointer |
//! | `gp`/`tp` | `x3`/`x4` | Reserved |
//! | `t0–t6` | `x5–x7, x28–x31` | Caller-saved temporaries |
//! | `s0–s11` | `x8–x9, x18–x27` | Callee-saved |
//! | `a0–a7` | `x10–x17` | Argument / return registers |
//!
//! # Algorithm
//!
//! 1. Compute live intervals for every SSA value in the function.
//! 2. Linear scan: assign each interval to a physical register or spill.
//! 3. Determine which callee-saved regs (`sN`) are actually used.
//! 4. Build prologue: compute frame size, save `ra` + used `sN` regs.
//! 5. Rewrite all ops to use physical register names.

use std::collections::{HashMap, HashSet};
use crate::backend::riscv::instruction_select::{AddressingMode, RiscVOp, SelectedFunction};

// ---------------------------------------------------------------------------
// Register pools
// ---------------------------------------------------------------------------

/// Caller-saved temporaries (t0–t6).  First choice for short-lived values.
const TEMP_REGS: &[&str] = &["t0", "t1", "t2", "t3", "t4", "t5", "t6"];

/// Callee-saved registers (s1–s11).  For values that live across calls.
const SAVED_REGS: &[&str] = &[
    "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11",
];

/// Argument registers (a0–a7).  Used for function parameters / return.
const ARG_REGS: &[&str] = &["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7"];

/// s0 is used as frame pointer.
const FRAME_PTR: &str = "s0";
const ZERO_REG: &str = "zero";

// ---------------------------------------------------------------------------
// Live interval
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct LiveInterval {
    var: String,
    start: usize,
    end: usize,
    /// Crosses a call? (i.e. appears both before and after a `call`/`jalr`)
    crosses_call: bool,
}

// ---------------------------------------------------------------------------
// Per-function allocator state
// ---------------------------------------------------------------------------

struct AllocState {
    /// Physical register pool — `true` = free.
    temp_free: Vec<bool>,
    saved_free: Vec<bool>,
    /// Assigned location for each variable.
    assignment: HashMap<String, String>,
    /// Currently active intervals (in registers).
    active: Vec<(LiveInterval, usize, bool)>, // (interval, reg_idx, is_saved)
    /// Callee-saved registers actually used (need save/restore).
    used_saved: Vec<String>,
    /// Total stack frame size (aligned to 16).
    frame_size: i64,
    /// The function's ops — we scan them to detect calls.
    has_calls: bool,
}

impl AllocState {
    fn new() -> Self {
        AllocState {
            temp_free: vec![true; TEMP_REGS.len()],
            saved_free: vec![true; SAVED_REGS.len()],
            assignment: HashMap::new(),
            active: Vec::new(),
            used_saved: Vec::new(),
            frame_size: 0,
            has_calls: false,
        }
    }

    /// Expire intervals whose end ≤ current position.
    fn expire(&mut self, pos: usize) {
        self.active.retain(|(iv, reg_idx, is_saved)| {
            if iv.end <= pos {
                if *is_saved {
                    self.saved_free[*reg_idx] = true;
                } else {
                    self.temp_free[*reg_idx] = true;
                }
                false
            } else {
                true
            }
        });
    }

    /// Try to allocate a register for `iv`.  Spills to stack on failure.
    fn allocate(&mut self, iv: &LiveInterval) {
        // Prefer temps for non-call-crossing values, saved for call-crossing.
        let reg = if iv.crosses_call {
            // Call-crossing: use callee-saved (survive calls).
            let i = self.saved_free.iter().position(|&f| f);
            i.map(|idx| (idx, SAVED_REGS[idx].to_string(), true))
        } else {
            // Short-lived: try temps first, then saved.
            let ti = self.temp_free.iter().position(|&f| f);
            let i = match ti {
                Some(idx) => Some((idx, TEMP_REGS[idx].to_string(), false)),
                None => self.saved_free.iter().position(|&f| f).map(|idx| {
                    (idx, SAVED_REGS[idx].to_string(), true)
                }),
            };
            i
        };

        if let Some((reg_idx, reg_name, is_saved)) = reg {
            if is_saved {
                self.saved_free[reg_idx] = false;
                let name = SAVED_REGS[reg_idx].to_string();
                if !self.used_saved.contains(&name) {
                    self.used_saved.push(name);
                }
            } else {
                self.temp_free[reg_idx] = false;
            }
            self.assignment
                .insert(iv.var.clone(), reg_name);
            self.active
                .push((iv.clone(), reg_idx, is_saved));
        } else {
            // All registers occupied — spill to stack.
            let slot = self.frame_size;
            self.frame_size += 8;
            self.assignment.insert(
                iv.var.clone(),
                format!("%spill_{}", iv.var),
            );
        }
    }

    fn alloc_from_pool(
        &self,
        pool: &[bool],
        names: &[&str],
        _is_saved: bool,
    ) -> Option<(usize, String, bool)> {
        pool.iter()
            .position(|&free| free)
            .map(|idx| (idx, names[idx].to_string(), _is_saved))
    }
}

// ---------------------------------------------------------------------------
// Compute live intervals
// ---------------------------------------------------------------------------

fn compute_intervals(ops: &[&RiscVOp]) -> Vec<LiveInterval> {
    let mut starts: HashMap<String, usize> = HashMap::new();
    let mut ends: HashMap<String, usize> = HashMap::new();
    let mut crosses: HashSet<String> = HashSet::new();
    let mut seen_call = false;

    for (pos, op) in ops.iter().enumerate() {
        if matches!(op, RiscVOp::Call { .. } | RiscVOp::Jalr { .. }) {
            seen_call = true;
        }

        let uses = op_uses(op);
        let defs = op_defines(op);

        for u in &uses {
            starts.entry(u.clone()).or_insert(pos);
            ends.insert(u.clone(), pos);
            if seen_call {
                crosses.insert(u.clone());
            }
        }
        for d in &defs {
            starts.entry(d.clone()).or_insert(pos);
            ends.insert(d.clone(), pos);
            if seen_call {
                crosses.insert(d.clone());
            }
        }
    }

    starts
        .into_iter()
        .map(|(var, start)| LiveInterval {
            var,
            start,
            end: start,
            crosses_call: false,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Main allocator entry point
// ---------------------------------------------------------------------------

/// Run register allocation + prologue/epilogue on all functions.
pub fn allocate_and_frame(functions: &mut [SelectedFunction]) {
    for func in functions {
        let mut state = AllocState::new();

        // Scan to detect calls.
        for block in &func.blocks {
            for op in &block.ops {
                if matches!(op, RiscVOp::Call { .. } | RiscVOp::Jalr { .. }) {
                    state.has_calls = true;
                }
            }
        }

        // Collect all ops (flatten blocks for interval analysis).
        let all_ops: Vec<&RiscVOp> = func.blocks.iter().flat_map(|b| b.ops.iter()).collect();
        let intervals = compute_intervals(&all_ops);

        // Sort intervals by start position.
        let mut sorted: Vec<&LiveInterval> = intervals.iter().collect();
        sorted.sort_by_key(|iv| (iv.start, iv.end));

        // Linear scan.
        for iv in &sorted {
            state.expire(iv.start);
            state.allocate(iv);
        }

        // Align frame to 16 bytes.
        state.frame_size = ((state.frame_size + 15) / 16) * 16;

        // Rewrite all ops to use physical register names.
        for block in &mut func.blocks {
            for op in &mut block.ops {
                rewrite_op(op, &state.assignment);
            }
        }

        // Build prologue/epilogue.
        func.frame_size = state.frame_size;
        // Ensure at least 16 bytes of frame when AddrOf is used
        // (needs [s0, #-16] for temp slot).
        if func.frame_size < 16 {
            let has_addrof = func.blocks.iter().flat_map(|b| b.ops.iter()).any(|op| {
                if let RiscVOp::Sd { addr: AddressingMode::BaseOffset(base, -16), .. } = op {
                    base == "s0"
                } else {
                    false
                }
            });
            if has_addrof {
                func.frame_size = 16;
            }
        }
        let prologue = build_prologue(&state);
        let epilogue = build_epilogue(&state);

        // Prepend prologue and append epilogue to the entry block.
        if let Some(entry) = func.blocks.first_mut() {
            entry.ops.splice(0..0, prologue);
            entry.ops.push(RiscVOp::Label {
                label: "body".to_string(),
            });
            entry.ops.push(epilogue);
        }
    }
}

// ---------------------------------------------------------------------------
// Prologue / epilogue construction
// ---------------------------------------------------------------------------

fn build_prologue(state: &AllocState) -> Vec<RiscVOp> {
    let mut prologue = Vec::new();

    if state.frame_size == 0 && !state.has_calls && state.used_saved.is_empty() {
        return prologue; // leaf function, no frame needed
    }

    // Save ra if function calls other functions.
    if state.has_calls {
        prologue.push(RiscVOp::Sd {
            rs2: "ra".to_string(),
            addr: crate::backend::riscv::instruction_select::AddressingMode::BaseOffset(
                "sp".to_string(),
                -8,
            ),
        });
    }

    // Save used callee-saved registers.
    for (i, reg) in state.used_saved.iter().enumerate() {
        let offset = -8 - (i as i64 + 1) * 8;
        prologue.push(RiscVOp::Sd {
            rs2: reg.clone(),
            addr: crate::backend::riscv::instruction_select::AddressingMode::BaseOffset(
                "sp".to_string(),
                offset as i16,
            ),
        });
    }

    // Allocate stack frame.
    let save_size = (if state.has_calls { 1 } else { 0 } + state.used_saved.len()) as i64 * 8;
    let total_frame = state.frame_size.max(save_size + 16); // at least alignment space
    if total_frame > 0 {
        if total_frame <= 2047 {
            prologue.push(RiscVOp::Addi {
                rd: "sp".to_string(),
                rs1: "sp".to_string(),
                imm: -(total_frame as i16),
            });
        } else {
            // Frame too large for addi immediate — use a temporary.
            prologue.push(RiscVOp::Li {
                rd: "t0".to_string(),
                imm: total_frame,
            });
            prologue.push(RiscVOp::Sub {
                rd: "sp".to_string(),
                rs1: "sp".to_string(),
                rs2: "t0".to_string(),
            });
        }
    }

    // Set frame pointer.
    prologue.push(RiscVOp::Mv {
        rd: "s0".to_string(),
        rs1: "sp".to_string(),
    });

    prologue
}

fn build_epilogue(state: &AllocState) -> RiscVOp {
    // Restore sp, restore saved regs, restore ra, return.
    // The epilogue is emitted as a single label target.
    // For simplicity, the emitter handles it inline.
    RiscVOp::Epilogue
}

// ---------------------------------------------------------------------------
// Rewrite virtual → physical register names
// ---------------------------------------------------------------------------

fn rewrite_op(op: &mut RiscVOp, assignment: &HashMap<String, String>) {
    let phys = |name: &str| -> String {
        if name.starts_with('%') && !name.starts_with("sp")
            && !name.starts_with("ra") && !name.starts_with("zero")
            && !name.starts_with("s0")
        {
            assignment
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.to_string())
        } else {
            name.to_string()
        }
    };

    match op {
        RiscVOp::Add { rd, rs1, rs2 }
        | RiscVOp::Sub { rd, rs1, rs2 }
        | RiscVOp::Mul { rd, rs1, rs2 }
        | RiscVOp::Div { rd, rs1, rs2 }
        | RiscVOp::And { rd, rs1, rs2 }
        | RiscVOp::Or { rd, rs1, rs2 }
        | RiscVOp::Xor { rd, rs1, rs2 } => {
            *rd = phys(rd);
            *rs1 = phys(rs1);
            *rs2 = phys(rs2);
        }
        RiscVOp::Addi { rd, rs1, .. }
        | RiscVOp::Sltiu { rd, rs1, .. }
        | RiscVOp::Xori { rd, rs1, .. }
        | RiscVOp::Ori { rd, rs1, .. }
        | RiscVOp::Andi { rd, rs1, .. } => {
            *rd = phys(rd);
            *rs1 = phys(rs1);
        }
        RiscVOp::Slli { rd, rs1, .. }
        | RiscVOp::Srli { rd, rs1, .. }
        | RiscVOp::Srai { rd, rs1, .. } => {
            *rd = phys(rd);
            *rs1 = phys(rs1);
        }
        RiscVOp::Lui { rd, .. } | RiscVOp::Li { rd, .. } => {
            *rd = phys(rd);
        }
        RiscVOp::Mv { rd, rs1 } => {
            *rd = phys(rd);
            *rs1 = phys(rs1);
        }
        RiscVOp::Slt { rd, rs1, rs2 }
        | RiscVOp::Sltu { rd, rs1, rs2 } => {
            *rd = phys(rd);
            *rs1 = phys(rs1);
            *rs2 = phys(rs2);
        }
        RiscVOp::Slti { rd, rs1, .. } => {
            *rd = phys(rd);
            *rs1 = phys(rs1);
        }
        RiscVOp::Seqz { rd, rs } | RiscVOp::Snez { rd, rs } => {
            *rd = phys(rd);
            *rs = phys(rs);
        }
        RiscVOp::Ld { rd, addr } | RiscVOp::Lbu { rd, addr } => {
            *rd = phys(rd);
            *addr = rewrite_addr(addr, assignment);
        }
        RiscVOp::Sd { rs2, addr } | RiscVOp::Sb { rs2, addr } => {
            *rs2 = phys(rs2);
            *addr = rewrite_addr(addr, assignment);
        }
        RiscVOp::Beq { rs1, rs2, .. }
        | RiscVOp::Bne { rs1, rs2, .. }
        | RiscVOp::Blt { rs1, rs2, .. }
        | RiscVOp::Bge { rs1, rs2, .. } => {
            // Leave branch labels and conditions as-is.
        }
        RiscVOp::J { .. }
        | RiscVOp::Call { .. }
        | RiscVOp::Jalr { .. }
        | RiscVOp::Ret
        | RiscVOp::Label { .. }
        | RiscVOp::Prologue { .. }
        | RiscVOp::Epilogue => {}
    }
}

fn rewrite_addr(
    addr: &mut crate::backend::riscv::instruction_select::AddressingMode,
    assignment: &HashMap<String, String>,
) -> crate::backend::riscv::instruction_select::AddressingMode {
    let src = addr.clone();
    match src {
        crate::backend::riscv::instruction_select::AddressingMode::Base(r) => {
            crate::backend::riscv::instruction_select::AddressingMode::Base(
                assignment.get(&r).cloned().unwrap_or(r),
            )
        }
        crate::backend::riscv::instruction_select::AddressingMode::BaseOffset(r, o) => {
            crate::backend::riscv::instruction_select::AddressingMode::BaseOffset(
                assignment.get(&r).cloned().unwrap_or(r),
                o,
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn op_uses(op: &RiscVOp) -> Vec<String> {
    match op {
        RiscVOp::Add { rs1, rs2, .. }
        | RiscVOp::Sub { rs1, rs2, .. }
        | RiscVOp::Mul { rs1, rs2, .. }
        | RiscVOp::Div { rs1, rs2, .. }
        | RiscVOp::And { rs1, rs2, .. }
        | RiscVOp::Or { rs1, rs2, .. }
        | RiscVOp::Xor { rs1, rs2, .. } => vec![rs1.clone(), rs2.clone()],
        RiscVOp::Addi { rs1, .. }
        | RiscVOp::Sltiu { rs1, .. }
        | RiscVOp::Xori { rs1, .. }
        | RiscVOp::Ori { rs1, .. }
        | RiscVOp::Andi { rs1, .. }
        | RiscVOp::Slli { rs1, .. }
        | RiscVOp::Srli { rs1, .. }
        | RiscVOp::Srai { rs1, .. }
        | RiscVOp::Slti { rs1, .. } => vec![rs1.clone()],
        RiscVOp::Mv { rs1, .. } => vec![rs1.clone()],
        RiscVOp::Slt { rs1, rs2, .. } | RiscVOp::Sltu { rs1, rs2, .. } => {
            vec![rs1.clone(), rs2.clone()]
        }
        RiscVOp::Seqz { rs, .. } | RiscVOp::Snez { rs, .. } => vec![rs.clone()],
        RiscVOp::Ld { addr, .. } | RiscVOp::Lbu { addr, .. } => {
            let mut v = vec![];
            if let crate::backend::riscv::instruction_select::AddressingMode::Base(r) = addr {
                v.push(r.clone());
            }
            if let crate::backend::riscv::instruction_select::AddressingMode::BaseOffset(r, _) = addr {
                v.push(r.clone());
            }
            v
        }
        RiscVOp::Sd { rs2, addr, .. } | RiscVOp::Sb { rs2, addr, .. } => {
            let mut v = vec![rs2.clone()];
            if let crate::backend::riscv::instruction_select::AddressingMode::Base(r) = addr {
                v.push(r.clone());
            }
            if let crate::backend::riscv::instruction_select::AddressingMode::BaseOffset(r, _) = addr {
                v.push(r.clone());
            }
            v
        }
        RiscVOp::Beq { rs1, rs2, .. }
        | RiscVOp::Bne { rs1, rs2, .. }
        | RiscVOp::Blt { rs1, rs2, .. }
        | RiscVOp::Bge { rs1, rs2, .. } => vec![rs1.clone(), rs2.clone()],
        RiscVOp::Jalr { rs1 } => vec![rs1.clone()],
        _ => vec![],
    }
}

fn op_defines(op: &RiscVOp) -> Vec<String> {
    match op {
        RiscVOp::Add { rd, .. }
        | RiscVOp::Sub { rd, .. }
        | RiscVOp::Mul { rd, .. }
        | RiscVOp::Div { rd, .. }
        | RiscVOp::And { rd, .. }
        | RiscVOp::Or { rd, .. }
        | RiscVOp::Xor { rd, .. }
        | RiscVOp::Addi { rd, .. }
        | RiscVOp::Sltiu { rd, .. }
        | RiscVOp::Xori { rd, .. }
        | RiscVOp::Ori { rd, .. }
        | RiscVOp::Andi { rd, .. }
        | RiscVOp::Slli { rd, .. }
        | RiscVOp::Srli { rd, .. }
        | RiscVOp::Srai { rd, .. }
        | RiscVOp::Lui { rd, .. }
        | RiscVOp::Ld { rd, .. }
        | RiscVOp::Lbu { rd, .. }
        | RiscVOp::Li { rd, .. }
        | RiscVOp::Mv { rd, .. }
        | RiscVOp::Slt { rd, .. }
        | RiscVOp::Sltu { rd, .. }
        | RiscVOp::Slti { rd, .. }
        | RiscVOp::Seqz { rd, .. }
        | RiscVOp::Snez { rd, .. } => vec![rd.clone()],

        RiscVOp::Label { .. }
        | RiscVOp::J { .. }
        | RiscVOp::Call { .. }
        | RiscVOp::Jalr { .. }
        | RiscVOp::Ret
        | RiscVOp::Beq { .. }
        | RiscVOp::Bne { .. }
        | RiscVOp::Blt { .. }
        | RiscVOp::Bge { .. }
        | RiscVOp::Prologue { .. }
        | RiscVOp::Epilogue => vec![],

        RiscVOp::Sd { .. } | RiscVOp::Sb { .. } => vec![],
    }
}
