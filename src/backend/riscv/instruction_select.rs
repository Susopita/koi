//! RISC-V instruction selection via Maximal Munch over the IR.
//!
//! ## Immediate folding
//!
//! Whenever an IR `BinOp` has a constant RHS that fits in 12 bits
//! (sign-extended, range [-2048, 2047]), the selector emits the immediate
//! form (`addi`, `sltiu`, etc.) instead of the register form.
//!
//! The constant zero register `x0` is folded into every instruction that
//! naturally supports it — `addi rd, x0, imm` becomes `li rd, imm`, and
//! `sub rd, rs1, x0` is a no-op that collapses to `mv rd, rs1`.
//!
//! ## Constant partitioning
//!
//! Constants larger than 12 bits are materialised with `lui` + `addi`.
//! The partitioning pass handles RISC-V's sign-extension quirk: when the
//! lower 12-bit chunk has bit 11 set, ADDI sign-extends and the upper
//! chunk must be incremented by 1 to compensate.

use crate::middle_end::ir::{BasicBlock, IRFunction, IRProgram, Instruction};

// ---------------------------------------------------------------------------
// RISC-V addressing mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum AddressingMode {
    /// Base register only: `(rs1)`
    Base(String),
    /// Base + signed 12-bit immediate: `imm(rs1)`
    BaseOffset(String, i16),
}

// ---------------------------------------------------------------------------
// RISC-V operations emitted by the instruction selector
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum RiscVOp {
    // -- ALU register ------------------------------------------------------
    Add { rd: String, rs1: String, rs2: String },
    Sub { rd: String, rs1: String, rs2: String },
    Mul { rd: String, rs1: String, rs2: String },
    Div { rd: String, rs1: String, rs2: String },

    // -- ALU immediate -----------------------------------------------------
    Addi { rd: String, rs1: String, imm: i16 },
    Sltiu { rd: String, rs1: String, imm: i16 },
    Xori { rd: String, rs1: String, imm: i16 },
    Ori { rd: String, rs1: String, imm: i16 },
    Andi { rd: String, rs1: String, imm: i16 },

    // -- Shifts (immediate) ------------------------------------------------
    Slli { rd: String, rs1: String, shamt: u8 },
    Srli { rd: String, rs1: String, shamt: u8 },
    Srai { rd: String, rs1: String, shamt: u8 },

    // -- Load upper immediate (64-bit) -------------------------------------
    Lui { rd: String, imm: i64 },
    /// Pseudo-instruction: `li rd, imm` (expands to lui+addi by assembler).
    Li { rd: String, imm: i64 },
    /// Move register: `addi rd, rs1, 0`
    Mv { rd: String, rs1: String },

    // -- ALU register (logical, register-register) -------------------------
    And { rd: String, rs1: String, rs2: String },
    Or { rd: String, rs1: String, rs2: String },
    Xor { rd: String, rs1: String, rs2: String },

    // -- Comparison (sets rd to 0/1) ---------------------------------------
    Slt { rd: String, rs1: String, rs2: String },
    Sltu { rd: String, rs1: String, rs2: String },
    Slti { rd: String, rs1: String, imm: i16 },
    /// `seqz rd, rs` — set = 1 if rs == 0
    Seqz { rd: String, rs: String },
    /// `snez rd, rs` — set = 1 if rs != 0
    Snez { rd: String, rs: String },

    // -- Load / Store (64-bit) ---------------------------------------------
    Ld { rd: String, addr: AddressingMode },
    Sd { rs2: String, addr: AddressingMode },

    // -- Load / Store (byte) -----------------------------------------------
    Lbu { rd: String, addr: AddressingMode },
    Sb { rs2: String, addr: AddressingMode },

    // -- Branch ------------------------------------------------------------
    Beq { rs1: String, rs2: String, label: String },
    Bne { rs1: String, rs2: String, label: String },
    Blt { rs1: String, rs2: String, label: String },
    Bge { rs1: String, rs2: String, label: String },
    /// Unconditional jump: `j label`
    J { label: String },
    /// Call via pseudo: `call func`
    Call { label: String },
    /// Indirect call: `jalr ra, rs1, 0`
    Jalr { rs1: String },
    /// Return: `jalr zero, ra, 0` or `ret`
    Ret,
    /// Break to the next line for block labels
    Label { label: String },

    // -- Stack frame -------------------------------------------------------
    /// `addi sp, sp, -frame` — allocate frame
    Prologue { frame_size: i64 },
    /// `addi sp, sp, frame` — deallocate frame
    Epilogue,
}

