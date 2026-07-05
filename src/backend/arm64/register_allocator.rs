//! Chaitin-Briggs graph-colouring register allocator for ARM64.
//!
//! Builds an interference graph from live intervals, colours each node
//! (virtual register) with one of 30 physical registers (`x0`–`x30` minus
//! `xzr`), and spills to the stack when colouring fails.  A coalescing
//! heuristic groups adjacent loads/stores into `LDP`/`STP` pairs.
//!
//! # Physical register file
//!
//! Allocatable: `x0`–`x30` except `xzr` — 30 registers.
//! Reserved / special-purpose:
//! - `sp` — stack pointer
//! - `xzr` — zero register
//! - `x29` — frame pointer
//! - `x30` — link register (call return address)
//!
//! `x29` and `x30` are reserved only when the current function contains
//! calls; otherwise they are free for allocation.

use std::collections::{HashMap, HashSet};
use crate::backend::arm64::instruction_select::{A64Op, AddressingMode, SelectedBlock, SelectedFunction};

// ---------------------------------------------------------------------------
// Physical register pool
// ---------------------------------------------------------------------------

/// All AArch64 callee-saved general-purpose registers (x19-x28).
/// Caller-saved registers (x0-x18) are not allocatable because values
/// assigned to them would be clobbered by function calls.
const ALL_REGS: &[&str] = &[
    "x19", "x20", "x21", "x22", "x23", "x24", "x25", "x26", "x27", "x28",
];

/// x0-x1 are used for ABI argument passing and return values.
const RESERVED: &[&str] = &["x29", "x30"];

/// Number of allocatable registers (10 callee-saved).
const NUM_REGS: usize = 10;

// ---------------------------------------------------------------------------
// Interference graph
// ---------------------------------------------------------------------------

/// One node in the interference graph — a virtual register (SSA value).
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    /// Registers this node cannot share (live at the same time).
    pub interference: HashSet<usize>,
    /// Preferred physical register colour, or `None`.
    pub colour: Option<usize>,
    /// Whether this node has been pre-coloured (e.g. ABI parameter regs).
    pub pre_coloured: bool,
    /// Degree in the interference graph (number of neighbours).
    pub degree: usize,
    /// Whether this node is currently on the stack (simplification phase).
    pub on_stack: bool,
    /// Names of variables that are the "same" — candidates for coalescing
    /// with this node (e.g. a mov from one to another).
    pub coalesce_group: Vec<String>,
}

/// The full interference graph.
#[derive(Debug, Clone)]
pub struct InterferenceGraph {
    pub nodes: Vec<Node>,
    /// Map from value name → node index.
    pub name_to_idx: HashMap<String, usize>,
    /// Physical register names.
    pub phys_regs: Vec<&'static str>,
    /// Number of colours available.
    pub num_colours: usize,
}

impl InterferenceGraph {
    pub fn new() -> Self {
        let phys_regs: Vec<&str> = ALL_REGS
            .iter()
            .filter(|r| !RESERVED.contains(r))
            .copied()
            .collect();

        InterferenceGraph {
            nodes: Vec::new(),
            name_to_idx: HashMap::new(),
            num_colours: phys_regs.len(),
            phys_regs,
        }
    }

    /// Add a node for `name`, or return its index if it already exists.
    fn get_or_create(&mut self, name: &str) -> usize {
        if let Some(&idx) = self.name_to_idx.get(name) {
            return idx;
        }
        let idx = self.nodes.len();
        self.name_to_idx.insert(name.to_string(), idx);
        self.nodes.push(Node {
            name: name.to_string(),
            interference: HashSet::new(),
            colour: None,
            pre_coloured: false,
            degree: 0,
            on_stack: false,
            coalesce_group: Vec::new(),
        });
        idx
    }

    /// Record interference between `a` and `b` (undirected).
    fn interfere(&mut self, a: &str, b: &str) {
        if a == b {
            return;
        }
        let ia = self.get_or_create(a);
        let ib = self.get_or_create(b);
        self.nodes[ia].interference.insert(ib);
        self.nodes[ib].interference.insert(ia);
    }

