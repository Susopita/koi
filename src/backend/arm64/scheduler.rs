//! List scheduling over A64Ops.
//!
//! Builds a data-dependency DAG for each basic block, assigns latency
//! weights to each instruction, then reorders them to hide memory latency
//! by interleaving independent ALU work.
//!
//! # Latency model (ARM Cortex-A72-ish cycles)
//!
//! | Op class | Latency | Pipe |
//! |---|---|---|
//! | ALU reg-reg (`add`, `sub`, `and`, `lsl`, ...) | 1 | I0/I1 |
//! | ALU imm (`addimm`, `movimm`, ...) | 1 | I0/I1 |
//! | Mul / Div | 4 / 16 | M0 / D0 |
//! | Load (`ldr`, `ldp`, `ldrb`, `ldrsw`) | 4 | L0 |
//! | Store (`str`, `stp`, `strb`) | 1 (addr) | S0 |
//! | Branch / call (`b`, `bl`, `blr`) | 1 (direct) / 3 (indirect) | B0 |
//! | Float ALU (`fadd`, `fmul`, `fdiv`) | 3 / 5 / 10 | F0 |
//! | Float load / store | 5 / 1 | L0 / S0 |
//! | Conditional select (`csel`, `csinc`) | 2 | I0 |
//! | Mov wide (`movz`, `movk`, `movreg`) | 1 | I0 |

use std::cmp::Reverse;
use std::collections::{BinaryHeap, BTreeMap, HashSet};

use crate::backend::arm64::instruction_select::{A64Op, SelectedFunction};

// ---------------------------------------------------------------------------
// DAG types
// ---------------------------------------------------------------------------

/// A single node in the dependence DAG.
#[derive(Debug, Clone)]
struct DagNode {
    /// Index in the original instruction list.
    _index: usize,
    /// The operation itself (borrowed for analysis, not stored here).
    /// Latency of this instruction — cycles until its result is ready.
    latency: u32,
    /// Predecessor indices (nodes that must issue before this one).
    preds: Vec<usize>,
    /// Successor indices (nodes that depend on this one).
    succs: Vec<usize>,
    /// Height of the longest path from this node to a leaf (critical path).
    height: u32,
    /// Earliest cycle this node can start.
    earliest: u32,
}

// ---------------------------------------------------------------------------
// Schedule a single basic block
// ---------------------------------------------------------------------------

/// Reorder the operations in a basic block to minimise stalls.
///
/// Terminators (B, BCond, Ret) are excluded from the scheduling DAG —
/// they always remain at the end of the block in their original order.
/// Only non-terminator instructions are reordered to hide latencies.
pub fn schedule_block(ops: &[A64Op]) -> Vec<A64Op> {
    // ---- Phase 0: Separate terminators ----
    // Terminators must stay at the end of the block and are not part of
    // the scheduling DAG.  Extract them first, schedule only the rest.
    let is_term = |op: &A64Op| -> bool {
        matches!(op, A64Op::B { .. } | A64Op::BCond { .. } | A64Op::Ret)
    };

    let non_terms: Vec<&A64Op> = ops.iter().filter(|op| !is_term(op)).collect();
    let terms: Vec<&A64Op> = ops.iter().filter(|op| is_term(op)).collect();

    if non_terms.len() <= 2 {
        // Not enough schedulable instructions to benefit from reordering.
        return ops.to_vec();
    }

    // ---- Phase 1: Build the DAG (from non-terminator ops only) ----
    let n = non_terms.len();
    let mut nodes: Vec<DagNode> = (0..n)
        .map(|i| DagNode {
            _index: i,
            latency: op_latency(non_terms[i]),
            preds: Vec::new(),
            succs: Vec::new(),
            height: 0,
            earliest: 0,
        })
        .collect();

    let mut last_write: BTreeMap<String, usize> = BTreeMap::new();
    let mut last_read: BTreeMap<String, Vec<usize>> = BTreeMap::new();

    for i in 0..n {
        let defs = op_defines(non_terms[i]);
        let uses = op_uses(non_terms[i]);

        // ---- RAW edges: this instruction reads what a previous wrote ----
        for u in &uses {
            if let Some(&writer) = last_write.get(u) {
                add_edge(&mut nodes, writer, i, "RAW");
            }
        }

        // ---- WAR edges: this instruction writes what a previous reads ----
        for d in &defs {
            if let Some(readers) = last_read.get(d) {
                for &reader in readers {
                    add_edge(&mut nodes, reader, i, "WAR");
                }
            }
        }

        // ---- WAW edges: two consecutive writes to the same register ----
        for d in &defs {
            if let Some(&prev_writer) = last_write.get(d) {
                add_edge(&mut nodes, prev_writer, i, "WAW");
            }
        }

        // Update tracking.
        for d in &defs {
            last_write.insert(d.clone(), i);
            last_read.remove(d);
        }
        for u in &uses {
            last_read.entry(u.clone()).or_default().push(i);
        }
    }

    // ---- Phase 2: Compute critical-path heights (backward) ----
    for i in (0..n).rev() {
        let max_succ = nodes[i]
            .succs
            .iter()
            .map(|&s| nodes[s].latency + nodes[s].height)
            .max()
            .unwrap_or(0);
        nodes[i].height = max_succ;
    }

    // ---- Phase 3: List scheduling (non-terminators only) ----
    let mut remaining: HashSet<usize> = (0..n).collect();
    let mut ready: BinaryHeap<Reverse<PriorityNode>> = BinaryHeap::new();
    let mut scheduled: Vec<A64Op> = Vec::with_capacity(n + terms.len());
    let mut cycle: u32 = 0;

    for i in 0..n {
        if nodes[i].preds.is_empty() {
            ready.push(Reverse(PriorityNode {
                cycle_ready: 0,
                height: nodes[i].height,
                index: i,
            }));
        }
    }

    while let Some(Reverse(pn)) = ready.pop() {
        let i = pn.index;
        cycle = cycle.max(pn.cycle_ready);

        scheduled.push(non_terms[i].clone());

        let succs: Vec<usize> = nodes[i].succs.clone();
        for &s in &succs {
            let start = cycle + nodes[i].latency;
            nodes[s].earliest = nodes[s].earliest.max(start);
            nodes[s].preds.retain(|&p| p != i);
            if nodes[s].preds.is_empty() && remaining.contains(&s) {
                ready.push(Reverse(PriorityNode {
                    cycle_ready: nodes[s].earliest,
                    height: nodes[s].height,
                    index: s,
                }));
            }
        }

        remaining.remove(&i);
    }

    // ---- Phase 4: Append terminators in their original order ----
    scheduled.extend(terms.into_iter().cloned());
    scheduled
}

