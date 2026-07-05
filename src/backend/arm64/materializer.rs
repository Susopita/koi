//! Greedy binary decomposition of 64-bit constants.
//!
//! The instruction selector naively emits `Movz` + `Movk` sequences for
//! every large constant.  This pass re-materialises them with the
//! **shortest possible** `MOVZ`/`MOVK` sequence: for a 64-bit constant,
//! the greedy algorithm emits `MOVZ` for the first non-zero 16-bit slice
//! and `MOVK` for each subsequent non-zero slice, skipping zero slices.
//!
//! # Algorithm
//!
//! 1. Split the 64-bit value into four 16-bit chunks (`[b0, b1, b2, b3]`).
//! 2. Find the first non-zero chunk — that becomes a `MOVZ` (clears upper
//!    bits implicitly).
//! 3. For every *other* non-zero chunk, emit a `MOVK` (does not clear
//!    upper bits, only overwrites the targeted 16-bit lane).
//!
//! Result: at most 4 instructions, often 2-3 for typical constants.

use crate::backend::arm64::instruction_select::{A64Op, SelectedFunction, SelectedBlock};

/// Run the materializer over a list of selected functions.
pub fn materialize_constants(functions: &mut [SelectedFunction]) {
    for func in functions {
        for block in &mut func.blocks {
            let mut new_ops: Vec<A64Op> = Vec::with_capacity(block.ops.len());
            let mut i = 0;
            while i < block.ops.len() {
                if let A64Op::Movz { rd, imm, shift } = &block.ops[i] {
                    if *shift == 0 {
                        // Check if the next ops are Movk building on the same rd.
                        let (fully_materialized, consumed) =
                            collect_movk_sequence(&block.ops[i..], rd);
                        if let Some(value) = fully_materialized {
                            // Re-emit with optimal greed sequence.
                            emit_greedy(&mut new_ops, rd, value);
                            i += consumed;
                            continue;
                        }
                    }
                }
                new_ops.push(block.ops[i].clone());
                i += 1;
            }
            block.ops = new_ops;
        }
    }
}

/// Collect consecutive Movk ops writing to `rd`, reconstruct the full value.
/// Returns `(Some(value), count)` if a Movz+Movk sequence is found,
/// or `(None, 1)` if not.
fn collect_movk_sequence(ops: &[A64Op], rd: &str) -> (Option<u64>, usize) {
    let first = match &ops[0] {
        A64Op::Movz { rd: r, imm, shift } if r == rd && *shift == 0 => *imm as u64,
        _ => return (None, 1),
    };

    let mut value = first;
    let mut consumed = 1;

    for op in &ops[1..] {
        match op {
            A64Op::Movk { rd: r, imm, shift } if r == rd => {
                let shifted = (*imm as u64) << *shift;
                value |= shifted;
                consumed += 1;
            }
            _ => break,
        }
    }

    (Some(value), consumed)
}

/// Emit the shortest MOVZ/MOVK sequence for `value` into `rd`.
fn emit_greedy(ops: &mut Vec<A64Op>, rd: &str, value: u64) {
    let chunks: [u16; 4] = [
        (value & 0xFFFF) as u16,
        ((value >> 16) & 0xFFFF) as u16,
        ((value >> 32) & 0xFFFF) as u16,
        ((value >> 48) & 0xFFFF) as u16,
    ];

    // Find the first non-zero chunk (the MOVZ position).
    let first = chunks.iter().position(|c| *c != 0).unwrap_or(0);

    for (i, chunk) in chunks.iter().enumerate() {
        if *chunk == 0 {
            continue;
        }
        if i == first {
            ops.push(A64Op::Movz {
                rd: rd.to_string(),
                imm: *chunk,
                shift: (i * 16) as u8,
            });
        } else {
            ops.push(A64Op::Movk {
                rd: rd.to_string(),
                imm: *chunk,
                shift: (i * 16) as u8,
            });
        }
    }

    // If the entire value is zero, emit a single `mov rd, #0`.
    if chunks.iter().all(|c| *c == 0) {
        ops.push(A64Op::Movz {
            rd: rd.to_string(),
            imm: 0,
            shift: 0,
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &[A64Op]) -> Vec<A64Op> {
        let mut func = SelectedFunction {
            name: "test".into(),
            blocks: vec![SelectedBlock {
                label: "entry".into(),
                ops: source.to_vec(),
            }],
            frame_size: 0,
            parameters: Vec::new(),
            used_callee_saved: Vec::new(),
        };
        let funcs = &mut [func];
        materialize_constants(funcs);
        funcs[0].blocks.swap_remove(0).ops
    }

    #[test]
    fn small_constant_stays_as_movz() {
        let ops = run(&[A64Op::Movz {
            rd: "x0".into(),
            imm: 42,
            shift: 0,
        }]);
        assert_eq!(ops.len(), 1, "42 should be single movz");
        assert!(matches!(&ops[0], A64Op::Movz { imm: 42, .. }));
    }

    #[test]
    fn constant_0x1_0000_needs_movz_plus_movk() {
        // 0x10000 = 65536 = chunk[0] = 0, chunk[1] = 1
        let ops = run(&[
            A64Op::Movz {
                rd: "x0".into(),
                imm: 0,
                shift: 0,
            },
            A64Op::Movk {
                rd: "x0".into(),
                imm: 1,
                shift: 16,
            },
        ]);
        assert_eq!(ops.len(), 1, "0x10000 should collapse to one movz at shift 16");
        assert!(matches!(&ops[0], A64Op::Movz { imm: 1, shift: 16, .. }));
    }

    #[test]
    fn constant_0x0001_0002_0003_0004_is_four_chunks() {
        let val = 0x0001_0002_0003_0004u64;
        let ops = run(&[
            A64Op::Movz {
                rd: "x0".into(),
                imm: (val & 0xFFFF) as u16,
                shift: 0,
            },
            A64Op::Movk {
                rd: "x0".into(),
                imm: ((val >> 16) & 0xFFFF) as u16,
                shift: 16,
            },
            A64Op::Movk {
                rd: "x0".into(),
                imm: ((val >> 32) & 0xFFFF) as u16,
                shift: 32,
            },
            A64Op::Movk {
                rd: "x0".into(),
                imm: ((val >> 48) & 0xFFFF) as u16,
                shift: 48,
            },
        ]);
        assert_eq!(ops.len(), 4, "0x0001000200030004 needs 4 insns");
    }

    #[test]
    fn zero_constant_emits_movz_0() {
        let ops = run(&[A64Op::Movz {
            rd: "x0".into(),
            imm: 0,
            shift: 0,
        }]);
        assert_eq!(ops.len(), 1);
        assert!(matches!(&ops[0], A64Op::Movz { imm: 0, .. }));
    }
}