    /// Record a coalescing hint: `a` and `b` should get the same colour.
    fn coalesce(&mut self, a: &str, b: &str) {
        let ia = self.get_or_create(a);
        self.nodes[ia].coalesce_group.push(b.to_string());
    }

    /// Build the interference graph from a list of selected blocks.
    pub fn from_blocks(blocks: &[SelectedBlock]) -> Self {
        let mut graph = InterferenceGraph::new();

        for block in blocks {
            // Collect all value names defined in this block.
            let mut defs_in_block: Vec<String> = Vec::new();
            for op in &block.ops {
                // Collect any register names (SSA values) referenced.
                let used = op_uses(op);
                let defined = op_defines(op);

                // Every use interferes with every other live definition.
                for u in &used {
                    for d in &defs_in_block {
                        if u != d {
                            graph.interfere(u, d);
                        }
                    }
                }

                // The defined value interferes with all currently-live defs.
                if let Some(d) = &defined {
                    for other in &defs_in_block {
                        if d != other {
                            graph.interfere(d, other);
                        }
                    }
                    defs_in_block.push(d.clone());
                }

                // Coalescing: if this is a MovReg, source and dest can coalesce.
                if let A64Op::MovReg { rd, rm } = op {
                    graph.coalesce(rd, rm);
                }
            }
        }

        graph
    }

    /// Assign physical registers via Chaitin-Briggs.
    /// Returns a map from value name → physical register name.
    pub fn colour(&mut self) -> HashMap<String, String> {
        let mut result: HashMap<String, String> = HashMap::new();

        // ---- Phase 1: Simplify ----
        let mut stack: Vec<usize> = Vec::new();
        let mut remaining: HashSet<usize> = (0..self.nodes.len()).collect();

        loop {
            // Find a node with degree < num_colours.
            let low_deg = remaining
                .iter()
                .find(|&&idx| self.nodes[idx].degree < self.num_colours && !self.nodes[idx].pre_coloured);

            match low_deg {
                Some(&idx) => {
                    stack.push(idx);
                    remaining.remove(&idx);
                    // Remove its interference edges (decrement neighbours' degrees).
                    for &nbr in &self.nodes[idx].interference.clone() {
                        if remaining.contains(&nbr) {
                            self.nodes[nbr].degree =
                                self.nodes[nbr].degree.saturating_sub(1);
                        }
                    }
                }
                None => {
                    if remaining.is_empty() {
                        break;
                    }
                    // Spill candidate: node with highest degree.
                    let spill = remaining
                        .iter()
                        .max_by_key(|&&idx| self.nodes[idx].degree)
                        .copied()
                        .unwrap();
                    stack.push(spill);
                    remaining.remove(&spill);
                    for &nbr in &self.nodes[spill].interference.clone() {
                        if remaining.contains(&nbr) {
                            self.nodes[nbr].degree =
                                self.nodes[nbr].degree.saturating_sub(1);
                        }
                    }
                }
            }
        }

        // ---- Phase 2: Select (assign colours in reverse order) ----
        let mut assigned: HashMap<usize, usize> = HashMap::new();

        for &idx in stack.iter().rev() {
            let mut used: HashSet<usize> = HashSet::new();
            for &nbr in &self.nodes[idx].interference {
                if let Some(&c) = assigned.get(&nbr) {
                    used.insert(c);
                }
            }

            // Try to pick a colour that's not used by neighbours.
            let mut colour: Option<usize> = None;
            for c in 0..self.num_colours {
                if !used.contains(&c) {
                    colour = Some(c);
                    break;
                }
            }

            if let Some(c) = colour {
                assigned.insert(idx, c);
                self.nodes[idx].colour = Some(c);
                let reg_name = self.phys_regs[c].to_string();
                result.insert(self.nodes[idx].name.clone(), reg_name);
            } else {
                // All colours taken — spill to a stack slot.
                let slot = format!("%spill_{}", self.nodes[idx].name);
                result.insert(self.nodes[idx].name.clone(), slot);
            }
        }

        result
    }