// ---------------------------------------------------------------------------
// Priority for the ready queue
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct PriorityNode {
    /// Earliest cycle this node can begin (sorted ascending).
    cycle_ready: u32,
    /// Height of the critical path from this node (sorted descending).
    height: u32,
    index: usize,
}

impl Ord for PriorityNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Primary key: cycle_ready (lower first).
        // Secondary: height (higher first = critical path first).
        self.cycle_ready
            .cmp(&other.cycle_ready)
            .then_with(|| other.height.cmp(&self.height))
    }
}

impl PartialOrd for PriorityNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------
// Helper: add a dependence edge
// ---------------------------------------------------------------------------

fn add_edge(nodes: &mut Vec<DagNode>, from: usize, to: usize, _kind: &str) {
    // Avoid duplicate edges (common when a node reads the same reg twice).
    if nodes[to].preds.contains(&from) {
        return;
    }
    nodes[from].succs.push(to);
    nodes[to].preds.push(from);
}

// ---------------------------------------------------------------------------
// Extract register-usage info (re-implemented here to keep scheduler
// self-contained, avoiding import cycles).
// ---------------------------------------------------------------------------

/// Register names used (read) by an op.
fn op_uses(op: &A64Op) -> Vec<String> {
    match op {
        A64Op::Add { rn, rm, .. }
        | A64Op::Sub { rn, rm, .. }
        | A64Op::And { rn, rm, .. }
        | A64Op::Orr { rn, rm, .. }
        | A64Op::Eor { rn, rm, .. } => vec![rn.clone(), rm.reg.clone()],
        A64Op::AddImm { rn, .. }
        | A64Op::SubImm { rn, .. }
        | A64Op::CmpImm { rn, .. } => vec![rn.clone()],
        A64Op::Mul { rn, rm, .. } | A64Op::Sdiv { rn, rm, .. } => {
            vec![rn.clone(), rm.clone()]
        }
        A64Op::Lsl { rn, .. }
        | A64Op::Lsr { rn, .. }
        | A64Op::Asr { rn, .. } => vec![rn.clone()],
        A64Op::Cmp { rn, rm } => vec![rn.clone(), rm.reg.clone()],
        A64Op::Csel { rn, rm, .. } | A64Op::Csinc { rn, rm, .. } => {
            let mut v = vec![rn.clone(), rm.clone()];
            v.push("nzcv".into());
            v
        }
        A64Op::Cset { .. } | A64Op::BCond { .. } => {
            vec!["nzcv".to_string()]
        }
        // Chain all control-flow ops (branches, calls, returns) so the
        // scheduler cannot reorder them past each other.
        A64Op::B { .. } | A64Op::Ret => {
            vec!["ctrl".to_string()]
        }
        A64Op::Bl { .. } => {
            // Bl implicitly uses all argument registers (x0-x7) and memory
            // (callee may read through any pointer), so stores before the
            // call are not reordered past it.
            vec!["ctrl".to_string(), "mem".to_string(),
                 "x0".to_string(), "x1".to_string(),
                 "x2".to_string(), "x3".to_string(),
                 "x4".to_string(), "x5".to_string(),
                 "x6".to_string(), "x7".to_string()]
        }
        A64Op::Blr { reg } => {
            let v = vec!["ctrl".to_string(), "mem".to_string(),
                         "x0".to_string(), "x1".to_string(),
                         "x2".to_string(), "x3".to_string(),
                         "x4".to_string(), "x5".to_string(),
                         "x6".to_string(), "x7".to_string(), reg.clone()];
            v
        }
        A64Op::PrintI64Arg { reg } | A64Op::PrintStringArg { reg } | A64Op::PrintF64Arg { reg } => {
            vec!["ctrl".to_string(), "mem".to_string(), reg.clone()]
        }
        A64Op::MovReg { rm, .. } => vec![rm.clone()],
        A64Op::Str { rs, addr, .. } | A64Op::StrFloat { rs, addr } => {
            let mut v = vec!["mem".to_string(), rs.clone()];
            addr_uses(&mut v, addr);
            v
        }
        A64Op::Stp { rt1, rt2, addr, .. } => {
            let mut v = vec!["mem".to_string(), rt1.clone(), rt2.clone()];
            addr_uses(&mut v, addr);
            v
        }
        A64Op::Strb { rs, addr } => {
            let mut v = vec!["mem".to_string(), rs.clone()];
            addr_uses(&mut v, addr);
            v
        }
        A64Op::Ldr { addr, .. }
        | A64Op::Ldrb { addr, .. }
        | A64Op::Ldrsw { addr, .. }
        | A64Op::LdrFloat { addr, .. } => {
            let mut v = vec!["mem".to_string()];
            addr_uses(&mut v, addr);
            v
        }
        A64Op::Ldp { addr, .. } => {
            let mut v = vec!["mem".to_string()];
            addr_uses(&mut v, addr);
            v
        }
        // Float scratch ops (using d0/d1) are serialised via "fpu" resource
        // to prevent the scheduler from reordering them.
        A64Op::FAdd { rn, rm, .. }
        | A64Op::FSub { rn, rm, .. }
        | A64Op::FMul { rn, rm, .. }
        | A64Op::FDiv { rn, rm, .. } => vec![rn.clone(), rm.clone(), "fpu".to_string()],
        A64Op::FCmp { rn, rm } => vec![rn.clone(), rm.clone(), "fpu".to_string()],
        A64Op::FMov { rd, rm } => {
            let mut v = vec![rm.clone()];
            if rd == "d0" || rd == "d1" || rm == "d0" || rm == "d1" {
                v.push("fpu".to_string());
            }
            v
        }
        A64Op::AddrOf { rn, .. } => vec![rn.clone()],
        _ => vec![],
    }
}