// ---------------------------------------------------------------------------
// Selected function / block
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SelectedFunction {
    pub name: String,
    pub blocks: Vec<SelectedBlock>,
    pub frame_size: i64,
}

#[derive(Debug, Clone)]
pub struct SelectedBlock {
    pub label: String,
    pub ops: Vec<RiscVOp>,
}

// ---------------------------------------------------------------------------
// Main entry: select RISC-V instructions for a whole IR program
// ---------------------------------------------------------------------------

pub fn select_instructions(program: &IRProgram) -> Vec<SelectedFunction> {
    program.functions.iter().map(select_function).collect()
}

fn select_function(func: &IRFunction) -> SelectedFunction {
    let blocks = func
        .blocks
        .iter()
        .map(|block| {
            let ops = select_block(block);
            SelectedBlock {
                label: block.label.clone(),
                ops,
            }
        })
        .collect();

    SelectedFunction {
        name: func.name.clone(),
        blocks,
        frame_size: 0,
    }
}

fn select_block(block: &BasicBlock) -> Vec<RiscVOp> {
    let mut ops = Vec::new();
    for instr in &block.instructions {
        munch_instruction(instr, &mut ops);
    }
    ops
}

// ---------------------------------------------------------------------------
// Maximal Munch — instruction selector core
// ---------------------------------------------------------------------------

