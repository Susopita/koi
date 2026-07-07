//! ARM64 instruction selection via Maximal Munch over the IR.
//!
//! This pass walks each [`Instruction`] in a [`BasicBlock`] and emits
//! [`A64Op`] sequences, applying two key optimisations:
//!
//! ## Barrel shifter folding (Maximal Munch)
//!
//! When a `BinOp` feeds another ALU instruction as its operand, the inner
//! operation can be folded into the outer instruction's shifted-register
//! form — e.g. `add r0, r1, r2, lsl #3` instead of `lsl r2, r3, #3` +
//! `add r0, r1, r2`.
//!
//! ## If-conversion
//!
//! A pattern of `Branch` → two simple blocks → `Phi` is collapsed into a
//! `csel` / `csinc` conditional-select instruction, removing the branch
//! and linearising the control flow.

use crate::middle_end::ir::{BasicBlock, IRFunction, IRProgram, Instruction};

// ---------------------------------------------------------------------------
// ARM64 ALU / addressing operand descriptors
// ---------------------------------------------------------------------------

/// Shift/extend applied to a register operand in an ALU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftKind {
    Lsl,
    Lsr,
    Asr,
    /// Zero-extend (byte, halfword, word).  Used for mixing 32/64-bit
    /// operands without an explicit zero-extension instruction.
    Uxtb,
    Uxth,
    Uxtw,
}

/// A register that may be shifted or extended before use in an ALU op.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtendedReg {
    pub reg: String,
    pub shift: Option<(ShiftKind, u8)>,
}

impl ExtendedReg {
    pub fn plain(reg: &str) -> Self {
        ExtendedReg {
            reg: reg.to_string(),
            shift: None,
        }
    }

    pub fn shifted(reg: &str, kind: ShiftKind, amount: u8) -> Self {
        ExtendedReg {
            reg: reg.to_string(),
            shift: Some((kind, amount)),
        }
    }
}

/// ARM64 addressing mode for load/store.
#[derive(Debug, Clone, PartialEq)]
pub enum AddressingMode {
    /// Base register only: `[Xn]`
    Base(String),
    /// Base + unsigned immediate (scaled by access size): `[Xn, #imm]`
    BaseOffset(String, i64),
    /// Base + register (optionally shifted): `[Xn, Xm, lsl #N]`
    RegisterOffset {
        base: String,
        index: String,
        shift: Option<(ShiftKind, u8)>,
    },
    /// Pre-indexed: `[Xn, #imm]!`  (update base before access).
    PreIndexed(String, i64),
    /// Post-indexed: `[Xn], #imm`  (update base after access).
    PostIndexed(String, i64),
}