    /// Attempt coalescing: for each node, look at its coalesce group and
    /// try to assign adjacent colours (for ldp/stp).
    pub fn coalesce_registers(&mut self, assignment: &mut HashMap<String, String>) {
        for i in 0..self.nodes.len() {
            if self.nodes[i].colour.is_none() {
                continue;
            }
            let base_colour = self.nodes[i].colour.unwrap();

            for mate_name in &self.nodes[i].coalesce_group.clone() {
                if let Some(&mate_idx) = self.name_to_idx.get(mate_name) {
                    if self.nodes[mate_idx].colour.is_some() {
                        continue; // already coloured
                    }

                    // Try the same colour first; if that's taken, try an
                    // adjacent colour (for ldp/stp pair formation).
                    for offset in [0, 1, -1, 2, -2] {
                        let candidate = (base_colour as isize + offset) as usize
                            % self.num_colours;

                        let mut conflict = false;
                        for &nbr in &self.nodes[mate_idx].interference {
                            if let Some(nc) = self.nodes[nbr].colour {
                                if nc == candidate {
                                    conflict = true;
                                    break;
                                }
                            }
                        }

                        if !conflict {
                            self.nodes[mate_idx].colour = Some(candidate);
                            let reg_name = self.phys_regs[candidate].to_string();
                            assignment.insert(mate_name.clone(), reg_name);
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// Collect register names used by an A64Op.
fn op_uses(op: &A64Op) -> Vec<String> {
    match op {
        A64Op::Add { rn, rm, .. } | A64Op::Sub { rn, rm, .. } | A64Op::And { rn, rm, .. }
        | A64Op::Orr { rn, rm, .. } | A64Op::Eor { rn, rm, .. } => {
            let mut v = vec![rn.clone()];
            v.push(rm.reg.clone());
            v
        }
        A64Op::AddImm { rn, .. } | A64Op::SubImm { rn, .. } | A64Op::CmpImm { rn, .. } => {
            vec![rn.clone()]
        }
        A64Op::Mul { rn, rm, .. } | A64Op::Sdiv { rn, rm, .. } => {
            vec![rn.clone(), rm.clone()]
        }
        A64Op::Lsl { rn, .. } | A64Op::Lsr { rn, .. } | A64Op::Asr { rn, .. } => {
            vec![rn.clone()]
        }
        A64Op::Cmp { rn, rm, .. } => {
            let mut v = vec![rn.clone()];
            v.push(rm.reg.clone());
            v
        }
        A64Op::Csel { rn, rm, .. } | A64Op::Csinc { rn, rm, .. } => {
            vec![rn.clone(), rm.clone()]
        }
        A64Op::MovReg { rm, .. } => vec![rm.clone()],
        A64Op::Blr { reg } => vec![reg.clone()],
        A64Op::Str { rs, addr, .. } | A64Op::StrFloat { rs, addr } => {
            let mut v = vec![rs.clone()];
            add_addr_uses(&mut v, addr);
            v
        }
        A64Op::Stp { rt1, rt2, addr, .. } => {
            let mut v = vec![rt1.clone(), rt2.clone()];
            add_addr_uses(&mut v, addr);
            v
        }
        A64Op::Strb { rs, addr } => {
            let mut v = vec![rs.clone()];
            add_addr_uses(&mut v, addr);
            v
        }
        A64Op::Ldr { addr, .. }
        | A64Op::Ldrb { addr, .. }
        | A64Op::Ldrsw { addr, .. }
        | A64Op::LdrFloat { addr, .. } => {
            let mut v = vec![];
            add_addr_uses(&mut v, addr);
            v
        }
        A64Op::Ldp { addr, .. } => {
            let mut v = vec![];
            add_addr_uses(&mut v, addr);
            v
        }
        A64Op::PrintI64Arg { reg } | A64Op::PrintStringArg { reg } => vec![reg.clone()],
        _ => vec![],
    }
}

/// Collect register names defined by an A64Op.
fn op_defines(op: &A64Op) -> Option<String> {
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
        | A64Op::Ldr { rd, .. }
        | A64Op::Ldrb { rd, .. }
        | A64Op::Ldrsw { rd, .. }
        | A64Op::LdrFloat { rd, .. }
        | A64Op::Movz { rd, .. }
        | A64Op::Movk { rd, .. }
        | A64Op::FAdd { rd, .. }
        | A64Op::FSub { rd, .. }
        | A64Op::FMul { rd, .. }
        | A64Op::FDiv { rd, .. }
        | A64Op::FMov { rd, .. }
        | A64Op::FMovImm { rd, .. } => Some(rd.clone()),

        A64Op::Ldp { rt1, .. } => {
            // LDP defines two registers; we return the first for
            // interference tracking, and handle the second separately.
            Some(rt1.clone())
        }
        _ => None,
    }
}

fn add_addr_uses(uses: &mut Vec<String>, addr: &AddressingMode) {
    match addr {
        AddressingMode::Base(r)
        | AddressingMode::BaseOffset(r, _)
        | AddressingMode::PreIndexed(r, _)
        | AddressingMode::PostIndexed(r, _) => {
            uses.push(r.clone());
        }
        AddressingMode::RegisterOffset { base, index, .. } => {
            uses.push(base.clone());
            uses.push(index.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// LDP / STP coalescing pass
// ---------------------------------------------------------------------------

/// Scan the ops list for pairs of adjacent loads/stores that can be
/// merged into `LDP` / `STP`.  Runs after register allocation so that
/// physical register names are known.
pub fn coalesce_ldp_stp(ops: &mut Vec<A64Op>) {
    let mut i = 0;
    while i + 1 < ops.len() {
        let merged = try_merge_pair(&ops[i], &ops[i + 1]);
        if let Some(merged_op) = merged {
            ops[i] = merged_op;
            ops.remove(i + 1);
        }
        i += 1;
    }
}

fn try_merge_pair(first: &A64Op, second: &A64Op) -> Option<A64Op> {
    // Two consecutive LDRs to the same base with consecutive offsets
    // can become LDP iff the destination registers are distinct.
    match (first, second) {
        (
            A64Op::Ldr {
                rd: r1,
                addr: AddressingMode::BaseOffset(b1, o1),
                ..
            },
            A64Op::Ldr {
                rd: r2,
                addr: AddressingMode::BaseOffset(b2, o2),
                ..
            },
        ) if b1 == b2 && *o1 + 8 == *o2 && r1 != r2 => {
            Some(A64Op::Ldp {
                rt1: r1.clone(),
                rt2: r2.clone(),
                addr: AddressingMode::BaseOffset(b1.clone(), *o1),
                ty: "i64".to_string(),
            })
        }
        // Two consecutive STRs to the same base with consecutive offsets.
        (
            A64Op::Str {
                rs: r1,
                addr: AddressingMode::BaseOffset(b1, o1),
                ..
            },
            A64Op::Str {
                rs: r2,
                addr: AddressingMode::BaseOffset(b2, o2),
                ..
            },
        ) if b1 == b2 && *o1 + 8 == *o2 && r1 != r2 => {
            Some(A64Op::Stp {
                rt1: r1.clone(),
                rt2: r2.clone(),
                addr: AddressingMode::BaseOffset(b1.clone(), *o1),
                ty: "i64".to_string(),
            })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Main entry: run register allocation on all functions
// ---------------------------------------------------------------------------

/// Assign physical registers and return updated functions.
pub fn allocate_registers(functions: &mut [SelectedFunction]) {
    for func in functions {
        // Build interference graph.
        let mut graph = InterferenceGraph::from_blocks(&func.blocks);

        // Colour the graph (Chaitin-Briggs).
        let mut assignment = graph.colour();

        // Coalesce (adjacent colour assignment for ldp/stp).
        graph.coalesce_registers(&mut assignment);

        // Pre-colour physical registers: every name that looks like a physical
        // ARM64 register must map to itself so the allocator never reassigns them.
        for (name, _) in assignment.clone().iter() {
            if is_phys_reg(name) {
                assignment.insert(name.clone(), name.clone());
            }
        }

        // Rewrite every op to use physical register names.
        for block in &mut func.blocks {
            for op in &mut block.ops {
                rewrite_op(op, &assignment);
            }

            // Try to form LDP / STP pairs.
            coalesce_ldp_stp(&mut block.ops);
        }

        // Collect which callee-saved registers (x19-x28) are used in the function.
        let callee_saved = ["x19", "x20", "x21", "x22", "x23", "x24", "x25", "x26", "x27", "x28"];
        let mut used: Vec<String> = Vec::new();
        for block in &func.blocks {
            for op in &block.ops {
                let regs = all_regs_in_op(op);
                for r in regs {
                    if callee_saved.contains(&r.as_str()) && !used.contains(&r) {
                        used.push(r);
                    }
                }
            }
        }
        // Sort by register number for deterministic stp pairing.
        used.sort_by(|a, b| {
            let an: u32 = a[1..].parse().unwrap_or(0);
            let bn: u32 = b[1..].parse().unwrap_or(0);
            an.cmp(&bn)
        });
        func.used_callee_saved = used;
    }
}

/// Collect all register names referenced in an A64Op.
fn all_regs_in_op(op: &A64Op) -> Vec<String> {
    // Use a combined list from op_uses and op_defines.
    let mut regs = op_uses(op);
    if let Some(d) = op_defines(op) {
        regs.push(d);
    }
    regs
}

/// Check if a name looks like a physical ARM64 register (x0-x30, sp, xzr, etc.).
fn is_phys_reg(name: &str) -> bool {
    if name.starts_with('x') {
        if let Ok(n) = name[1..].parse::<u32>() {
            return n <= 30;
        }
    }
    matches!(name, "sp" | "xzr" | "fp" | "lr" | "wzr")
}

/// Replace virtual register names in an A64Op with physical ones.
fn rewrite_op(op: &mut A64Op, assignment: &HashMap<String, String>) {
    // Counter for fresh spill temporaries.
    use std::sync::atomic::{AtomicU64, Ordering};
    static SPILL_TMP: AtomicU64 = AtomicU64::new(0);

    let phys = |name: &str| -> String {
        if let Some(p) = assignment.get(name) {
            return p.clone();
        }
        // Unallocated virtual register — use a temp register.
        if name.starts_with('%') {
            let n = SPILL_TMP.fetch_add(1, Ordering::Relaxed);
            format!("x{}", 9 + (n % 19)) // x9–x28 as spill temps
        } else {
            name.to_string()
        }
    };

    let rewrite_addr = |addr: &mut AddressingMode| {
        match addr {
            AddressingMode::Base(r)
            | AddressingMode::BaseOffset(r, _)
            | AddressingMode::PreIndexed(r, _)
            | AddressingMode::PostIndexed(r, _) => {
                *r = phys(r);
            }
            AddressingMode::RegisterOffset { base, index, .. } => {
                *base = phys(base);
                *index = phys(index);
            }
        }
    };

    match op {
        A64Op::Add { rd, rn, rm, .. }
        | A64Op::Sub { rd, rn, rm, .. }
        | A64Op::And { rd, rn, rm, .. }
        | A64Op::Orr { rd, rn, rm, .. }
        | A64Op::Eor { rd, rn, rm, .. } => {
            *rd = phys(rd);
            *rn = phys(rn);
            rm.reg = phys(&rm.reg);
        }
        A64Op::Mul { rd, rn, rm, .. } | A64Op::Sdiv { rd, rn, rm, .. } => {
            *rd = phys(rd);
            *rn = phys(rn);
            *rm = phys(rm);
        }
        A64Op::Lsl { rd, rn, .. }
        | A64Op::Lsr { rd, rn, .. }
        | A64Op::Asr { rd, rn, .. }
        | A64Op::AddImm { rd, rn, .. }
        | A64Op::SubImm { rd, rn, .. } => {
            *rd = phys(rd);
            *rn = phys(rn);
        }
        A64Op::CmpImm { rn, .. } => { *rn = phys(rn); }
        A64Op::MovImm { rd, .. } | A64Op::Movz { rd, .. } | A64Op::Movk { rd, .. } => {
            *rd = phys(rd);
        }
        A64Op::MovReg { rd, rm } => { *rd = phys(rd); *rm = phys(rm); }
        A64Op::Cmp { rn, rm } => { *rn = phys(rn); rm.reg = phys(&rm.reg); }
        A64Op::Csel { rd, rn, rm, .. } | A64Op::Csinc { rd, rn, rm, .. } => {
            *rd = phys(rd); *rn = phys(rn); *rm = phys(rm);
        }
        A64Op::Cset { rd, .. } => { *rd = phys(rd); }
        A64Op::Ldr { rd, addr, .. } | A64Op::Ldrb { rd, addr, .. }
        | A64Op::Ldrsw { rd, addr, .. } | A64Op::LdrFloat { rd, addr, .. } => {
            *rd = phys(rd); rewrite_addr(addr);
        }
        A64Op::Str { rs, addr, .. } | A64Op::StrFloat { rs, addr, .. }
        | A64Op::Strb { rs, addr, .. } => {
            *rs = phys(rs); rewrite_addr(addr);
        }
        A64Op::Ldp { rt1, rt2, addr, .. } | A64Op::Stp { rt1, rt2, addr, .. } => {
            *rt1 = phys(rt1); *rt2 = phys(rt2); rewrite_addr(addr);
        }
        A64Op::Blr { reg } => { *reg = phys(reg); }
        A64Op::FAdd { rd, rn, rm, .. } | A64Op::FSub { rd, rn, rm, .. }
        | A64Op::FMul { rd, rn, rm, .. } | A64Op::FDiv { rd, rn, rm, .. } => {
            *rd = phys(rd); *rn = phys(rn); *rm = phys(rm);
        }
        A64Op::FCmp { rn, rm } => { *rn = phys(rn); *rm = phys(rm); }
        A64Op::FMov { rd, rm } => { *rd = phys(rd); *rm = phys(rm); }
        A64Op::FMovImm { rd, .. } => { *rd = phys(rd); }
        A64Op::PrintI64Arg { reg } | A64Op::PrintStringArg { reg } => {
            *reg = phys(reg);
        }
        A64Op::B { .. } | A64Op::BCond { .. } | A64Op::Bl { .. }
        | A64Op::Ret | A64Op::StpFrame | A64Op::LdpFrame
        | A64Op::Prologue { .. } | A64Op::Epilogue => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interference_two_ops_different_regs() {
        let blocks = vec![SelectedBlock {
            label: "entry".into(),
            ops: vec![
                A64Op::AddImm {
                    rd: "v0".into(),
                    rn: "x0".into(),
                    imm: 1,
                    ty: "i64".into(),
                },
                A64Op::AddImm {
                    rd: "v1".into(),
                    rn: "v0".into(),
                    imm: 2,
                    ty: "i64".into(),
                },
            ],
        }];

        let mut graph = InterferenceGraph::from_blocks(&blocks);
        let mut assignment = graph.colour();
        graph.coalesce_registers(&mut assignment);

        assert!(assignment.contains_key("v0"));
        assert!(assignment.contains_key("v1"));
        // Both should get different physical registers.
        assert_ne!(assignment["v0"], assignment["v1"]);
    }

    #[test]
    fn ldp_pair_formation() {
        let mut ops = vec![
            A64Op::Ldr {
                rd: "x0".into(),
                addr: AddressingMode::BaseOffset("sp".into(), 0),
                ty: "i64".into(),
            },
            A64Op::Ldr {
                rd: "x1".into(),
                addr: AddressingMode::BaseOffset("sp".into(), 8),
                ty: "i64".into(),
            },
        ];
        coalesce_ldp_stp(&mut ops);
        assert_eq!(ops.len(), 1, "two ldrs should merge into one ldp");
        assert!(matches!(ops[0], A64Op::Ldp { .. }));
    }

    #[test]
    fn non_consecutive_offsets_do_not_merge() {
        let mut ops = vec![
            A64Op::Ldr {
                rd: "x0".into(),
                addr: AddressingMode::BaseOffset("sp".into(), 0),
                ty: "i64".into(),
            },
            A64Op::Ldr {
                rd: "x1".into(),
                addr: AddressingMode::BaseOffset("sp".into(), 16), // gap
                ty: "i64".into(),
            },
        ];
        coalesce_ldp_stp(&mut ops);
        assert_eq!(ops.len(), 2, "non-consecutive offsets should not merge");
    }
}