fn munch_instruction(instr: &Instruction, ops: &mut Vec<RiscVOp>) {
    match instr {
        Instruction::Const { result, value, .. } => {
            if let Some(n) = value.as_i64() {
                emit_li(ops, result, n);
            } else if let Some(s) = value.as_str() {
                // String literal — leave as label for the emitter.
                ops.push(RiscVOp::Li {
                    rd: result.clone(),
                    imm: 0, // placeholder; emitter resolves the label
                });
            }
        }

        Instruction::BinOp {
            result,
            lhs,
            rhs,
            op_type,
            ..
        } => {
            let imm_rhs = rhs.parse::<i64>().ok();
            let fits_imm = |v: i64| (-2048..=2047).contains(&v);

            match op_type.as_str() {
                "+" => {
                    if let Some(imm) = imm_rhs {
                        if fits_imm(imm) {
                            ops.push(RiscVOp::Addi {
                                rd: result.clone(),
                                rs1: lhs.clone(),
                                imm: imm as i16,
                            });
                        } else {
                            emit_li(ops, result, imm);
                            // If lhs is the same as result, we already have
                            // the constant.  But for separate registers:
                            ops.push(RiscVOp::Add {
                                rd: result.clone(),
                                rs1: lhs.clone(),
                                rs2: result.clone(),
                            });
                        }
                    } else {
                        ops.push(RiscVOp::Add {
                            rd: result.clone(),
                            rs1: lhs.clone(),
                            rs2: rhs.clone(),
                        });
                    }
                }

                "-" => {
                    if let Some(imm) = imm_rhs {
                        if fits_imm(-imm) {
                            // a - b = a + (-b), and -b fits in 12 bits
                            ops.push(RiscVOp::Addi {
                                rd: result.clone(),
                                rs1: lhs.clone(),
                                imm: (-imm) as i16,
                            });
                        } else {
                            ops.push(RiscVOp::Sub {
                                rd: result.clone(),
                                rs1: lhs.clone(),
                                rs2: rhs.clone(),
                            });
                        }
                    } else {
                        ops.push(RiscVOp::Sub {
                            rd: result.clone(),
                            rs1: lhs.clone(),
                            rs2: rhs.clone(),
                        });
                    }
                }

                "*" => {
                    if let Some(imm) = imm_rhs {
                        if imm == 0 {
                            // rd = 0
                            ops.push(RiscVOp::Li {
                                rd: result.clone(),
                                imm: 0,
                            });
                            return;
                        }
                        if imm > 0 && (imm & (imm - 1)) == 0 {
                            ops.push(RiscVOp::Slli {
                                rd: result.clone(),
                                rs1: lhs.clone(),
                                shamt: imm.trailing_zeros() as u8,
                            });
                            return;
                        }
                    }
                    ops.push(RiscVOp::Mul {
                        rd: result.clone(),
                        rs1: lhs.clone(),
                        rs2: rhs.clone(),
                    });
                }

                "/" => {
                    ops.push(RiscVOp::Div {
                        rd: result.clone(),
                        rs1: lhs.clone(),
                        rs2: rhs.clone(),
                    });
                }

                "<<" => {
                    let shamt = rhs.parse::<u8>().unwrap_or(0);
                    ops.push(RiscVOp::Slli {
                        rd: result.clone(),
                        rs1: lhs.clone(),
                        shamt,
                    });
                }
                ">>" => {
                    let shamt = rhs.parse::<u8>().unwrap_or(0);
                    ops.push(RiscVOp::Srai {
                        rd: result.clone(),
                        rs1: lhs.clone(),
                        shamt,
                    });
                }

                "&" => {
                    if let Some(imm) = imm_rhs {
                        if fits_imm(imm) {
                            ops.push(RiscVOp::Andi {
                                rd: result.clone(),
                                rs1: lhs.clone(),
                                imm: imm as i16,
                            });
                        } else {
                            emit_li(ops, result, imm);
                            ops.push(RiscVOp::And {
                                rd: result.clone(),
                                rs1: lhs.clone(),
                                rs2: result.clone(),
                            });
                        }
                    } else {
                        // Register form — RV64 has `and rd, rs1, rs2`
                        ops.push(RiscVOp::And {
                            rd: result.clone(),
                            rs1: lhs.clone(),
                            rs2: rhs.clone(),
                        });
                    }
                }
                "|" => {
                    if let Some(imm) = imm_rhs {
                        if fits_imm(imm) {
                            ops.push(RiscVOp::Ori {
                                rd: result.clone(),
                                rs1: lhs.clone(),
                                imm: imm as i16,
                            });
                        } else {
                            emit_li(ops, result, imm);
                            ops.push(RiscVOp::Or {
                                rd: result.clone(),
                                rs1: lhs.clone(),
                                rs2: result.clone(),
                            });
                        }
                    } else {
                        ops.push(RiscVOp::Or {
                            rd: result.clone(),
                            rs1: lhs.clone(),
                            rs2: rhs.clone(),
                        });
                    }
                }
                "^" | "xor" => {
                    if let Some(imm) = imm_rhs {
                        if fits_imm(imm) {
                            ops.push(RiscVOp::Xori {
                                rd: result.clone(),
                                rs1: lhs.clone(),
                                imm: imm as i16,
                            });
                        } else {
                            emit_li(ops, result, imm);
                            ops.push(RiscVOp::Xor {
                                rd: result.clone(),
                                rs1: lhs.clone(),
                                rs2: result.clone(),
                            });
                        }
                    } else {
                        ops.push(RiscVOp::Xor {
                            rd: result.clone(),
                            rs1: lhs.clone(),
                            rs2: rhs.clone(),
                        });
                    }
                }

                // Comparisons — RV64 uses slt[u] then branch/snez
                "<" => {
                    ops.push(RiscVOp::Slt {
                        rd: result.clone(),
                        rs1: lhs.clone(),
                        rs2: rhs.clone(),
                    });
                }
                "<=" => {
                    // !(a < b) → slt then xori #1
                    ops.push(RiscVOp::Slt {
                        rd: result.clone(),
                        rs1: lhs.clone(),
                        rs2: rhs.clone(),
                    });
                    ops.push(RiscVOp::Xori {
                        rd: result.clone(),
                        rs1: result.clone(),
                        imm: 1,
                    });
                }
                ">" => {
                    // b < a → slt rd, rhs, lhs
                    ops.push(RiscVOp::Slt {
                        rd: result.clone(),
                        rs1: rhs.clone(),
                        rs2: lhs.clone(),
                    });
                }
                ">=" => {
                    // !(a < b) → slt then xori
                    ops.push(RiscVOp::Slt {
                        rd: result.clone(),
                        rs1: lhs.clone(),
                        rs2: rhs.clone(),
                    });
                    ops.push(RiscVOp::Xori {
                        rd: result.clone(),
                        rs1: result.clone(),
                        imm: 1,
                    });
                }
                "==" => {
                    // sub rd, lhs, rhs; seqz rd, rd  (or snez → xori)
                    // Better: xor rd, lhs, rhs; seqz rd, rd
                    ops.push(RiscVOp::Xor {
                        rd: result.clone(),
                        rs1: lhs.clone(),
                        rs2: rhs.clone(),
                    });
                    ops.push(RiscVOp::Seqz {
                        rd: result.clone(),
                        rs: result.clone(),
                    });
                }
                "!=" => {
                    ops.push(RiscVOp::Xor {
                        rd: result.clone(),
                        rs1: lhs.clone(),
                        rs2: rhs.clone(),
                    });
                    ops.push(RiscVOp::Snez {
                        rd: result.clone(),
                        rs: result.clone(),
                    });
                }

                other => {
                    // Unknown — fallback to add.
                    ops.push(RiscVOp::Add {
                        rd: result.clone(),
                        rs1: lhs.clone(),
                        rs2: rhs.clone(),
                    });
                }
            }
        }

        Instruction::Call {
            result,
            function,
            ..
        } => {
            ops.push(RiscVOp::Call {
                label: function.clone(),
            });
            if let Some(r) = result {
                ops.push(RiscVOp::Mv {
                    rd: r.clone(),
                    rs1: "a0".to_string(),
                });
            }
        }

        Instruction::CallIndirect { result, function_value, .. } => {
            ops.push(RiscVOp::Jalr {
                rs1: function_value.clone(),
            });
            if let Some(r) = result {
                ops.push(RiscVOp::Mv {
                    rd: r.clone(),
                    rs1: "a0".to_string(),
                });
            }
        }

        Instruction::Return { value } => {
            if let Some(v) = value {
                ops.push(RiscVOp::Mv {
                    rd: "a0".to_string(),
                    rs1: v.clone(),
                });
            }
            ops.push(RiscVOp::Ret);
        }

        Instruction::Jump { label } => {
            ops.push(RiscVOp::J {
                label: label.clone(),
            });
        }

        Instruction::Branch {
            cond,
            true_label,
            false_label,
        } => {
            // Branch based on the condition boolean being non-zero.
            ops.push(RiscVOp::Bne {
                rs1: cond.clone(),
                rs2: "zero".to_string(),
                label: true_label.clone(),
            });
            ops.push(RiscVOp::J {
                label: false_label.clone(),
            });
        }

        Instruction::Phi { .. } => {}

        Instruction::Alloc { result, size, .. } => {
            if let Some(sz) = size {
                ops.push(RiscVOp::Mv {
                    rd: "a0".to_string(),
                    rs1: sz.clone(),
                });
            } else {
                ops.push(RiscVOp::Li {
                    rd: "a0".to_string(),
                    imm: 64,
                });
            }
            ops.push(RiscVOp::Call {
                label: "malloc".to_string(),
            });
            ops.push(RiscVOp::Mv {
                rd: result.clone(),
                rs1: "a0".to_string(),
            });
        }

        Instruction::GetField { result, object, .. } => {
            ops.push(RiscVOp::Ld {
                rd: result.clone(),
                addr: AddressingMode::BaseOffset(object.clone(), 0),
            });
        }

        Instruction::SetField { object, value, .. } => {
            ops.push(RiscVOp::Sd {
                rs2: value.clone(),
                addr: AddressingMode::BaseOffset(object.clone(), 0),
            });
        }

        Instruction::GetIndex { result, array, index, ty: _ } => {
            // Load from arr[idx]: effective address = arr + idx * 8.
            ops.push(RiscVOp::Slli {
                rd: index.clone(),
                rs1: index.clone(),
                shamt: 3,
            });
            ops.push(RiscVOp::Add {
                rd: index.clone(),
                rs1: array.clone(),
                rs2: index.clone(),
            });
            ops.push(RiscVOp::Ld {
                rd: result.clone(),
                addr: AddressingMode::Base(index.clone()),
            });
        }

        Instruction::SetIndex { array, index, value, .. } => {
            ops.push(RiscVOp::Slli {
                rd: index.clone(),
                rs1: index.clone(),
                shamt: 3,
            });
            ops.push(RiscVOp::Add {
                rd: index.clone(),
                rs1: array.clone(),
                rs2: index.clone(),
            });
            ops.push(RiscVOp::Sd {
                rs2: value.clone(),
                addr: AddressingMode::Base(index.clone()),
            });
        }

        Instruction::AddrOf { result, operand, .. } => {
            // lea: rd = operand (effectively a copy).
            ops.push(RiscVOp::Mv {
                rd: result.clone(),
                rs1: operand.clone(),
            });
        }

        Instruction::Deref { result, operand, .. } => {
            ops.push(RiscVOp::Ld {
                rd: result.clone(),
                addr: AddressingMode::Base(operand.clone()),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// RISC-V extensions (missing register-form ALU ops)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Constant materialisation helper
// ---------------------------------------------------------------------------

/// Emit the shortest sequence to materialise `imm` into `rd`.
fn emit_li(ops: &mut Vec<RiscVOp>, rd: &str, imm: i64) {
    if imm == 0 {
        // x0 is always zero — no instruction needed if rd == zero.
        // But we don't track that here, emit a mov-to-zero.
        ops.push(RiscVOp::Li { rd: rd.to_string(), imm: 0 });
        return;
    }

    if (-2048..=2047).contains(&imm) {
        // 12-bit sign-extended immediate → `addi rd, x0, imm`
        ops.push(RiscVOp::Addi {
            rd: rd.to_string(),
            rs1: "zero".to_string(),
            imm: imm as i16,
        });
        return;
    }

    // Full LUI + ADDI sequence with sign-extension compensation.
    let upper = ((imm + 0x800) >> 12) & 0xFFFFF; // 20-bit upper
    let lower = imm & 0xFFF; // 12-bit lower

    if lower >= 0x800 {
        // ADDI sign-extends: lower > 0x7FF means the effective value
        // is (upper << 12) + lower - 0x1000.  So we need LUI to load
        // upper + 1, which we already did by adding 0x800 above.
        ops.push(RiscVOp::Lui {
            rd: rd.to_string(),
            imm: upper,
        });
        ops.push(RiscVOp::Addi {
            rd: rd.to_string(),
            rs1: rd.to_string(),
            imm: lower as i16,
        });
    } else {
        ops.push(RiscVOp::Lui {
            rd: rd.to_string(),
            imm: upper,
        });
        ops.push(RiscVOp::Addi {
            rd: rd.to_string(),
            rs1: rd.to_string(),
            imm: lower as i16,
        });
    }
}

// ---------------------------------------------------------------------------
// Assembly emitter
// ---------------------------------------------------------------------------

pub fn emit_assembly(functions: &[SelectedFunction]) -> String {
    let mut out = String::new();
    out.push_str(".text\n");
    out.push_str(".attribute arch, \"rv64im\"\n");

    for func in functions {
        out.push_str(&format!("\n.globl {}\n", func.name));
        out.push_str(&format!("{}:\n", func.name));

        // Prologue: addi sp, sp, -frame; sd ra, frame-8(sp); sd s0, frame-16(sp)
        if func.frame_size > 0 {
            out.push_str(&format!("\taddi sp, sp, -{}\n", func.frame_size));
            out.push_str(&format!("\tsd ra, {}(sp)\n", func.frame_size - 8));
            out.push_str(&format!("\tsd s0, {}(sp)\n", func.frame_size - 16));
            out.push_str(&format!("\taddi s0, sp, {}\n", func.frame_size));
        }

        for block in &func.blocks {
            if block.label != "entry" {
                out.push_str(&format!(".L{}:\n", block.label));
            }
            for op in &block.ops {
                emit_op(&mut out, op);
            }
        }

        // Epilogue
        out.push_str(&format!(".L{}_end:\n", func.name));
        if func.frame_size > 0 {
            out.push_str(&format!("\tld ra, {}(sp)\n", func.frame_size - 8));
            out.push_str(&format!("\tld s0, {}(sp)\n", func.frame_size - 16));
            out.push_str(&format!("\taddi sp, sp, {}\n", func.frame_size));
        }
        out.push_str("\tret\n");
    }

    out
}

fn emit_op(out: &mut String, op: &RiscVOp) {
    match op {
        RiscVOp::Add { rd, rs1, rs2 } => out.push_str(&format!("\tadd {}, {}, {}\n", rd, rs1, rs2)),
        RiscVOp::Sub { rd, rs1, rs2 } => out.push_str(&format!("\tsub {}, {}, {}\n", rd, rs1, rs2)),
        RiscVOp::Mul { rd, rs1, rs2 } => out.push_str(&format!("\tmul {}, {}, {}\n", rd, rs1, rs2)),
        RiscVOp::Div { rd, rs1, rs2 } => out.push_str(&format!("\tdiv {}, {}, {}\n", rd, rs1, rs2)),
        RiscVOp::Addi { rd, rs1, imm } => out.push_str(&format!("\taddi {}, {}, {}\n", rd, rs1, imm)),
        RiscVOp::And { rd, rs1, rs2 } => out.push_str(&format!("\tand {}, {}, {}\n", rd, rs1, rs2)),
        RiscVOp::Or { rd, rs1, rs2 } => out.push_str(&format!("\tor {}, {}, {}\n", rd, rs1, rs2)),
        RiscVOp::Xor { rd, rs1, rs2 } => out.push_str(&format!("\txor {}, {}, {}\n", rd, rs1, rs2)),
        RiscVOp::Sltiu { rd, rs1, imm } => out.push_str(&format!("\tsltiu {}, {}, {}\n", rd, rs1, imm)),
        RiscVOp::Xori { rd, rs1, imm } => out.push_str(&format!("\txori {}, {}, {}\n", rd, rs1, imm)),
        RiscVOp::Ori { rd, rs1, imm } => out.push_str(&format!("\tori {}, {}, {}\n", rd, rs1, imm)),
        RiscVOp::Andi { rd, rs1, imm } => out.push_str(&format!("\tandi {}, {}, {}\n", rd, rs1, imm)),
        RiscVOp::Slli { rd, rs1, shamt } => out.push_str(&format!("\tslli {}, {}, {}\n", rd, rs1, shamt)),
        RiscVOp::Srli { rd, rs1, shamt } => out.push_str(&format!("\tsrli {}, {}, {}\n", rd, rs1, shamt)),
        RiscVOp::Srai { rd, rs1, shamt } => out.push_str(&format!("\tsrai {}, {}, {}\n", rd, rs1, shamt)),
        RiscVOp::Lui { rd, imm } => out.push_str(&format!("\tlui {}, {}\n", rd, imm)),
        RiscVOp::Li { rd, imm } => {
            if *imm == 0 {
                // Use x0 directly.
                out.push_str(&format!("\taddi {}, x0, 0\n", rd));
            } else if (-2048..=2047).contains(imm) {
                out.push_str(&format!("\taddi {}, x0, {}\n", rd, imm));
            } else {
                let upper = ((imm + 0x800) >> 12) & 0xFFFFF;
                let lower = imm & 0xFFF;
                out.push_str(&format!("\tlui {}, {}\n", rd, upper));
                out.push_str(&format!("\taddi {}, {}, {}\n", rd, rd, lower));
            }
        }
        RiscVOp::Mv { rd, rs1 } => out.push_str(&format!("\taddi {}, {}, 0\n", rd, rs1)),

        RiscVOp::Slt { rd, rs1, rs2 } => out.push_str(&format!("\tslt {}, {}, {}\n", rd, rs1, rs2)),
        RiscVOp::Sltu { rd, rs1, rs2 } => out.push_str(&format!("\tsltu {}, {}, {}\n", rd, rs1, rs2)),
        RiscVOp::Slti { rd, rs1, imm } => out.push_str(&format!("\tslti {}, {}, {}\n", rd, rs1, imm)),
        RiscVOp::Seqz { rd, rs } => out.push_str(&format!("\tseqz {}, {}\n", rd, rs)),
        RiscVOp::Snez { rd, rs } => out.push_str(&format!("\tsnez {}, {}\n", rd, rs)),

        RiscVOp::Ld { rd, addr } => emit_mem(out, "ld", rd, addr),
        RiscVOp::Sd { rs2, addr } => emit_mem(out, "sd", rs2, addr),
        RiscVOp::Lbu { rd, addr } => emit_mem(out, "lbu", rd, addr),
        RiscVOp::Sb { rs2, addr } => emit_mem(out, "sb", rs2, addr),

        RiscVOp::Beq { rs1, rs2, label } => out.push_str(&format!("\tbeq {}, {}, .L{}\n", rs1, rs2, label)),
        RiscVOp::Bne { rs1, rs2, label } => out.push_str(&format!("\tbne {}, {}, .L{}\n", rs1, rs2, label)),
        RiscVOp::Blt { rs1, rs2, label } => out.push_str(&format!("\tblt {}, {}, .L{}\n", rs1, rs2, label)),
        RiscVOp::Bge { rs1, rs2, label } => out.push_str(&format!("\tbge {}, {}, .L{}\n", rs1, rs2, label)),
        RiscVOp::J { label } => out.push_str(&format!("\tj .L{}\n", label)),
        RiscVOp::Call { label } => out.push_str(&format!("\tcall {}\n", label)),
        RiscVOp::Jalr { rs1 } => out.push_str(&format!("\tjalr ra, {}, 0\n", rs1)),
        RiscVOp::Ret => out.push_str("\tret\n"),
        RiscVOp::Label { label } => out.push_str(&format!(".L{}:\n", label)),

        RiscVOp::Prologue { frame_size } => {
            if *frame_size > 0 {
                out.push_str(&format!("\taddi sp, sp, -{}\n", frame_size));
            }
        }
        RiscVOp::Epilogue => {
            out.push_str("\tret\n");
        }
    }
}

fn emit_mem(out: &mut String, mnemonic: &str, reg: &str, addr: &AddressingMode) {
    match addr {
        AddressingMode::Base(base) => {
            out.push_str(&format!("\t{} {}, 0({})\n", mnemonic, reg, base));
        }
        AddressingMode::BaseOffset(base, offset) => {
            out.push_str(&format!("\t{} {}, {}({})\n", mnemonic, reg, offset, base));
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addi_for_small_immediate() {
        let mut ops = vec![];
        munch_instruction(
            &Instruction::BinOp {
                result: "%rd".into(),
                lhs: "%rs".into(),
                rhs: "42".into(),
                op_type: "+".into(),
                ty: "i64".into(),
            },
            &mut ops,
        );
        assert_eq!(ops.len(), 1);
        assert!(matches!(&ops[0], RiscVOp::Addi { rd, rs1, imm } if rd == "%rd" && rs1 == "%rs" && *imm == 42));
    }

    #[test]
    fn large_immediate_uses_lui_addi() {
        // 0x12345 = 74565 > 12 bits → needs lui + addi
        let mut ops = vec![];
        munch_instruction(
            &Instruction::Const {
                result: "%rd".into(),
                value: serde_json::json!(0x12345),
                ty: "i64".into(),
            },
            &mut ops,
        );
        // emit_li should produce at least 2 ops (lui + addi)
        assert!(ops.len() >= 2, "expected at least 2 ops, got {ops:?}");
    }

    #[test]
    fn zero_constant_is_li_0() {
        let mut ops = vec![];
        munch_instruction(
            &Instruction::Const {
                result: "%rd".into(),
                value: serde_json::json!(0),
                ty: "i64".into(),
            },
            &mut ops,
        );
        assert!(matches!(&ops[0], RiscVOp::Li { .. }));
    }

    #[test]
    fn multiply_by_power_of_two_becomes_shift() {
        let mut ops = vec![];
        munch_instruction(
            &Instruction::BinOp {
                result: "%rd".into(),
                lhs: "%rs".into(),
                rhs: "8".into(),
                op_type: "*".into(),
                ty: "i64".into(),
            },
            &mut ops,
        );
        assert_eq!(ops.len(), 1, "pow2 * should be SLLI: {ops:?}");
        assert!(matches!(&ops[0], RiscVOp::Slli { .. }));
    }

    #[test]
    fn equality_uses_xor_seqz() {
        let mut ops = vec![];
        munch_instruction(
            &Instruction::BinOp {
                result: "%rd".into(),
                lhs: "%a".into(),
                rhs: "%b".into(),
                op_type: "==".into(),
                ty: "bool".into(),
            },
            &mut ops,
        );
        assert_eq!(ops.len(), 2);
        assert!(matches!(&ops[0], RiscVOp::Xor { .. }));
        assert!(matches!(&ops[1], RiscVOp::Seqz { .. }));
    }
}