/// Register names defined (written) by an op.
fn op_defines(op: &A64Op) -> Vec<String> {
    match op {
        A64Op::Add { rd, .. }
        | A64Op::Sub { rd, .. }
        | A64Op::Mul { rd, .. }
        | A64Op::Sdiv { rd, .. }
        | A64Op::And { rd, .. }
        | A64Op::Orr { rd, .. }
        | A64Op::Eor { rd, .. }
        | A64Op::Lsl { rd, .. }
        | A64Op::Lsr { rd, .. }
        | A64Op::Asr { rd, .. }
        | A64Op::AddImm { rd, .. }
        | A64Op::SubImm { rd, .. }
        | A64Op::MovImm { rd, .. }
        | A64Op::MovReg { rd, .. }
        | A64Op::Csel { rd, .. }
        | A64Op::Csinc { rd, .. }
        | A64Op::Cset { rd, .. }
        | A64Op::Movz { rd, .. }
        | A64Op::Movk { rd, .. }
        | A64Op::FMovImm { rd, .. }
        | A64Op::LoadString { rd, .. }
        | A64Op::LoadFuncAddr { rd, .. }
        | A64Op::AddrOf { rd, .. }
        | A64Op::LoadFloat { rd, .. } => vec![rd.clone()],
        // Loads define both the destination register AND "mem" (to serialize
        // with stores). Stores define only "mem".
        A64Op::Ldr { rd, .. } | A64Op::Ldrb { rd, .. } | A64Op::Ldrsw { rd, .. }
        | A64Op::LdrFloat { rd, .. } => vec![rd.clone(), "mem".to_string()],
        A64Op::Ldp { rt1, rt2, .. } => vec![rt1.clone(), rt2.clone(), "mem".to_string()],
        A64Op::Str { .. } | A64Op::Stp { .. } | A64Op::Strb { .. } | A64Op::StrFloat { .. } => {
            vec!["mem".to_string()]
        }
        // Float scratch ops also define "fpu" to prevent reordering.
        A64Op::FAdd { rd, .. } | A64Op::FSub { rd, .. } | A64Op::FMul { rd, .. }
        | A64Op::FDiv { rd, .. } | A64Op::FMov { rd, .. } => {
            vec!["fpu".to_string(), rd.clone()]
        }
        A64Op::FCmp { .. } => vec!["fpu".to_string(), "nzcv".to_string()],
        A64Op::Cmp { .. } | A64Op::CmpImm { .. } | A64Op::FCmp { .. } => vec!["nzcv".to_string()],
        // Chain control-flow ops to prevent reordering across them.
        // Calls define both control (for ordering) and x0 (return value
        // register), so the scheduler knows argument MovReg instructions
        // cannot be moved past the call.
        A64Op::Bl { .. } | A64Op::Blr { .. } => {
            vec!["ctrl".to_string(), "x0".to_string(), "mem".to_string()]
        }
        A64Op::B { .. } | A64Op::BCond { .. } | A64Op::Ret => {
            vec!["ctrl".to_string()]
        }
        // Print pseudo-ops emit inline str/bl that touch memory.
        A64Op::PrintI64Arg { .. } | A64Op::PrintStringArg { .. } | A64Op::PrintF64Arg { .. } => {
            vec!["ctrl".to_string(), "mem".to_string()]
        }
        _ => vec![],
    }
}