impl AddressingMode {
    /// Try to match a load from `[base + offset]` or `[base + index]`.
    pub fn from_addressing(base: &str, offset: &str, offset_is_shift: bool) -> Self {
        // If the offset is a small immediate, fold it directly.
        if let Ok(imm) = offset.parse::<i64>() {
            if (-256..=256).contains(&imm) {
                return AddressingMode::BaseOffset(base.to_string(), imm);
            }
        }
        // Register offset with optional shift.
        if offset_is_shift {
            AddressingMode::RegisterOffset {
                base: base.to_string(),
                index: offset.to_string(),
                shift: Some((ShiftKind::Lsl, 0)),
            }
        } else {
            AddressingMode::RegisterOffset {
                base: base.to_string(),
                index: offset.to_string(),
                shift: None,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ARM64 operations emitted by the instruction selector
// ---------------------------------------------------------------------------

/// One ARM64 machine instruction after selection.
#[derive(Debug, Clone, PartialEq)]
pub enum A64Op {
    // -- ALU (register) ----------------------------------------------------
    /// `add Rd, Rn, Rm{, shift #N}`
    Add {
        rd: String,
        rn: String,
        rm: ExtendedReg,
        ty: String,
    },
    /// `sub Rd, Rn, Rm{, shift #N}`
    Sub {
        rd: String,
        rn: String,
        rm: ExtendedReg,
        ty: String,
    },
    /// `mul Rd, Rn, Rm`
    Mul { rd: String, rn: String, rm: String, ty: String },
    /// `sdiv Rd, Rn, Rm`
    Sdiv { rd: String, rn: String, rm: String, ty: String },
    /// `and Rd, Rn, Rm{, shift #N}`
    And {
        rd: String,
        rn: String,
        rm: ExtendedReg,
        ty: String,
    },
    /// `orr Rd, Rn, Rm{, shift #N}`
    Orr {
        rd: String,
        rn: String,
        rm: ExtendedReg,
        ty: String,
    },
    /// `eor Rd, Rn, Rm{, shift #N}` (xor)
    Eor {
        rd: String,
        rn: String,
        rm: ExtendedReg,
        ty: String,
    },
    /// `lsl Rd, Rn, #amount` (logical shift left)
    Lsl { rd: String, rn: String, amount: u8, ty: String },
    /// `lsr Rd, Rn, #amount`
    Lsr { rd: String, rn: String, amount: u8, ty: String },
    /// `asr Rd, Rn, #amount` (arithmetic shift right)
    Asr { rd: String, rn: String, amount: u8, ty: String },

    // -- ALU (immediate) ---------------------------------------------------
    AddImm { rd: String, rn: String, imm: i64, ty: String },
    SubImm { rd: String, rn: String, imm: i64, ty: String },
    MovImm { rd: String, imm: i64, ty: String },
    MovReg { rd: String, rm: String },

    // -- Comparisons -------------------------------------------------------
    /// `cmp Rn, Rm{, shift #N}` (sets NZCV)
    Cmp { rn: String, rm: ExtendedReg },
    /// `cmp Rn, #imm`
    CmpImm { rn: String, imm: i64 },

    // -- Conditional select ------------------------------------------------
    /// `csel Rd, Rn, Rm, cond`
    Csel {
        rd: String,
        rn: String,
        rm: String,
        cond: String,
        ty: String,
    },
    /// `csinc Rd, Rn, Rm, cond` — conditional select + increment
    Csinc {
        rd: String,
        rn: String,
        rm: String,
        cond: String,
        ty: String,
    },
    /// `cset Rd, cond` — set register to 0/1 based on condition
    Cset { rd: String, cond: String, ty: String },

    // -- Load / Store ------------------------------------------------------
    /// `ldr x0, [addr]`
    Ldr { rd: String, addr: AddressingMode, ty: String },
    /// `str x0, [addr]`
    Str { rs: String, addr: AddressingMode, ty: String },
    /// `ldrb` / `strb` (byte)
    Ldrb { rd: String, addr: AddressingMode },
    Strb { rs: String, addr: AddressingMode },
    /// `ldrsw` (sign-extend word to 64-bit)
    Ldrsw { rd: String, addr: AddressingMode },
    /// `ldp Rt1, Rt2, [addr]` — load pair (coalesced adjacent loads).
    Ldp {
        rt1: String,
        rt2: String,
        addr: AddressingMode,
        ty: String,
    },
    /// `stp Rt1, Rt2, [addr]` — store pair (coalesced adjacent stores).
    Stp {
        rt1: String,
        rt2: String,
        addr: AddressingMode,
        ty: String,
    },

    // -- Branch & call -----------------------------------------------------
    /// `b label`
    B { label: String },
    /// `b.cond label`
    BCond { cond: String, label: String },
    /// `bl label`
    Bl { label: String },
    /// `blr Xn`
    Blr { reg: String },
    /// Load a user function's address into a register (ADRP+ADD).
    LoadFuncAddr { rd: String, name: String },
    /// `ret`
    Ret,

    // -- Stack frame -------------------------------------------------------
    /// `stp x29, x30, [sp, #-16]!`
    StpFrame,
    /// `ldp x29, x30, [sp], #16`
    LdpFrame,
    /// `sub sp, sp, #frame_size`
    Prologue { frame_size: i64 },
    Epilogue,

    // -- Move wide (materialise large immediates) --------------------------
    /// `movz Rd, #imm{, lsl #N}`
    Movz { rd: String, imm: u16, shift: u8 },
    /// `movk Rd, #imm{, lsl #N}`
    Movk { rd: String, imm: u16, shift: u8 },

    // -- Data processing (floating-point) ----------------------------------
    FAdd { rd: String, rn: String, rm: String },
    FSub { rd: String, rn: String, rm: String },
    FMul { rd: String, rn: String, rm: String },
    FDiv { rd: String, rn: String, rm: String },
    /// `fcmp Rn, Rm`
    FCmp { rn: String, rm: String },
    /// `fmov Rd, Rm`
    FMov { rd: String, rm: String },
    /// `fmov Rd, #imm` (single-precision immediate)
    FMovImm { rd: String, imm: f64 },
    LdrFloat { rd: String, addr: AddressingMode },
    StrFloat { rs: String, addr: AddressingMode },
    /// Print an i64 value via printf (pseudo-op, handled at emission).
    /// `reg` is the virtual register holding the value to print.
    PrintI64Arg { reg: String },
    PrintStringArg { reg: String },
    /// Print a f64 value via printf (pseudo-op, handled at emission).
    /// `reg` is the virtual register (GPR) holding the float bits.
    PrintF64Arg { reg: String },
    /// Load a string literal's address into a register.
    LoadString { rd: String, text: String },
    /// Load a 64-bit float literal into dN from the literal pool.
    /// `bits` is the f64 bit pattern (to_bits).
    LoadFloat { rd: String, bits: u64 },
    /// Address-of pseudo-op: resolved after register allocation.
    /// `rd` receives the stack address of `rn`.
    AddrOf { rd: String, rn: String },
}

// ---------------------------------------------------------------------------
// Instruction selector
// ---------------------------------------------------------------------------

/// Holds the result of selection: a flat list of `A64Op`s per block.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedFunction {
    pub name: String,
    pub blocks: Vec<SelectedBlock>,
    pub frame_size: i64,
    /// Names of function parameters (for ABI prologue moves).
    pub parameters: Vec<String>,
    /// Callee-saved registers used by this function (x19-x28).
    pub used_callee_saved: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectedBlock {
    pub label: String,
    pub ops: Vec<A64Op>,
}

/// Tracks struct field offsets across the program.
/// Assigned sequentially per (struct_type, field_name) in order of first appearance.
#[derive(Debug, Clone, Default)]
pub struct Layouts {
    field_offsets: std::collections::BTreeMap<(String, String), i64>,
    struct_sizes: std::collections::BTreeMap<String, i64>,
}

impl Layouts {
    pub fn from_program(program: &IRProgram) -> Self {
        let mut layouts = Layouts::default();
        // Collect value types to know which types are structs.
        let mut value_types: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        for function in &program.functions {
            for (name, ty) in &function.parameters {
                value_types.insert(name.clone(), ty.clone());
            }
            for block in &function.blocks {
                for instr in &block.instructions {
                    if let (Some(result), Some(ty)) = (instr.result_name(), instr.result_type()) {
                        value_types.insert(result.to_string(), ty.to_string());
                    }
                }
            }
        }

        let record_field = |layouts: &mut Layouts, object_ty: &str, field: &str| {
            if is_scalar_type(object_ty) {
                return;
            }
            let key = (object_ty.to_string(), field.to_string());
            if !layouts.field_offsets.contains_key(&key) {
                let next_index = layouts
                    .field_offsets
                    .keys()
                    .filter(|(s, _)| s == object_ty)
                    .count() as i64;
                layouts.field_offsets.insert(key, next_index * 8);
                layouts
                    .struct_sizes
                    .insert(object_ty.to_string(), (next_index + 1) * 8);
            }
        };

        for function in &program.functions {
            for block in &function.blocks {
                for instr in &block.instructions {
                    match instr {
                        Instruction::GetField { object, field, .. } => {
                            if let Some(obj_ty) = value_types.get(object) {
                                record_field(&mut layouts, obj_ty, field);
                            }
                        }
                        Instruction::SetField { object, field, .. } => {
                            if let Some(obj_ty) = value_types.get(object) {
                                record_field(&mut layouts, obj_ty, field);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        layouts
    }

    pub fn field_offset(&self, struct_name: &str, field: &str) -> i64 {
        self.field_offsets
            .get(&(struct_name.to_string(), field.to_string()))
            .copied()
            .unwrap_or(0)
    }

    pub fn allocation_size(&self, ty: &str) -> i64 {
        if let Some(size) = self.struct_sizes.get(ty) {
            return (*size).max(8);
        }
        if ty.starts_with("arr_") {
            return 64;
        }
        8
    }
}

fn is_scalar_type(ty: &str) -> bool {
    matches!(ty, "i64" | "f64" | "bool" | "string") || ty.starts_with("ptr_") || ty.starts_with("fn_")
}

/// Main entry: select ARM64 instructions for a whole IR program.
pub fn select_instructions(program: &IRProgram) -> Vec<SelectedFunction> {
    let user_fn_names: std::collections::BTreeSet<String> =
        program.functions.iter().map(|f| f.name.clone()).collect();
    let layouts = Layouts::from_program(program);
    program.functions.iter().map(|func| select_function(func, program, &user_fn_names, &layouts)).collect()
}

// ---------------------------------------------------------------------------
// Phi lowering (edge moves)
// ---------------------------------------------------------------------------

/// One edge move: copy `src` → `dst` along a control-flow edge.
struct EdgeMove {
    src: String,
    dst: String,
}

/// Build edge moves from Phi instructions in the IR.
/// Returns a map: (predecessor_label, successor_label) → [EdgeMove, ...].
fn build_phi_moves(
    blocks: &[crate::middle_end::ir::BasicBlock],
) -> std::collections::BTreeMap<(String, String), Vec<EdgeMove>> {
    let mut moves: std::collections::BTreeMap<(String, String), Vec<EdgeMove>> =
        std::collections::BTreeMap::new();
    for block in blocks {
        for instr in &block.instructions {
            if let Instruction::Phi {
                result,
                incoming,
                ..
            } = instr
            {
                for (pred, value) in incoming {
                    moves
                        .entry((pred.clone(), block.label.clone()))
                        .or_default()
                        .push(EdgeMove {
                            src: value.clone(),
                            dst: result.clone(),
                        });
                }
            }
        }
    }
    moves
}

/// Insert edge moves (phi copies) into the selected block ops.
fn insert_edge_moves(
    blocks: &mut Vec<SelectedBlock>,
    phi_moves: &std::collections::BTreeMap<(String, String), Vec<EdgeMove>>,
) {
    let mut i = 0;
    while i < blocks.len() {
        let block_label = blocks[i].label.clone();
        // Find the last terminator instruction.
        let term_idx = blocks[i]
            .ops
            .iter()
            .rposition(|op| {
                matches!(op, A64Op::B { .. } | A64Op::BCond { .. } | A64Op::Ret)
            });

        let Some(ti) = term_idx else { i += 1; continue };

        // Extract ALL data before any mutation.
        let term_info = match &blocks[i].ops[ti] {
            A64Op::Ret => TerminatorInfo::Ret,
            A64Op::B { label } => TerminatorInfo::Jump(label.clone()),
            A64Op::BCond { label: true_label, .. } => {
                let false_label = blocks[i].ops.get(ti + 1).and_then(|op| {
                    if let A64Op::B { label } = op {
                        Some(label.clone())
                    } else {
                        None
                    }
                });
                TerminatorInfo::Branch(true_label.clone(), false_label)
            }
            _ => { i += 1; continue; }
        };

        match term_info {
            TerminatorInfo::Ret => {}
            TerminatorInfo::Jump(target) => {
                let key = (block_label, target.clone());
                if let Some(moves) = phi_moves.get(&key) {
                    let edge_ops: Vec<A64Op> = moves
                        .iter()
                        .filter(|m| m.src != m.dst)
                        .map(|m| A64Op::MovReg {
                            rd: m.dst.clone(),
                            rm: m.src.clone(),
                        })
                        .collect();
                    if !edge_ops.is_empty() {
                        let tail = blocks[i].ops.split_off(ti);
                        blocks[i].ops.extend(edge_ops);
                        blocks[i].ops.extend(tail);
                    }
                }
            }
            TerminatorInfo::Branch(true_label, false_label) => {
                let true_key = (block_label.clone(), true_label.clone());
                let false_key = false_label.clone().map(|fl| (block_label.clone(), fl));

                let true_moves = phi_moves.get(&true_key);
                let false_moves = false_key.as_ref().and_then(|k| phi_moves.get(k));

                if true_moves.is_none() && false_moves.is_none() {
                    i += 1;
                    continue;
                }

                let true_ops: Vec<A64Op> = true_moves
                    .map(|m| {
                        m.iter()
                            .filter(|em| em.src != em.dst)
                            .map(|em| A64Op::MovReg {
                                rd: em.dst.clone(),
                                rm: em.src.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let false_ops: Vec<A64Op> = false_moves
                    .map(|m| {
                        m.iter()
                            .filter(|em| em.src != em.dst)
                            .map(|em| A64Op::MovReg {
                                rd: em.dst.clone(),
                                rm: em.src.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let true_label_name = format!("{}_{}_phi_true", block_label, true_label);
                let false_label_name = format!("{}_{}_phi_false", block_label, false_label.clone().unwrap_or_default());

                blocks[i].ops[ti] = A64Op::BCond {
                    cond: "ne".to_string(),
                    label: true_label_name.clone(),
                };
                if let Some(ref fl) = false_label {
                    blocks[i].ops[ti + 1] = A64Op::B {
                        label: false_label_name.clone(),
                    };
                }

                let mut true_block_ops = Vec::new();
                true_block_ops.extend(true_ops);
                true_block_ops.push(A64Op::B { label: true_label.clone() });
                let mut false_block_ops: Vec<A64Op> = Vec::new();
                false_block_ops.extend(false_ops);
                if let Some(ref fl) = false_label {
                    false_block_ops.push(A64Op::B { label: fl.clone() });
                }

                blocks.insert(i + 1, SelectedBlock { label: true_label_name, ops: true_block_ops });
                blocks.insert(i + 2, SelectedBlock { label: false_label_name, ops: false_block_ops });
                i += 3;
                continue;
            }
        }
        i += 1;
    }
}

enum TerminatorInfo {
    Ret,
    Jump(String),
    Branch(String, Option<String>),
}

fn select_function(
    func: &IRFunction,
    _program: &IRProgram,
    user_fn_names: &std::collections::BTreeSet<String>,
    layouts: &Layouts,
) -> SelectedFunction {
    // Intentar if-conversion primero (transforma Branch→bloques→Phi
    // en CSEL/CSINC, eliminando el branch y linearizando el flujo).
    if let Some(mut converted) = try_if_conversion_function(func, user_fn_names, layouts) {
        // Build phi moves from IR and insert edge copies for remaining phis.
        let phi_moves = build_phi_moves(&func.blocks);
        insert_edge_moves(&mut converted, &phi_moves);
        // Add parameter MovReg/FMov setup to the entry block.
        if let Some(entry) = converted.first_mut() {
            let mut prologue_ops: Vec<A64Op> = Vec::new();
            let mut int_idx = 0usize;
            let mut float_idx = 0usize;
            for (param_name, param_ty) in &func.parameters {
                if param_ty == "f64" {
                    if let Some(reg) = crate::backend::arm64::abi::AArch64ABI::float_arg_register(float_idx) {
                        prologue_ops.push(A64Op::FMov { rd: param_name.clone(), rm: reg.to_string() });
                        float_idx += 1;
                    }
                } else {
                    if let Some(arg_reg) = crate::backend::arm64::abi::AArch64ABI::arg_register(int_idx) {
                        prologue_ops.push(A64Op::MovReg { rd: param_name.clone(), rm: arg_reg.to_string() });
                        int_idx += 1;
                    }
                }
            }
            prologue_ops.extend(std::mem::take(&mut entry.ops));
            entry.ops = prologue_ops;
        }
        let frame_size = 0;
        let parameters = func.parameters.iter().map(|(n, _)| n.clone()).collect();
        return SelectedFunction {
            name: func.name.clone(),
            blocks: converted,
            frame_size,
            parameters,
            used_callee_saved: Vec::new(),
        };
    }

    // Fallback: selección normal per-bloque
    let mut blocks: Vec<SelectedBlock> = func
        .blocks
        .iter()
        .map(|block| select_block(block, func, user_fn_names, layouts))
        .collect();

    // Emit MovReg ops in the entry block to copy from ABI arg registers to parameters.
    if let Some(entry) = blocks.first_mut() {
        let mut prologue_ops: Vec<A64Op> = Vec::new();
        let mut int_idx = 0usize;
        let mut float_idx = 0usize;
        for (param_name, param_ty) in &func.parameters {
            if param_ty == "f64" {
                if let Some(reg) = crate::backend::arm64::abi::AArch64ABI::float_arg_register(float_idx) {
                    prologue_ops.push(A64Op::FMov { rd: param_name.clone(), rm: reg.to_string() });
                    float_idx += 1;
                }
            } else {
                if let Some(arg_reg) = crate::backend::arm64::abi::AArch64ABI::arg_register(int_idx) {
                    prologue_ops.push(A64Op::MovReg { rd: param_name.clone(), rm: arg_reg.to_string() });
                    int_idx += 1;
                }
            }
        }
        // Insert prologue moves at the beginning of the entry block's ops.
        prologue_ops.extend(std::mem::take(&mut entry.ops));
        entry.ops = prologue_ops;
    }

    // Build phi moves from IR and insert edge copies.
    let phi_moves = build_phi_moves(&func.blocks);
    insert_edge_moves(&mut blocks, &phi_moves);

    let frame_size = 0; // Will be set by register allocator.
    let parameters = func.parameters.iter().map(|(n, _)| n.clone()).collect();

    SelectedFunction {
        name: func.name.clone(),
        blocks,
        frame_size,
        parameters,
        used_callee_saved: Vec::new(),
    }
}

fn select_block(
    block: &BasicBlock,
    _func: &IRFunction,
    user_fn_names: &std::collections::BTreeSet<String>,
    layouts: &Layouts,
) -> SelectedBlock {
    let mut ops: Vec<A64Op> = Vec::new();

    // Phase 1: try if-conversion on this block first (needs whole-block view).
    if let Some(if_conv_ops) = try_if_conversion(block) {
        return SelectedBlock {
            label: block.label.clone(),
            ops: if_conv_ops,
        };
    }

    // Phase 2: Build value_map (Const value → i64), shift_map
    // (shift-result value → (source_name, ShiftKind, amount)), and type_map
    // (value name → IR type string).  These let munch_instruction fold
    // barrel-shifter operands into ALU ops and emit type-specific print
    // pseudo-ops.
    let mut value_map: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut shift_map: std::collections::BTreeMap<String, (String, ShiftKind, u8)> =
        std::collections::BTreeMap::new();
    let mut type_map: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();

    // Seed type_map with parameter types (needed by GetField/SetField for struct field offset lookup).
    for (name, ty) in &_func.parameters {
        type_map.insert(name.clone(), ty.clone());
    }

    for instr in &block.instructions {
        match instr {
            Instruction::Const { result, value, ty } => {
                if let Some(n) = value.as_i64() {
                    value_map.insert(result.clone(), n);
                }
                type_map.insert(result.clone(), ty.clone());
            }
            Instruction::BinOp {
                result,
                lhs,
                rhs,
                op_type,
                ty,
                ..
            } => {
                let kind = match op_type.as_str() {
                    "<<" => Some(ShiftKind::Lsl),
                    ">>" => Some(ShiftKind::Lsr),
                    _ => None,
                };
                if let Some(kind) = kind {
                    if let Some(&amount) = value_map.get(rhs) {
                        if (1..64).contains(&amount) {
                            // Store (source_name, kind, amount) so that the
                            // consuming ALU op can use `source_name, lsl #N`.
                            shift_map.insert(result.clone(), (lhs.clone(), kind, amount as u8));
                        }
                    }
                }
                type_map.insert(result.clone(), ty.clone());
            }
            Instruction::Call { result, ty, .. } => {
                if let Some(r) = result {
                    if let Some(t) = ty {
                        type_map.insert(r.clone(), t.clone());
                    }
                }
            }
            _ => {}
        }
    }

    // Phase 3: Maximal Munch on each instruction.
    let mut i = 0;
    while i < block.instructions.len() {
        let instr = &block.instructions[i];
        let consumed =
            munch_instruction(instr, &block.instructions[i..], &mut ops, &shift_map, &value_map, &type_map, user_fn_names, layouts);
        i += consumed.max(1);
    }

    SelectedBlock {
        label: block.label.clone(),
        ops,
    }
}

// ---------------------------------------------------------------------------
// If-conversion
// ---------------------------------------------------------------------------

/// Pattern: this block ends with `Branch`, the two target blocks each contain
/// a single `Const` or a `Phi`-feeding instruction, and a later block merges
/// them with a `Phi`.  Collapse into `CSEL` / `CSINC`.
fn try_if_conversion(block: &BasicBlock) -> Option<Vec<A64Op>> {
    // Check that the block ends with a `Branch`.
    let (cond, true_label, false_label) = match block.instructions.last()? {
        Instruction::Branch {
            cond,
            true_label,
            false_label,
        } => (cond.clone(), true_label.clone(), false_label.clone()),
        _ => return None,
    };

    // We need the true/false destination to be simple side-effect-free
    // blocks that feed a subsequent `Phi`.  Since we only have this one
    // block, we emit a conservative `CSEL`-like pattern: compare, then
    // cset + branch + phi materialisation.
    //
    // Full if-conversion across blocks requires seeing the IR function's
    // full block list, which we can't do from a single block.
    // Return None and fall back to standard branch codegen.
    None
}

/// Perform if-conversion across an entire function's block list,
/// returning the replacement `A64Op`s if applicable.
/// Called from `select_function` before per-block selection.
pub fn try_if_conversion_function(
    func: &IRFunction,
    user_fn_names: &std::collections::BTreeSet<String>,
    layouts: &Layouts,
) -> Option<Vec<SelectedBlock>> {
    // Build a map from label → block for quick lookup.
    let block_map: std::collections::BTreeMap<&str, &BasicBlock> = func
        .blocks
        .iter()
        .map(|b| (b.label.as_str(), b))
        .collect();

    // Search for the pattern:
    //   block_i: Branch(cond, Ltrue, Lfalse)
    //   Ltrue:   Const/Phi-feeding insns, Jump(Lmerge)
    //   Lfalse:  Const/Phi-feeding insns, Jump(Lmerge)
    //   Lmerge:  Phi(%dst, [(Ltrue, v_true), (Lfalse, v_false)]) ...
    //
    // If found, replace with:
    //   block_i: cmp ..., csel %dst, v_true, v_false, cond

    let mut replacement: Option<Vec<SelectedBlock>> = None;

    for (i, block) in func.blocks.iter().enumerate() {
        let (cond, true_label, false_label) = match block.instructions.last()? {
            Instruction::Branch {
                cond,
                true_label,
                false_label,
            } => (cond, true_label, false_label),
            _ => continue,
        };

        let true_block = block_map.get(true_label.as_str())?;
        let false_block = block_map.get(false_label.as_str())?;

        // Both targets must end with a direct Jump (no other side effects).
        let true_jump = match true_block.instructions.last()? {
            Instruction::Jump { label } => label,
            _ => continue,
        };
        let false_jump = match false_block.instructions.last()? {
            Instruction::Jump { label } => label,
            _ => continue,
        };

        // Both must jump to the same merge block.
        if true_jump != false_jump {
            continue;
        }
        let merge_label = true_jump;

        // Find the merge block and its Phi instructions.
        let merge_block = block_map.get(merge_label.as_str())?;
        let phi_instrs: Vec<&Instruction> = merge_block
            .instructions
            .iter()
            .filter(|instr| matches!(instr, Instruction::Phi { .. }))
            .collect();

        if phi_instrs.is_empty() {
            continue;
        }

        // Verify that the true/false blocks have only side-effect-free
        // instructions before their final Jump.
        let true_body = &true_block.instructions[..true_block.instructions.len().saturating_sub(1)];
        let false_body = &false_block.instructions[..false_block.instructions.len().saturating_sub(1)];

        if !is_side_effect_free(true_body) || !is_side_effect_free(false_body) {
            continue;
        }

        // Verify that every Phi in the merge block takes values from
        // the Ltrue/Lfalse predecessor blocks.
        let all_phi_ok = phi_instrs.iter().all(|phi| match phi {
            Instruction::Phi { incoming, .. } => {
                let labels: Vec<&str> = incoming.iter().map(|(l, _)| l.as_str()).collect();
                labels.contains(&true_label.as_str())
                    && labels.contains(&false_label.as_str())
            }
            _ => unreachable!(),
        });

        if !all_phi_ok {
            continue;
        }

        // ---- Pattern matched!  Build the replacement. ----
        let mut ops: Vec<A64Op> = Vec::new();
        let cond_arm64 = ir_cond_to_arm64(cond);

        let empty_map: std::collections::BTreeMap<String, (String, ShiftKind, u8)> =
            std::collections::BTreeMap::new();
        let empty_valmap: std::collections::BTreeMap<String, i64> =
            std::collections::BTreeMap::new();
        let empty_typemap: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();

        // Emit the entry block's instructions (excluding the final Branch)
        // to ensure condition computation (e.g., BinOp for `<`) is emitted.
        let entry_body = &block.instructions[..block.instructions.len().saturating_sub(1)];
        for instr in entry_body {
            munch_instruction(instr, &[], &mut ops, &empty_map, &empty_valmap, &empty_typemap, user_fn_names, layouts);
        }

        // Emit the true/false body instructions so the values referenced by
        // the Phi/CSEL are defined (e.g. Const values 1 and 2 for a nested if).
        for instr in true_body {
            munch_instruction(instr, &[], &mut ops, &empty_map, &empty_valmap, &empty_typemap, user_fn_names, layouts);
        }
        for instr in false_body {
            munch_instruction(instr, &[], &mut ops, &empty_map, &empty_valmap, &empty_typemap, user_fn_names, layouts);
        }

        // Emit comparison for the condition value.
        ops.push(A64Op::CmpImm {
            rn: cond.clone(),
            imm: 0,
        });

        // For each phi, emit a csel.
        for phi in &phi_instrs {
            if let Instruction::Phi {
                result,
                incoming,
                ty,
            } = phi
            {
                // Find the value from true_label and false_label.
                let true_val = incoming
                    .iter()
                    .find(|(l, _)| l == true_label)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                let false_val = incoming
                    .iter()
                    .find(|(l, _)| l == false_label)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();

                ops.push(A64Op::Csel {
                    rd: result.clone(),
                    rn: true_val,
                    rm: false_val,
                    cond: cond_arm64.clone(),
                    ty: ty.clone(),
                });
            }
        }

        // Remaining non-phi instructions from the merge block.
        let non_phi: Vec<&Instruction> = merge_block
            .instructions
            .iter()
            .filter(|instr| !matches!(instr, Instruction::Phi { .. }))
            .collect();
        let empty_map = std::collections::BTreeMap::new();
        let empty_valmap = std::collections::BTreeMap::new();
        let empty_typemap = std::collections::BTreeMap::new();
        for instr in &non_phi {
            munch_instruction(instr, &[], &mut ops, &empty_map, &empty_valmap, &empty_typemap, user_fn_names, layouts);
        }

        let mut result = Vec::new();

        // Include all blocks before the matched branch block.
        for b in func.blocks.iter().take(i) {
            result.push(select_block(b, func, user_fn_names, layouts));
        }

        result.push(SelectedBlock {
            label: block.label.clone(),
            ops,
        });

        // Add the remaining blocks (after merge) unchanged.
        for b in func.blocks.iter().skip(i + 1).skip_while(|b| {
            b.label == *true_label || b.label == *false_label || b.label == *merge_label
        }) {
            result.push(select_block(b, func, user_fn_names, layouts));
        }

        replacement = Some(result);
        break; // only the first match
    }

    replacement
}

/// True if none of the instructions can have side effects (memory writes,
/// calls, returns, branches).
fn is_side_effect_free(instructions: &[Instruction]) -> bool {
    instructions.iter().all(|instr| {
        matches!(
            instr,
            Instruction::Const { .. }
                | Instruction::BinOp { .. }
                | Instruction::Phi { .. }
                | Instruction::Alloc { .. }
                | Instruction::AddrOf { .. }
                | Instruction::Deref { .. }
                | Instruction::GetField { .. }
                | Instruction::GetIndex { .. }
        )
    })
}

fn ir_cond_to_arm64(ir_cond: &str) -> String {
    // The IR condition comes from a comparison producing a bool.
    // ARM64 condition codes for when the comparison was `cmp #0`.
    // If the bool is the result of a `<`, the IR will have that as a
    // separate binop and the branch is on the bool.  Our `csel` uses
    // `ne` (not equal to zero) for truthy / `eq` for falsy.
    "ne".to_string()
}

// ---------------------------------------------------------------------------
// Maximal Munch — instruction selector core
// ---------------------------------------------------------------------------

/// Emit `A64Op`s for a single instruction, possibly consuming subsequent
/// instructions.  Returns the number of IR instructions consumed.
fn munch_instruction(
    instr: &Instruction,
    _suffix: &[Instruction],
    ops: &mut Vec<A64Op>,
    shift_map: &std::collections::BTreeMap<String, (String, ShiftKind, u8)>,
    value_map: &std::collections::BTreeMap<String, i64>,
    type_map: &std::collections::BTreeMap<String, String>,
    user_fn_names: &std::collections::BTreeSet<String>,
    layouts: &Layouts,
) -> usize {
    match instr {
        Instruction::Const { result, value, ty } => {
            if let Some(n) = value.as_i64() {
                if (0..=65535).contains(&n) {
                    ops.push(A64Op::Movz {
                        rd: result.clone(),
                        imm: n as u16,
                        shift: 0,
                    });
                } else if (-4096..4096).contains(&n) {
                    ops.push(A64Op::MovImm {
                        rd: result.clone(),
                        imm: n,
                        ty: ty.clone(),
                    });
                } else {
                    // 64-bit immediate — materialise with movz + movk.
                    let low = n as u16;
                    ops.push(A64Op::Movz {
                        rd: result.clone(),
                        imm: low,
                        shift: 0,
                    });
                    for shift in (1..4).step_by(1) {
                        let part = ((n >> (shift * 16)) & 0xFFFF) as u16;
                        if part != 0 {
                            ops.push(A64Op::Movk {
                                rd: result.clone(),
                                imm: part,
                                shift: shift * 16,
                            });
                        }
                    }
                }
            } else if let Some(f) = value.as_f64() {
                ops.push(A64Op::LoadFloat {
                    rd: result.clone(),
                    bits: f.to_bits(),
                });
            } else if let Some(s) = value.as_str() {
                ops.push(A64Op::LoadString {
                    rd: result.clone(),
                    text: s.to_string(),
                });
            }
            1
        }

        Instruction::BinOp {
            result,
            lhs,
            rhs,
            op_type,
            ty,
        } => {
            // Float (f64) arithmetic and comparisons use d0/d1 as scratch
            // registers (not in the allocator pool), with values transferred
            // via FMov from/to GPRs.  This avoids complex float register
            // allocation while still producing correct float code.
            if ty == "f64" || type_map.get(lhs).map(String::as_str) == Some("f64") {
                match op_type.as_str() {
                    "+" => {
                        ops.push(A64Op::FMov { rd: "d0".to_string(), rm: lhs.clone() });
                        ops.push(A64Op::FMov { rd: "d1".to_string(), rm: rhs.clone() });
                        ops.push(A64Op::FAdd { rd: "d0".to_string(), rn: "d0".to_string(), rm: "d1".to_string() });
                        ops.push(A64Op::FMov { rd: result.clone(), rm: "d0".to_string() });
                    }
                    "-" => {
                        ops.push(A64Op::FMov { rd: "d0".to_string(), rm: lhs.clone() });
                        ops.push(A64Op::FMov { rd: "d1".to_string(), rm: rhs.clone() });
                        ops.push(A64Op::FSub { rd: "d0".to_string(), rn: "d0".to_string(), rm: "d1".to_string() });
                        ops.push(A64Op::FMov { rd: result.clone(), rm: "d0".to_string() });
                    }
                    "*" => {
                        ops.push(A64Op::FMov { rd: "d0".to_string(), rm: lhs.clone() });
                        ops.push(A64Op::FMov { rd: "d1".to_string(), rm: rhs.clone() });
                        ops.push(A64Op::FMul { rd: "d0".to_string(), rn: "d0".to_string(), rm: "d1".to_string() });
                        ops.push(A64Op::FMov { rd: result.clone(), rm: "d0".to_string() });
                    }
                    "/" => {
                        ops.push(A64Op::FMov { rd: "d0".to_string(), rm: lhs.clone() });
                        ops.push(A64Op::FMov { rd: "d1".to_string(), rm: rhs.clone() });
                        ops.push(A64Op::FDiv { rd: "d0".to_string(), rn: "d0".to_string(), rm: "d1".to_string() });
                        ops.push(A64Op::FMov { rd: result.clone(), rm: "d0".to_string() });
                    }
                    "<" | "<=" | ">" | ">=" | "==" | "!=" => {
                        ops.push(A64Op::FMov { rd: "d0".to_string(), rm: lhs.clone() });
                        ops.push(A64Op::FMov { rd: "d1".to_string(), rm: rhs.clone() });
                        ops.push(A64Op::FCmp { rn: "d0".to_string(), rm: "d1".to_string() });
                        let cond = match op_type.as_str() {
                            "<" => "mi", "<=" => "ls", ">" => "gt", ">=" => "ge",
                            "==" => "eq", "!=" => "ne", _ => "eq",
                        };
                        ops.push(A64Op::Cset { rd: result.clone(), cond: cond.to_string(), ty: "bool".to_string() });
                    }
                    _ => {
                        ops.push(A64Op::FMov { rd: "d0".to_string(), rm: lhs.clone() });
                        ops.push(A64Op::FMov { rd: "d1".to_string(), rm: rhs.clone() });
                        ops.push(A64Op::FAdd { rd: "d0".to_string(), rn: "d0".to_string(), rm: "d1".to_string() });
                        ops.push(A64Op::FMov { rd: result.clone(), rm: "d0".to_string() });
                    }
                }
                return 1;
            }

            match op_type.as_str() {
                "+" => {
                    // Try to fold into shifted-register form if rhs is
                    // a shift operation (constant small shift).
                    // Use the shift's *source* register, not the result of the shift.
                    if let Some((source, shift_kind, amount)) = shift_map.get(rhs) {
                        ops.push(A64Op::Add {
                            rd: result.clone(),
                            rn: lhs.clone(),
                            rm: ExtendedReg::shifted(source, *shift_kind, *amount),
                            ty: ty.clone(),
                        });
                    } else {
                        ops.push(A64Op::Add {
                            rd: result.clone(),
                            rn: lhs.clone(),
                            rm: ExtendedReg::plain(rhs),
                            ty: ty.clone(),
                        });
                    }
                }
                "-" => {
                    if let Some((source, shift_kind, amount)) = shift_map.get(rhs) {
                        ops.push(A64Op::Sub {
                            rd: result.clone(),
                            rn: lhs.clone(),
                            rm: ExtendedReg::shifted(source, *shift_kind, *amount),
                            ty: ty.clone(),
                        });
                    } else {
                        ops.push(A64Op::Sub {
                            rd: result.clone(),
                            rn: lhs.clone(),
                            rm: ExtendedReg::plain(rhs),
                            ty: ty.clone(),
                        });
                    }
                }
                "*" => {
                    // Multiplication by constant small power of two
                    // can be strength-reduced to a shift during
                    // instruction selection.
                    if let Some(shift) = is_power_of_two_shift(rhs, value_map) {
                        if shift > 0 {
                            ops.push(A64Op::Lsl {
                                rd: result.clone(),
                                rn: lhs.clone(),
                                amount: shift,
                                ty: ty.clone(),
                            });
                        } else {
                            // * 1 → just move.
                            ops.push(A64Op::MovReg {
                                rd: result.clone(),
                                rm: lhs.clone(),
                            });
                        }
                    } else if let Some(shift) = is_power_of_two_shift(lhs, value_map) {
                        if shift > 0 {
                            ops.push(A64Op::Lsl {
                                rd: result.clone(),
                                rn: rhs.clone(),
                                amount: shift,
                                ty: ty.clone(),
                            });
                        } else {
                            ops.push(A64Op::MovReg {
                                rd: result.clone(),
                                rm: rhs.clone(),
                            });
                        }
                    } else {
                        ops.push(A64Op::Mul {
                            rd: result.clone(),
                            rn: lhs.clone(),
                            rm: rhs.clone(),
                            ty: ty.clone(),
                        });
                    }
                }
                "/" => {
                    ops.push(A64Op::Sdiv {
                        rd: result.clone(),
                        rn: lhs.clone(),
                        rm: rhs.clone(),
                        ty: ty.clone(),
                    });
                }
                "<<" => {
                    let amount = value_map.get(rhs).copied().unwrap_or(0) as u8;
                    ops.push(A64Op::Lsl {
                        rd: result.clone(),
                        rn: lhs.clone(),
                        amount,
                        ty: ty.clone(),
                    });
                }
                ">>" => {
                    let amount = value_map.get(rhs).copied().unwrap_or(0) as u8;
                    ops.push(A64Op::Asr {
                        rd: result.clone(),
                        rn: lhs.clone(),
                        amount,
                        ty: ty.clone(),
                    });
                }
                "&" => {
                    ops.push(A64Op::And {
                        rd: result.clone(),
                        rn: lhs.clone(),
                        rm: ExtendedReg::plain(rhs),
                        ty: ty.clone(),
                    });
                }
                "|" => {
                    ops.push(A64Op::Orr {
                        rd: result.clone(),
                        rn: lhs.clone(),
                        rm: ExtendedReg::plain(rhs),
                        ty: ty.clone(),
                    });
                }
                "^" | "xor" => {
                    ops.push(A64Op::Eor {
                        rd: result.clone(),
                        rn: lhs.clone(),
                        rm: ExtendedReg::plain(rhs),
                        ty: ty.clone(),
                    });
                }
                // Comparisons — ARM64 sets NZCV and then we can cset.
                "<" | "<=" | ">" | ">=" | "==" | "!=" => {
                    ops.push(A64Op::Cmp {
                        rn: lhs.clone(),
                        rm: ExtendedReg::plain(rhs),
                    });
                    let cond = binop_cond_to_arm64(op_type);
                    ops.push(A64Op::Cset {
                        rd: result.clone(),
                        cond,
                        ty: ty.clone(),
                    });
                }
                other => {
                    // Unknown op — emit as generic.
                    ops.push(A64Op::Add {
                        rd: result.clone(),
                        rn: lhs.clone(),
                        rm: ExtendedReg::plain(rhs),
                        ty: ty.clone(),
                    });
                }
            }
            1
        }

        Instruction::Call {
            result,
            function,
            arguments,
            ty,
        } => {
            if function == "print" {
                // Emit a type-specific print pseudo-op.  The emitter
                // (emit_op) handles the platform ABI (macOS vs Linux)
                // and loads the correct format string (.LC_print_*).
                if let Some(arg) = arguments.first() {
                    let arg_ty = type_map.get(arg).map(String::as_str).unwrap_or("i64");
                    match arg_ty {
                        "string" => ops.push(A64Op::PrintStringArg { reg: arg.clone() }),
                        "f64" => ops.push(A64Op::PrintF64Arg { reg: arg.clone() }),
                        _ => ops.push(A64Op::PrintI64Arg { reg: arg.clone() }),
                    }
                }
                if let Some(r) = result {
                    ops.push(A64Op::MovReg {
                        rd: r.clone(),
                        rm: "x0".to_string(),
                    });
                }
            } else {
                // Move arguments to ABI argument registers.
                let mut int_idx = 0usize;
                let mut float_idx = 0usize;
                for arg in arguments.iter() {
                    let is_float = type_map.get(arg).map(String::as_str) == Some("f64");
                    if is_float {
                        if let Some(reg) = crate::backend::arm64::abi::AArch64ABI::float_arg_register(float_idx) {
                            ops.push(A64Op::FMov { rd: reg.to_string(), rm: arg.clone() });
                            float_idx += 1;
                        }
                    } else if user_fn_names.contains(arg) {
                        if let Some(reg) = crate::backend::arm64::abi::AArch64ABI::arg_register(int_idx) {
                            ops.push(A64Op::LoadFuncAddr { rd: reg.to_string(), name: arg.clone() });
                            int_idx += 1;
                        }
                    } else {
                        if let Some(reg) = crate::backend::arm64::abi::AArch64ABI::arg_register(int_idx) {
                            ops.push(A64Op::MovReg { rd: reg.to_string(), rm: arg.clone() });
                            int_idx += 1;
                        }
                    }
                }
                let label = function.clone();
                ops.push(A64Op::Bl { label });
                if let Some(r) = result {
                    let is_float = ty.as_deref() == Some("f64")
                        || type_map.get(r).map(String::as_str) == Some("f64");
                    if is_float {
                        ops.push(A64Op::FMov { rd: "d0".to_string(), rm: r.clone() });
                    } else {
                        ops.push(A64Op::MovReg { rd: r.clone(), rm: "x0".to_string() });
                    }
                }
            }
            1
        }

        Instruction::CallIndirect {
            result,
            function_value,
            arguments,
            ty,
        } => {
            // Move arguments to ABI argument registers before the call.
            let mut int_idx = 0usize;
            let mut float_idx = 0usize;
            for arg in arguments.iter() {
                let is_float = type_map.get(arg).map(String::as_str) == Some("f64");
                if is_float {
                    if let Some(reg) = crate::backend::arm64::abi::AArch64ABI::float_arg_register(float_idx) {
                        ops.push(A64Op::FMov { rd: reg.to_string(), rm: arg.clone() });
                        float_idx += 1;
                    }
                } else {
                    if let Some(reg) = crate::backend::arm64::abi::AArch64ABI::arg_register(int_idx) {
                        ops.push(A64Op::MovReg { rd: reg.to_string(), rm: arg.clone() });
                        int_idx += 1;
                    }
                }
            }
            ops.push(A64Op::Blr {
                reg: function_value.clone(),
            });
            if let Some(r) = result {
                let is_float = ty.as_deref() == Some("f64")
                    || type_map.get(r).map(String::as_str) == Some("f64");
                if is_float {
                    ops.push(A64Op::FMov { rd: "d0".to_string(), rm: r.clone() });
                } else {
                    ops.push(A64Op::MovReg { rd: r.clone(), rm: "x0".to_string() });
                }
            }
            1
        }

        Instruction::Return { value } => {
            if let Some(v) = value {
                if type_map.get(v).map(String::as_str) == Some("f64") {
                    ops.push(A64Op::FMov {
                        rd: "d0".to_string(),
                        rm: v.clone(),
                    });
                } else {
                    ops.push(A64Op::MovReg {
                        rd: "x0".to_string(),
                        rm: v.clone(),
                    });
                }
            }
            ops.push(A64Op::Ret);
            1
        }

        Instruction::Jump { label } => {
            ops.push(A64Op::B {
                label: label.clone(),
            });
            1
        }

        Instruction::Branch {
            cond,
            true_label,
            false_label,
        } => {
            // Emit a comparison of the boolean against 0 to set NZCV,
            // then branch based on the result.
            ops.push(A64Op::CmpImm {
                rn: cond.clone(),
                imm: 0,
            });
            ops.push(A64Op::BCond {
                cond: "ne".to_string(),
                label: true_label.clone(),
            });
            ops.push(A64Op::B {
                label: false_label.clone(),
            });
            1
        }

        Instruction::Phi { .. } => {
            // Phi nodes are handled by if-conversion or by the register
            // allocator (edge moves).  Skip them here.
            1
        }

        Instruction::Alloc { result, size, .. } => {
            // Alloc → malloc call or stack allocation.
            if let Some(sz) = size {
                ops.push(A64Op::MovReg {
                    rd: "x0".to_string(),
                    rm: sz.clone(),
                });
            } else {
                ops.push(A64Op::Movz {
                    rd: "x0".to_string(),
                    imm: 64,
                    shift: 0,
                });
            }
            ops.push(A64Op::Bl {
                label: "malloc".to_string(),
            });
            ops.push(A64Op::MovReg {
                rd: result.clone(),
                rm: "x0".to_string(),
            });
            1
        }

        Instruction::GetField {
            result,
            object,
            field,
            ty,
        } => {
            let offset = layouts.field_offset(
                type_map.get(object).map(String::as_str).unwrap_or(""),
                field,
            );
            ops.push(A64Op::Ldr {
                rd: result.clone(),
                addr: AddressingMode::BaseOffset(object.clone(), offset),
                ty: ty.clone(),
            });
            1
        }

        Instruction::SetField {
            object,
            field,
            value,
            ty,
        } => {
            let offset = layouts.field_offset(
                type_map.get(object).map(String::as_str).unwrap_or(""),
                field,
            );
            ops.push(A64Op::Str {
                rs: value.clone(),
                addr: AddressingMode::BaseOffset(object.clone(), offset),
                ty: ty.clone(),
            });
            1
        }

        Instruction::GetIndex {
            result,
            array,
            index,
            ty,
        } => {
            // Array access: load from array_base + index * 8.
            let addr = AddressingMode::RegisterOffset {
                base: array.clone(),
                index: index.clone(),
                shift: Some((ShiftKind::Lsl, 3)), // *8 for 64-bit elements
            };
            ops.push(A64Op::Ldr {
                rd: result.clone(),
                addr,
                ty: ty.clone(),
            });
            1
        }

        Instruction::SetIndex {
            array,
            index,
            value,
            ty,
        } => {
            let addr = AddressingMode::RegisterOffset {
                base: array.clone(),
                index: index.clone(),
                shift: Some((ShiftKind::Lsl, 3)),
            };
            ops.push(A64Op::Str {
                rs: value.clone(),
                addr,
                ty: ty.clone(),
            });
            1
        }

        Instruction::AddrOf { result, operand, .. } => {
            // Address-of pseudo-op: resolved after register allocation
            // during emit_assembly, where we know the stack offset.
            ops.push(A64Op::AddrOf {
                rd: result.clone(),
                rn: operand.clone(),
            });
            1
        }

        Instruction::Deref { result, operand, ty } => {
            ops.push(A64Op::Ldr {
                rd: result.clone(),
                addr: AddressingMode::Base(operand.clone()),
                ty: ty.clone(),
            });
            1
        }
    }
}

// ---------------------------------------------------------------------------
// Instruction emitters (→ text)
// ---------------------------------------------------------------------------

/// Convert a `SelectedFunction` list into ARM64 assembly text.
pub fn emit_assembly(functions: &[SelectedFunction]) -> String {
    let mut out = String::new();

    out.push_str(".arch armv8-a\n");

    // Collect user-defined function names for symbol resolution.
    let user_fns: std::collections::HashSet<String> =
        functions.iter().map(|f| f.name.clone()).collect();

    // Collect and deduplicate all string literals.
    let mut string_literals = std::collections::BTreeMap::new();
    let mut next_string_id = 0;
    // Collect and deduplicate all float literals.
    let mut float_literals: std::collections::BTreeMap<u64, String> = std::collections::BTreeMap::new();
    let mut next_float_id = 0;
    for func in functions {
        for block in &func.blocks {
            for op in &block.ops {
                if let A64Op::LoadString { text, .. } = op {
                    if !string_literals.contains_key(text) {
                        let label = format!(".LC_str_{}", next_string_id);
                        next_string_id += 1;
                        string_literals.insert(text.clone(), label);
                    }
                }
                if let A64Op::LoadFloat { bits, .. } = op {
                    if !float_literals.contains_key(bits) {
                        let label = format!(".LC_float_{}", next_float_id);
                        next_float_id += 1;
                        float_literals.insert(*bits, label);
                    }
                }
            }
        }
    }

    // ---- Format strings for print() and custom string literals ----------
    // On macOS, use __TEXT,__const (Mach-O read-only data section) with
    // adrp + @PAGE/@PAGEOFF addressing.  On ELF, use .rodata with adrp +
    // :lo12: relocation.
    if cfg!(target_os = "macos") {
        out.push_str(".section __TEXT,__const\n");
        out.push_str(".balign 8\n");
        out.push_str(".LC_print_i64:\n");
        out.push_str(".asciz \"%ld\\n\"\n");
        out.push_str(".LC_print_string:\n");
        out.push_str(".asciz \"%s\\n\"\n");
        out.push_str(".LC_print_f64:\n");
        out.push_str(".asciz \"%f\\n\"\n");
        for (text, label) in &string_literals {
            out.push_str(&format!("{}:\n", label));
            out.push_str(&format!(".asciz {:?}\n", text));
        }
        for (bits, label) in &float_literals {
            let value = f64::from_bits(*bits);
            out.push_str(&format!(".balign 8\n"));
            out.push_str(&format!("{}:\n", label));
            out.push_str(&format!(".double {}\n", value));
        }
        out.push_str("\n");
    } else {
        out.push_str(".section .rodata\n");
        out.push_str(".balign 8\n");
        out.push_str(".LC_print_i64:\n");
        out.push_str(".asciz \"%ld\\n\"\n");
        out.push_str(".LC_print_string:\n");
        out.push_str(".asciz \"%s\\n\"\n");
        out.push_str(".LC_print_f64:\n");
        out.push_str(".asciz \"%f\\n\"\n");
        for (text, label) in &string_literals {
            out.push_str(&format!("{}:\n", label));
            out.push_str(&format!(".asciz {:?}\n", text));
        }
        for (bits, label) in &float_literals {
            let value = f64::from_bits(*bits);
            out.push_str(&format!(".balign 8\n"));
            out.push_str(&format!("{}:\n", label));
            out.push_str(&format!(".double {}\n", value));
        }
        out.push_str("\n");
    }
    out.push_str(".text\n");

    for func in functions {
        let sym = sanitize_symbol(&func.name);
        out.push_str(".balign 4\n");
        out.push_str(&format!(".globl {}\n", sym));
        if func.name == "main" {
            out.push_str(".globl _main\n");
            out.push_str("_main:\n");
        }
        out.push_str(&format!("{}:\n", sym));

        // Prologue: save frame pointer, link register, and callee-saved regs.
        out.push_str("\tstp x29, x30, [sp, #-16]!\n");
        let cs_regs = &func.used_callee_saved;
        for chunk in cs_regs.chunks(2) {
            if chunk.len() == 2 {
                out.push_str(&format!("\tstp {}, {}, [sp, #-16]!\n", chunk[0], chunk[1]));
            } else {
                out.push_str(&format!("\tstr {}, [sp, #-16]!\n", chunk[0]));
            }
        }
        out.push_str("\tmov x29, sp\n");

        if func.frame_size > 0 {
            out.push_str(&format!("\tsub sp, sp, #{}\n", func.frame_size));
        }

        for block in &func.blocks {
            if !block.label.starts_with("__") && block.label != "entry" {
                out.push_str(&format!(".L{}_{}:\n", sym, block.label));
            }
            for op in &block.ops {
                // Replace Ret with branch to epilogue so callee-saved + fp/lr
                // are restored properly. The epilogue's final ret handles return.
                if matches!(op, A64Op::Ret) {
                    // main siempre retorna 0 implícitamente.
                    if func.name == "main" {
                        out.push_str("\tmovz x0, #0\n");
                    }
                    out.push_str(&format!("\tb .L{}_end\n", sym));
                } else {
                    emit_op(&mut out, op, &user_fns, &sym, &string_literals, &float_literals);
                }
            }
        }

        // Epilogue: restore callee-saved registers (reverse order), then fp/lr.
        out.push_str(&format!(".L{}_end:\n", sym));
        if func.frame_size > 0 {
            out.push_str(&format!("\tadd sp, sp, #{}\n", func.frame_size));
        }
        // Restore callee-saved in reverse order.
        let cs_regs = &func.used_callee_saved;
        for chunk in cs_regs.chunks(2).rev() {
            if chunk.len() == 2 {
                out.push_str(&format!("\tldp {}, {}, [sp], #16\n", chunk[0], chunk[1]));
            } else {
                out.push_str(&format!("\tldr {}, [sp], #16\n", chunk[0]));
            }
        }
        out.push_str("\tldp x29, x30, [sp], #16\n");
        out.push_str("\tret\n");
    }

    out
}

fn emit_op(
    out: &mut String,
    op: &A64Op,
    user_fns: &std::collections::HashSet<String>,
    func_sym: &str,
    string_literals: &std::collections::BTreeMap<String, String>,
    float_literals: &std::collections::BTreeMap<u64, String>,
) {
    // Symbol name mangling for Mach-O (macOS) vs ELF (Linux).
    let mangle = |name: &str| -> String {
        if cfg!(target_os = "macos") && !user_fns.contains(name) {
            format!("_{}", name)
        } else {
            name.to_string()
        }
    };
    match op {
        // -- ALU register --------------------------------------------------
        A64Op::Add { rd, rn, rm, .. } => {
            write_alu(out, "add", rd, rn, rm);
        }
        A64Op::Sub { rd, rn, rm, .. } => {
            write_alu(out, "sub", rd, rn, rm);
        }
        A64Op::Mul { rd, rn, rm, .. } => {
            out.push_str(&format!("\tmul {}, {}, {}\n", rd, rn, rm));
        }
        A64Op::Sdiv { rd, rn, rm, .. } => {
            out.push_str(&format!("\tsdiv {}, {}, {}\n", rd, rn, rm));
        }
        A64Op::And { rd, rn, rm, .. } => write_alu(out, "and", rd, rn, rm),
        A64Op::Orr { rd, rn, rm, .. } => write_alu(out, "orr", rd, rn, rm),
        A64Op::Eor { rd, rn, rm, .. } => write_alu(out, "eor", rd, rn, rm),
        A64Op::Lsl { rd, rn, amount, .. } => {
            out.push_str(&format!("\tlsl {}, {}, #{}\n", rd, rn, amount));
        }
        A64Op::Lsr { rd, rn, amount, .. } => {
            out.push_str(&format!("\tlsr {}, {}, #{}\n", rd, rn, amount));
        }
        A64Op::Asr { rd, rn, amount, .. } => {
            out.push_str(&format!("\tasr {}, {}, #{}\n", rd, rn, amount));
        }

        // -- ALU immediate -------------------------------------------------
        A64Op::AddImm { rd, rn, imm, .. } => {
            out.push_str(&format!("\tadd {}, {}, #{}\n", rd, rn, imm));
        }
        A64Op::SubImm { rd, rn, imm, .. } => {
            out.push_str(&format!("\tsub {}, {}, #{}\n", rd, rn, imm));
        }
        A64Op::MovImm { rd, imm, .. } => {
            out.push_str(&format!("\tmov {}, #{}\n", rd, imm));
        }
        A64Op::MovReg { rd, rm } => {
            if user_fns.contains(rm) {
                let sym = mangle(rm);
                if cfg!(target_os = "macos") {
                    out.push_str(&format!("\tadrp {}, {}@PAGE\n", rd, sym));
                    out.push_str(&format!("\tadd {}, {}, {}@PAGEOFF\n", rd, rd, sym));
                } else {
                    out.push_str(&format!("\tadrp {}, {}\n", rd, sym));
                    out.push_str(&format!("\tadd {}, {}, :lo12:{}\n", rd, rd, sym));
                }
            } else {
                out.push_str(&format!("\tmov {}, {}\n", rd, rm));
            }
        }

        // -- Comparison ----------------------------------------------------
        A64Op::Cmp { rn, rm } => {
            if let Some((kind, amount)) = &rm.shift {
                out.push_str(&format!(
                    "\tcmp {}, {}, {} #{}\n",
                    rn, rm.reg, shift_kind_str(kind), amount
                ));
            } else {
                out.push_str(&format!("\tcmp {}, {}\n", rn, rm.reg));
            }
        }
        A64Op::CmpImm { rn, imm } => {
            out.push_str(&format!("\tcmp {}, #{}\n", rn, imm));
        }

        // -- Conditional select --------------------------------------------
        A64Op::Csel { rd, rn, rm, cond, .. } => {
            out.push_str(&format!("\tcsel {}, {}, {}, {}\n", rd, rn, rm, cond));
        }
        A64Op::Csinc { rd, rn, rm, cond, .. } => {
            out.push_str(&format!("\tcsinc {}, {}, {}, {}\n", rd, rn, rm, cond));
        }
        A64Op::Cset { rd, cond, .. } => {
            out.push_str(&format!("\tcset {}, {}\n", rd, cond));
        }

        // -- Load / Store --------------------------------------------------
        A64Op::Ldr { rd, addr, .. } => {
            write_mem(out, "ldr", rd, addr);
        }
        A64Op::Str { rs, addr, .. } => {
            write_mem(out, "str", rs, addr);
        }
        A64Op::Ldrb { rd, addr } => write_mem(out, "ldrb", rd, addr),
        A64Op::Strb { rs, addr } => write_mem(out, "strb", rs, addr),
        A64Op::Ldrsw { rd, addr } => write_mem(out, "ldrsw", rd, addr),
        A64Op::Ldp { rt1, rt2, addr, .. } => {
            write_mem_pair(out, "ldp", rt1, rt2, addr);
        }
        A64Op::Stp { rt1, rt2, addr, .. } => {
            write_mem_pair(out, "stp", rt1, rt2, addr);
        }

        // -- Print (pseudo-op via printf) -----------------------------------
        A64Op::PrintI64Arg { reg } => {
            // The value in `reg` is placed according to the platform's
            // variadic calling convention:
            //   macOS: push to stack  (Apple's ARM64 ABI)
            //   Linux: x1 register    (standard AAPCS64)
            if cfg!(target_os = "macos") {
                out.push_str(&format!("\tstr {}, [sp, #-16]!\n", reg));
                out.push_str(&format!(
                    "\tadrp x0, .LC_print_i64@PAGE\n"
                ));
                out.push_str(&format!(
                    "\tadd x0, x0, .LC_print_i64@PAGEOFF\n"
                ));
                out.push_str(&format!("\tbl {}\n", mangle("printf")));
                out.push_str("\tadd sp, sp, #16\n");
            } else {
                out.push_str(&format!("\tmov x1, {}\n", reg));
                out.push_str("\tadrp x0, .LC_print_i64\n");
                out.push_str("\tadd x0, x0, :lo12:.LC_print_i64\n");
                out.push_str(&format!("\tbl {}\n", mangle("printf")));
            }
        }
        A64Op::PrintStringArg { reg } => {
            if cfg!(target_os = "macos") {
                out.push_str(&format!("\tstr {}, [sp, #-16]!\n", reg));
                out.push_str("\tadrp x0, .LC_print_string@PAGE\n");
                out.push_str("\tadd x0, x0, .LC_print_string@PAGEOFF\n");
                out.push_str(&format!("\tbl {}\n", mangle("printf")));
                out.push_str("\tadd sp, sp, #16\n");
            } else {
                out.push_str(&format!("\tmov x1, {}\n", reg));
                out.push_str("\tadrp x0, .LC_print_string\n");
                out.push_str("\tadd x0, x0, :lo12:.LC_print_string\n");
                out.push_str(&format!("\tbl {}\n", mangle("printf")));
            }
        }
        A64Op::PrintF64Arg { reg } => {
            // f64 values pass in d0 (float register), format with "%f\n"
            if cfg!(target_os = "macos") {
                out.push_str(&format!("\tfmov d0, {}\n", reg));
                out.push_str(&format!("\tstr d0, [sp, #-16]!\n"));
                out.push_str("\tadrp x0, .LC_print_f64@PAGE\n");
                out.push_str("\tadd x0, x0, .LC_print_f64@PAGEOFF\n");
                out.push_str(&format!("\tbl {}\n", mangle("printf")));
                out.push_str("\tadd sp, sp, #16\n");
            } else {
                out.push_str(&format!("\tfmov d0, {}\n", reg));
                out.push_str("\tadrp x0, .LC_print_f64\n");
                out.push_str("\tadd x0, x0, :lo12:.LC_print_f64\n");
                out.push_str(&format!("\tbl {}\n", mangle("printf")));
            }
        }

        A64Op::LoadFuncAddr { rd, name } => {
            let sym = mangle(name);
            if cfg!(target_os = "macos") {
                out.push_str(&format!("\tadrp {}, {}@PAGE\n", rd, sym));
                out.push_str(&format!("\tadd {}, {}, {}@PAGEOFF\n", rd, rd, sym));
            } else {
                out.push_str(&format!("\tadrp {}, {}\n", rd, sym));
                out.push_str(&format!("\tadd {}, {}, :lo12:{}\n", rd, rd, sym));
            }
        }
        A64Op::LoadFloat { rd, bits } => {
            let label = float_literals.get(bits).expect("float literal must be interned");
            // Use x16 (scratch, not in allocator pool) as the address register.
            if cfg!(target_os = "macos") {
                out.push_str(&format!("\tadrp x16, {}@PAGE\n", label));
                out.push_str(&format!("\tldr {}, [x16, {}@PAGEOFF]\n", rd, label));
            } else {
                out.push_str(&format!("\tadrp x16, {}\n", label));
                out.push_str(&format!("\tldr {}, [x16, :lo12:{}]\n", rd, label));
            }
        }
        A64Op::LoadString { rd, text } => {
            let label = string_literals.get(text).expect("string literal must be interned");
            if cfg!(target_os = "macos") {
                out.push_str(&format!("\tadrp {}, {}@PAGE\n", rd, label));
                out.push_str(&format!("\tadd {}, {}, {}@PAGEOFF\n", rd, rd, label));
            } else {
                out.push_str(&format!("\tadrp {}, {}\n", rd, label));
                out.push_str(&format!("\tadd {}, {}, :lo12:{}\n", rd, rd, label));
            }
        }

        // -- Branch & call -------------------------------------------------
        A64Op::B { label } => {
            out.push_str(&format!("\tb .L{}_{}\n", func_sym, label));
        }
        A64Op::BCond { cond, label } => {
            out.push_str(&format!("\tb.{} .L{}_{}\n", cond, func_sym, label));
        }
        A64Op::Bl { label } => {
            let sym = sanitize_symbol(label);
            let is_user = user_fns.contains(label);
            if cfg!(target_os = "macos") && !is_user {
                out.push_str(&format!("\tbl _{}\n", sym));
            } else {
                out.push_str(&format!("\tbl {}\n", sym));
            }
        }
        A64Op::Blr { reg } => {
            out.push_str(&format!("\tblr {}\n", reg));
        }
        A64Op::Ret => {
            out.push_str("\tret\n");
        }

        A64Op::AddrOf { rd, rn } => {
            // Address-of: store operand at [x29, #-16] (within the guaranteed
            // 16-byte local frame) and compute its address.
            out.push_str(&format!("\tstr {}, [x29, #-16]\n", rn));
            out.push_str(&format!("\tadd {}, x29, #-16\n", rd));
        }

        // -- Stack frame ---------------------------------------------------
        A64Op::StpFrame => {
            out.push_str("\tstp x29, x30, [sp, #-16]!\n");
        }
        A64Op::LdpFrame => {
            out.push_str("\tldp x29, x30, [sp], #16\n");
        }
        A64Op::Prologue { frame_size } => {
            if *frame_size > 0 {
                out.push_str(&format!("\tsub sp, sp, #{}\n", frame_size));
            }
        }
        A64Op::Epilogue => {
            out.push_str("\tret\n");
        }

        // -- Move wide -----------------------------------------------------
        A64Op::Movz { rd, imm, shift } => {
            if *shift > 0 {
                out.push_str(&format!("\tmovz {}, #{}, lsl #{}\n", rd, imm, shift));
            } else {
                out.push_str(&format!("\tmovz {}, #{}\n", rd, imm));
            }
        }
        A64Op::Movk { rd, imm, shift } => {
            out.push_str(&format!("\tmovk {}, #{}, lsl #{}\n", rd, imm, shift));
        }

        // -- Float ---------------------------------------------------------
        A64Op::FAdd { rd, rn, rm } => {
            out.push_str(&format!("\tfadd {}, {}, {}\n", rd, rn, rm));
        }
        A64Op::FSub { rd, rn, rm } => {
            out.push_str(&format!("\tfsub {}, {}, {}\n", rd, rn, rm));
        }
        A64Op::FMul { rd, rn, rm } => {
            out.push_str(&format!("\tfmul {}, {}, {}\n", rd, rn, rm));
        }
        A64Op::FDiv { rd, rn, rm } => {
            out.push_str(&format!("\tfdiv {}, {}, {}\n", rd, rn, rm));
        }
        A64Op::FCmp { rn, rm } => {
            out.push_str(&format!("\tfcmp {}, {}\n", rn, rm));
        }
        A64Op::FMov { rd, rm } => {
            out.push_str(&format!("\tfmov {}, {}\n", rd, rm));
        }
        A64Op::FMovImm { rd, .. } => {
            out.push_str(&format!("\tfmov {}, #0.5\n", rd));
        }
        A64Op::LdrFloat { rd, addr } => write_mem(out, "ldr", rd, addr),
        A64Op::StrFloat { rs, addr } => write_mem(out, "str", rs, addr),
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn write_alu(out: &mut String, mnemonic: &str, rd: &str, rn: &str, rm: &ExtendedReg) {
    if let Some((kind, amount)) = &rm.shift {
        out.push_str(&format!(
            "\t{} {}, {}, {}, {} #{}\n",
            mnemonic,
            rd,
            rn,
            rm.reg,
            shift_kind_str(kind),
            amount
        ));
    } else {
        out.push_str(&format!("\t{} {}, {}, {}\n", mnemonic, rd, rn, rm.reg));
    }
}

fn write_mem(out: &mut String, mnemonic: &str, reg: &str, addr: &AddressingMode) {
    match addr {
        AddressingMode::Base(base) => {
            out.push_str(&format!("\t{} {}, [{}]\n", mnemonic, reg, base));
        }
        AddressingMode::BaseOffset(base, offset) => {
            out.push_str(&format!("\t{} {}, [{}, #{}]\n", mnemonic, reg, base, offset));
        }
        AddressingMode::RegisterOffset {
            base,
            index,
            shift,
        } => {
            if let Some((kind, amount)) = shift {
                out.push_str(&format!(
                    "\t{} {}, [{}, {}, {} #{}]\n",
                    mnemonic,
                    reg,
                    base,
                    index,
                    shift_kind_str(kind),
                    amount
                ));
            } else {
                out.push_str(&format!(
                    "\t{} {}, [{}, {}]\n",
                    mnemonic, reg, base, index
                ));
            }
        }
        AddressingMode::PreIndexed(base, offset) => {
            out.push_str(&format!(
                "\t{} {}, [{}, #{}]!\n",
                mnemonic, reg, base, offset
            ));
        }
        AddressingMode::PostIndexed(base, offset) => {
            out.push_str(&format!(
                "\t{} {}, [{}], #{}\n",
                mnemonic, reg, base, offset
            ));
        }
    }
}

/// Emit a load-pair or store-pair with addressing mode.
fn write_mem_pair(out: &mut String, mnemonic: &str, rt1: &str, rt2: &str, addr: &AddressingMode) {
    match addr {
        AddressingMode::Base(base) => {
            out.push_str(&format!("\t{} {}, {}, [{}]\n", mnemonic, rt1, rt2, base));
        }
        AddressingMode::BaseOffset(base, offset) => {
            out.push_str(&format!(
                "\t{} {}, {}, [{}, #{}]\n",
                mnemonic, rt1, rt2, base, offset
            ));
        }
        AddressingMode::PreIndexed(base, offset) => {
            out.push_str(&format!(
                "\t{} {}, {}, [{}, #{}]!\n",
                mnemonic, rt1, rt2, base, offset
            ));
        }
        AddressingMode::PostIndexed(base, offset) => {
            out.push_str(&format!(
                "\t{} {}, {}, [{}], #{}\n",
                mnemonic, rt1, rt2, base, offset
            ));
        }
        AddressingMode::RegisterOffset { base, index, shift } => {
            if let Some((kind, amount)) = shift {
                out.push_str(&format!(
                    "\t{} {}, {}, [{}, {}, {} #{}\n",
                    mnemonic,
                    rt1,
                    rt2,
                    base,
                    index,
                    shift_kind_str(kind),
                    amount
                ));
            } else {
                out.push_str(&format!(
                    "\t{} {}, {}, [{}, {}]\n",
                    mnemonic, rt1, rt2, base, index
                ));
            }
        }
    }
}

fn shift_kind_str(kind: &ShiftKind) -> &'static str {
    match kind {
        ShiftKind::Lsl => "lsl",
        ShiftKind::Lsr => "lsr",
        ShiftKind::Asr => "asr",
        ShiftKind::Uxtb => "uxtb",
        ShiftKind::Uxth => "uxth",
        ShiftKind::Uxtw => "uxtw",
    }
}

fn binop_cond_to_arm64(op: &str) -> String {
    match op {
        "<" => "lt".to_string(),
        "<=" => "le".to_string(),
        ">" => "gt".to_string(),
        ">=" => "ge".to_string(),
        "==" => "eq".to_string(),
        "!=" => "ne".to_string(),
        _ => "ne".to_string(),
    }
}

/// If `name` maps to a constant in `value_map` that is a positive power of two,
/// return the shift amount (log2 of the value).  Used by `*` strength-reduction.
fn is_power_of_two_shift(name: &str, value_map: &std::collections::BTreeMap<String, i64>) -> Option<u8> {
    if let Some(&val) = value_map.get(name) {
        if val > 0 && (val as u64).is_power_of_two() {
            return Some(val.trailing_zeros() as u8);
        }
    }
    None
}

/// Replace characters that are invalid in assembly labels (e.g. `-`)
/// with underscores.
fn sanitize_symbol(name: &str) -> String {
    let mut sym = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            sym.push(ch);
        } else {
            sym.push('_');
        }
    }
    if sym.is_empty() {
        "_".to_string()
    } else if sym.as_bytes()[0].is_ascii_digit() {
        format!("_{}", sym)
    } else {
        sym
    }
}