fn addr_uses(uses: &mut Vec<String>, addr: &crate::backend::arm64::instruction_select::AddressingMode) {
    match addr {
        crate::backend::arm64::instruction_select::AddressingMode::Base(r)
        | crate::backend::arm64::instruction_select::AddressingMode::BaseOffset(r, _)
        | crate::backend::arm64::instruction_select::AddressingMode::PreIndexed(r, _)
        | crate::backend::arm64::instruction_select::AddressingMode::PostIndexed(r, _) => {
            uses.push(r.clone());
        }
        crate::backend::arm64::instruction_select::AddressingMode::RegisterOffset {
            base, index, ..
        } => {
            uses.push(base.clone());
            uses.push(index.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// Per-function scheduler
// ---------------------------------------------------------------------------

/// Run list scheduling on all blocks in all functions.
pub fn schedule_functions(functions: &mut [SelectedFunction]) {
    for func in functions {
        for block in &mut func.blocks {
            block.ops = schedule_block(&block.ops);
        }
    }
}

// ---------------------------------------------------------------------------
// Latency table
// ---------------------------------------------------------------------------

/// Cycles until the result of `op` is available to a dependent instruction.
fn op_latency(op: &A64Op) -> u32 {
    match op {
        // Fast ALU — 1 cycle
        A64Op::Add { .. }
        | A64Op::Sub { .. }
        | A64Op::And { .. }
        | A64Op::Orr { .. }
        | A64Op::Eor { .. }
        | A64Op::Lsl { .. }
        | A64Op::Lsr { .. }
        | A64Op::Asr { .. }
        | A64Op::AddImm { .. }
        | A64Op::SubImm { .. }
        | A64Op::MovImm { .. }
        | A64Op::MovReg { .. }
        | A64Op::Movz { .. }
        | A64Op::Movk { .. }
        | A64Op::B { .. }
        | A64Op::BCond { .. }
        | A64Op::Cmp { .. }
        | A64Op::CmpImm { .. }
        | A64Op::Cset { .. } => 1,

        // Conditional select — 2 cycles
        A64Op::Csel { .. } | A64Op::Csinc { .. } => 2,

        // 32×32 multiply — 4 cycles, 64×64 → 5
        A64Op::Mul { .. } => 4,

        // Integer divide — 16 cycles
        A64Op::Sdiv { .. } => 16,

        // Loads — 4 cycles (L1 hit)
        A64Op::Ldr { .. }
        | A64Op::Ldrb { .. }
        | A64Op::Ldrsw { .. }
        | A64Op::Ldp { .. } => 4,

        // Stores — address calculation 1 cycle, but no result to wait for.
        // We return 1 so the scheduler doesn't stall on the store itself.
        A64Op::Str { .. } | A64Op::Stp { .. } | A64Op::Strb { .. } => 1,

        // Float ALU
        A64Op::FAdd { .. } | A64Op::FSub { .. } => 3,
        A64Op::FMul { .. } => 5,
        A64Op::FDiv { .. } => 10,
        A64Op::FMov { .. } | A64Op::FMovImm { .. } => 1,
        A64Op::FCmp { .. } => 2,

        // Float loads/stores
        A64Op::LdrFloat { .. } => 5,
        A64Op::StrFloat { .. } => 1,

        // Branches / calls
        A64Op::Bl { .. } => 1,
        A64Op::Blr { .. } => 3,
        A64Op::Ret => 1,

        // Stack frame ops
        A64Op::StpFrame | A64Op::LdpFrame => 4,
        A64Op::Prologue { .. } | A64Op::Epilogue => 1,

        // Print ops
        A64Op::PrintI64Arg { .. } | A64Op::PrintStringArg { .. } | A64Op::PrintF64Arg { .. } => 1,
        A64Op::LoadString { .. } => 1,
        A64Op::LoadFuncAddr { .. } => 2,
        A64Op::AddrOf { .. } => 1,
        A64Op::LoadFloat { .. } => 4,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::arm64::instruction_select::AddressingMode;

    #[test]
    fn two_independent_adds_are_unchanged() {
        let ops = vec![
            A64Op::AddImm {
                rd: "x0".into(),
                rn: "x1".into(),
                imm: 1,
                ty: "i64".into(),
            },
            A64Op::AddImm {
                rd: "x2".into(),
                rn: "x3".into(),
                imm: 2,
                ty: "i64".into(),
            },
        ];
        let scheduled = schedule_block(&ops);
        assert_eq!(scheduled.len(), 2);
    }

    #[test]
    fn raw_dependency_preserves_order() {
        // v1 = v0 + 1  →  v2 = v1 + 2
        // The second depends on the first (RAW on v1).
        let ops = vec![
            A64Op::AddImm {
                rd: "v1".into(),
                rn: "v0".into(),
                imm: 1,
                ty: "i64".into(),
            },
            A64Op::AddImm {
                rd: "v2".into(),
                rn: "v1".into(),
                imm: 2,
                ty: "i64".into(),
            },
        ];
        let scheduled = schedule_block(&ops);
        assert_eq!(scheduled.len(), 2);
        // Order must be preserved — first op should be the AddImm on v1.
        match &scheduled[0] {
            A64Op::AddImm { rd, rn, imm, .. } => {
                assert_eq!(rd, "v1");
                assert_eq!(rn, "v0");
                assert_eq!(*imm, 1);
            }
            other => panic!("expected AddImm, got {other:?}"),
        }
    }

    #[test]
    fn load_then_independent_alu_interleaves() {
        // ldr x0, [sp, #0]  (latency 4)
        // add x2, x3, x4    (latency 1, independent)
        // add x1, x0, #1     (depends on load — RAW)
        // The independent add should stay between the load and the
        // dependent add, hiding the load latency.
        let ops = vec![
            A64Op::Ldr {
                rd: "x0".into(),
                addr: AddressingMode::BaseOffset("sp".into(), 0),
                ty: "i64".into(),
            },
            A64Op::Add {
                rd: "x2".into(),
                rn: "x3".into(),
                rm: crate::backend::arm64::instruction_select::ExtendedReg::plain("x4"),
                ty: "i64".into(),
            },
            A64Op::AddImm {
                rd: "x1".into(),
                rn: "x0".into(),
                imm: 1,
                ty: "i64".into(),
            },
        ];
        let scheduled = schedule_block(&ops);
        assert_eq!(scheduled.len(), 3);
        // The independent add must come before the dependent add.
        let second_is_add = matches!(&scheduled[1], A64Op::Add { .. });
        assert!(second_is_add, "independent ALU should be scheduled second");
    }

    #[test]
    fn store_preserves_addr_dependency() {
        // add x0, ...  →  str x1, [x0, #0]
        // The store depends on the add for its base address.
        let ops = vec![
            A64Op::AddImm {
                rd: "x0".into(),
                rn: "sp".into(),
                imm: 8,
                ty: "i64".into(),
            },
            A64Op::Str {
                rs: "x1".into(),
                addr: AddressingMode::BaseOffset("x0".into(), 0),
                ty: "i64".into(),
            },
        ];
        let scheduled = schedule_block(&ops);
        assert_eq!(scheduled.len(), 2);
    }
}
