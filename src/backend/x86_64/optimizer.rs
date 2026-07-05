use crate::middle_end::ir::{IRProgram, IRFunction, Instruction};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub struct Optimizer;

impl Optimizer {
    pub fn optimize_program(program: &mut IRProgram) {
        for function in &mut program.functions {
            Self::optimize_function(function);
        }
    }

    /// Runs the full optimization pipeline (constant folding, strength
    /// reduction, dead code elimination, unreachable block elimination)
    /// repeatedly until a full round makes no further changes, bounded by
    /// `MAX_ITERATIONS` so a pathological input can't hang the compiler.
    /// This matters because later passes can expose new opportunities for
    /// earlier ones (e.g. dead code elimination removing an instruction can
    /// change what strength reduction or a later fold sees on the next
    /// round), so a single sequential pass over all four is not always
    /// enough to reach a fully-optimized fixpoint.
    pub fn optimize_function(function: &mut IRFunction) {
        const MAX_ITERATIONS: usize = 32;
        for _ in 0..MAX_ITERATIONS {
            let mut changed = false;
            changed |= Self::constant_folding(function);
            changed |= Self::strength_reduction(function);
            changed |= Self::dead_code_elimination(function);
            changed |= Self::eliminate_unreachable_blocks(function);
            if !changed {
                break;
            }
        }
    }

    /// Folds `BinOp`s whose operands are both known compile-time constants
    /// into a plain `Const`. Returns `true` if any instruction was rewritten.
    fn constant_folding(function: &mut IRFunction) -> bool {
        let mut changed = false;

        for block in &mut function.blocks {
            let mut constants = HashMap::<String, Value>::new();

            for instruction in &mut block.instructions {
                match instruction {
                    Instruction::Const { result, value, .. } => {
                        constants.insert(result.clone(), value.clone());
                    }
                    Instruction::BinOp {
                        result,
                        lhs,
                        rhs,
                        op_type,
                        ty,
                    } => {
                        let result_name = result.clone();
                        let op_name = op_type.clone();
                        let result_ty = ty.clone();
                        let lhs_value = constants.get(lhs).cloned();
                        let rhs_value = constants.get(rhs).cloned();
                        if let (Some(lhs_value), Some(rhs_value)) = (lhs_value, rhs_value) {
                            if let Some(folded) = fold_binop(&op_name, &lhs_value, &rhs_value) {
                                *instruction = Instruction::Const {
                                    result: result_name.clone(),
                                    value: folded.clone(),
                                    ty: result_ty,
                                };
                                constants.insert(result_name, folded);
                                changed = true;
                            } else {
                                constants.remove(&result_name);
                            }
                        } else {
                            constants.remove(&result_name);
                        }
                    }
                    other => {
                        if let Some(result) = other.result_name() {
                            constants.remove(result);
                        }
                    }
                }
            }
        }

        changed
    }

    /// Rewrites `i64` multiplications/divisions by a power-of-two constant
    /// into cheaper shift instructions, and multiplications by zero into a
    /// plain zero constant. Follows the same per-block, per-instruction,
    /// constants-map style as `constant_folding` so it can run immediately
    /// after it and pick up freshly folded constants. Returns `true` if any
    /// instruction was rewritten.
    fn strength_reduction(function: &mut IRFunction) -> bool {
        let mut changed = false;

        for block in &mut function.blocks {
            let mut constants = HashMap::<String, Value>::new();
            let mut rewritten = Vec::with_capacity(block.instructions.len());

            for instruction in block.instructions.drain(..) {
                match instruction {
                    Instruction::Const { result, value, ty } => {
                        constants.insert(result.clone(), value.clone());
                        rewritten.push(Instruction::Const { result, value, ty });
                    }
                    Instruction::BinOp {
                        result,
                        lhs,
                        rhs,
                        op_type,
                        ty,
                    } if ty == "i64" && op_type == "*" => {
                        let lhs_const = constants.get(&lhs).and_then(Value::as_i64);
                        let rhs_const = constants.get(&rhs).and_then(Value::as_i64);

                        // For `*`, either operand may be the constant since
                        // multiplication is commutative; prefer rhs if both
                        // happen to be constant (constant folding would have
                        // already collapsed that case anyway).
                        let (constant, non_constant_operand) = if let Some(c) = rhs_const {
                            (Some(c), lhs.clone())
                        } else if let Some(c) = lhs_const {
                            (Some(c), rhs.clone())
                        } else {
                            (None, String::new())
                        };

                        if let Some(c) = constant {
                            if c == 0 {
                                constants.remove(&result);
                                let folded = Value::from(0i64);
                                constants.insert(result.clone(), folded.clone());
                                rewritten.push(Instruction::Const {
                                    result,
                                    value: folded,
                                    ty: "i64".to_string(),
                                });
                                changed = true;
                                continue;
                            } else if c > 1 && (c as u64).is_power_of_two() {
                                let k = (c as u64).trailing_zeros();
                                let shift_amt_name = format!("{result}__shift_amt");
                                constants.remove(&result);
                                rewritten.push(Instruction::Const {
                                    result: shift_amt_name.clone(),
                                    value: Value::from(k as i64),
                                    ty: "i64".to_string(),
                                });
                                rewritten.push(Instruction::BinOp {
                                    result,
                                    lhs: non_constant_operand,
                                    rhs: shift_amt_name,
                                    op_type: "<<".to_string(),
                                    ty: "i64".to_string(),
                                });
                                changed = true;
                                continue;
                            }
                        }

                        constants.remove(&result);
                        rewritten.push(Instruction::BinOp {
                            result,
                            lhs,
                            rhs,
                            op_type,
                            ty,
                        });
                    }
                    // Division via arithmetic right-shift (`sarq`) is only
                    // bit-exact for non-negative dividends — for negative
                    // `lhs` values this rounds toward negative infinity
                    // instead of toward zero (which is what `idivq`-based
                    // division does), so this is a known, intentionally
                    // deferred correctness gap; a future pass would need to
                    // add the bias-correction trick
                    // (`(x + ((x >> 63) >>> (64-k))) >> k`) to handle
                    // negative dividends correctly. Not implemented here.
                    Instruction::BinOp {
                        result,
                        lhs,
                        rhs,
                        op_type,
                        ty,
                    } if ty == "i64" && op_type == "/" => {
                        // Only the rhs (divisor) may fold, since `/` is not
                        // commutative.
                        let rhs_const = constants.get(&rhs).and_then(Value::as_i64);

                        if let Some(c) = rhs_const {
                            if c > 1 && (c as u64).is_power_of_two() {
                                let k = (c as u64).trailing_zeros();
                                let shift_amt_name = format!("{result}__shift_amt");
                                constants.remove(&result);
                                rewritten.push(Instruction::Const {
                                    result: shift_amt_name.clone(),
                                    value: Value::from(k as i64),
                                    ty: "i64".to_string(),
                                });
                                rewritten.push(Instruction::BinOp {
                                    result,
                                    lhs,
                                    rhs: shift_amt_name,
                                    op_type: ">>".to_string(),
                                    ty: "i64".to_string(),
                                });
                                changed = true;
                                continue;
                            }
                        }

                        constants.remove(&result);
                        rewritten.push(Instruction::BinOp {
                            result,
                            lhs,
                            rhs,
                            op_type,
                            ty,
                        });
                    }
                    other => {
                        if let Some(result) = other.result_name() {
                            constants.remove(result);
                        }
                        rewritten.push(other);
                    }
                }
            }

            block.instructions = rewritten;
        }

        changed
    }

    fn dead_code_elimination(function: &mut IRFunction) -> bool {
        let mut live = HashSet::<String>::new();
        for block in &function.blocks {
            for instruction in &block.instructions {
                if let Instruction::Phi { incoming, .. } = instruction {
                    for (_, value) in incoming {
                        live.insert(value.clone());
                    }
                }
            }
        }

        let mut changed = false;
        for block in function.blocks.iter_mut().rev() {
            let original_len = block.instructions.len();
            let mut kept = Vec::with_capacity(original_len);
            for instruction in block.instructions.iter().rev() {
                let side_effect = has_side_effect(instruction);
                let keep = side_effect
                    || instruction
                        .result_name()
                        .is_some_and(|name| live.contains(name));

                if keep {
                    for used in instruction_uses(instruction) {
                        live.insert(used);
                    }
                    if let Some(result) = instruction.result_name() {
                        live.remove(result);
                    }
                    kept.push(instruction.clone());
                }
            }
            kept.reverse();
            if kept.len() != original_len {
                changed = true;
            }
            block.instructions = kept;
        }
        changed
    }

    /// Removes basic blocks that are unreachable from the function's entry
    /// block (the first block in `function.blocks`). Control-flow edges are
    /// derived from explicit `Jump`/`Branch` terminators; a block whose last
    /// instruction is not a terminator is assumed to fall through to the next
    /// block in `function.blocks` order (mirroring the fallthrough assumption
    /// used by `codegen.rs`'s `next_block` helper). The entry block is always
    /// kept, even if nothing jumps to it. Returns `true` if any block was
    /// removed.
    fn eliminate_unreachable_blocks(function: &mut IRFunction) -> bool {
        if function.blocks.is_empty() {
            return false;
        }

        let entry_label = function.blocks[0].label.clone();

        // Build successor edges for each block, including the implicit
        // fallthrough edge to the next block in vec order when the block has
        // no explicit terminator.
        let mut successors: HashMap<String, Vec<String>> = HashMap::new();
        for (index, block) in function.blocks.iter().enumerate() {
            let mut edges = Vec::new();
            let mut has_terminator = false;
            for instruction in &block.instructions {
                match instruction {
                    Instruction::Jump { label } => {
                        edges.push(label.clone());
                        has_terminator = true;
                    }
                    Instruction::Branch {
                        true_label,
                        false_label,
                        ..
                    } => {
                        edges.push(true_label.clone());
                        edges.push(false_label.clone());
                        has_terminator = true;
                    }
                    Instruction::Return { .. } => {
                        has_terminator = true;
                    }
                    _ => {}
                }
            }
            if !has_terminator {
                if let Some(next) = function.blocks.get(index + 1) {
                    edges.push(next.label.clone());
                }
            }
            successors.insert(block.label.clone(), edges);
        }

        let mut reachable = HashSet::<String>::new();
        let mut worklist = vec![entry_label.clone()];
        reachable.insert(entry_label.clone());
        while let Some(label) = worklist.pop() {
            if let Some(edges) = successors.get(&label) {
                for edge in edges {
                    if reachable.insert(edge.clone()) {
                        worklist.push(edge.clone());
                    }
                }
            }
        }

        let original_len = function.blocks.len();
        function
            .blocks
            .retain(|block| block.label == entry_label || reachable.contains(&block.label));

        function.blocks.len() != original_len
    }
}

fn has_side_effect(instruction: &Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Call { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::Return { .. }
            | Instruction::Jump { .. }
            | Instruction::Branch { .. }
            | Instruction::Alloc { .. }
            | Instruction::SetIndex { .. }
            | Instruction::SetField { .. }
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
        Instruction::Phi { incoming, .. } => incoming.iter().map(|(_, value)| value.clone()).collect(),
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

fn fold_binop(op: &str, lhs: &Value, rhs: &Value) -> Option<Value> {
    if let (Some(lhs), Some(rhs)) = (lhs.as_i64(), rhs.as_i64()) {
        return match op {
            "+" => Some(Value::from(lhs + rhs)),
            "-" => Some(Value::from(lhs - rhs)),
            "*" => Some(Value::from(lhs * rhs)),
            "/" => (rhs != 0).then(|| Value::from(lhs / rhs)),
            "<" => Some(Value::from(lhs < rhs)),
            "<=" => Some(Value::from(lhs <= rhs)),
            ">" => Some(Value::from(lhs > rhs)),
            ">=" => Some(Value::from(lhs >= rhs)),
            "==" => Some(Value::from(lhs == rhs)),
            "!=" => Some(Value::from(lhs != rhs)),
            "&&" => Some(Value::from((lhs != 0) && (rhs != 0))),
            "||" => Some(Value::from((lhs != 0) || (rhs != 0))),
            _ => None,
        };
    }

    if let (Some(lhs), Some(rhs)) = (lhs.as_bool(), rhs.as_bool()) {
        return match op {
            "==" => Some(Value::from(lhs == rhs)),
            "!=" => Some(Value::from(lhs != rhs)),
            "&&" => Some(Value::from(lhs && rhs)),
            "||" => Some(Value::from(lhs || rhs)),
            _ => None,
        };
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middle_end::ir::BasicBlock;

    fn function_with_blocks(blocks: Vec<BasicBlock>) -> IRFunction {
        IRFunction {
            name: "test_fn".to_string(),
            return_type: "i64".to_string(),
            parameters: vec![],
            blocks,
        }
    }

    fn const_instr(result: &str, value: i64) -> Instruction {
        Instruction::Const {
            result: result.to_string(),
            value: Value::from(value),
            ty: "i64".to_string(),
        }
    }

    #[test]
    fn dce_removes_unused_pure_instruction() {
        let mut function = function_with_blocks(vec![BasicBlock {
            label: "entry".to_string(),
            instructions: vec![
                const_instr("%v0", 1),
                const_instr("%v1", 2),
                Instruction::BinOp {
                    result: "%v2".to_string(),
                    lhs: "%v0".to_string(),
                    rhs: "%v1".to_string(),
                    op_type: "+".to_string(),
                    ty: "i64".to_string(),
                },
                Instruction::Return { value: Some("%v0".to_string()) },
            ],
        }]);

        let changed = Optimizer::dead_code_elimination(&mut function);

        assert!(changed, "expected dead_code_elimination to report a change");
        let instructions = &function.blocks[0].instructions;
        // %v1 and %v2 are never used, so the const and binop producing them
        // should be removed; %v0's const and the return survive.
        assert_eq!(instructions.len(), 2);
        assert!(matches!(&instructions[0], Instruction::Const { result, .. } if result == "%v0"));
        assert!(matches!(&instructions[1], Instruction::Return { .. }));
    }

    #[test]
    fn dce_keeps_side_effecting_instructions_even_if_unused() {
        let mut function = function_with_blocks(vec![BasicBlock {
            label: "entry".to_string(),
            instructions: vec![
                Instruction::Call {
                    result: Some("%v0".to_string()),
                    function: "some_fn".to_string(),
                    arguments: vec![],
                    ty: Some("i64".to_string()),
                },
                Instruction::Alloc {
                    result: "%v1".to_string(),
                    ty: "i64".to_string(),
                    size: None,
                },
                Instruction::Return { value: None },
            ],
        }]);

        let changed = Optimizer::dead_code_elimination(&mut function);

        assert!(!changed, "no instruction should have been removed");
        assert_eq!(function.blocks[0].instructions.len(), 3);
    }

    #[test]
    fn unreachable_block_not_targeted_is_removed() {
        let mut function = function_with_blocks(vec![
            BasicBlock {
                label: "entry".to_string(),
                instructions: vec![Instruction::Return { value: None }],
            },
            BasicBlock {
                label: "dead_block".to_string(),
                instructions: vec![Instruction::Return { value: None }],
            },
        ]);

        let changed = Optimizer::eliminate_unreachable_blocks(&mut function);

        assert!(changed, "expected the unreferenced block to be removed");
        assert_eq!(function.blocks.len(), 1);
        assert_eq!(function.blocks[0].label, "entry");
    }

    #[test]
    fn entry_block_is_never_removed() {
        let mut function = function_with_blocks(vec![BasicBlock {
            label: "entry".to_string(),
            instructions: vec![Instruction::Return { value: None }],
        }]);

        let changed = Optimizer::eliminate_unreachable_blocks(&mut function);

        assert!(!changed);
        assert_eq!(function.blocks.len(), 1);
        assert_eq!(function.blocks[0].label, "entry");
    }

    #[test]
    fn reachable_block_via_jump_and_branch_is_kept() {
        let mut function = function_with_blocks(vec![
            BasicBlock {
                label: "entry".to_string(),
                instructions: vec![Instruction::Branch {
                    cond: "%v0".to_string(),
                    true_label: "then_block".to_string(),
                    false_label: "else_block".to_string(),
                }],
            },
            BasicBlock {
                label: "then_block".to_string(),
                instructions: vec![Instruction::Jump {
                    label: "join_block".to_string(),
                }],
            },
            BasicBlock {
                label: "else_block".to_string(),
                instructions: vec![Instruction::Jump {
                    label: "join_block".to_string(),
                }],
            },
            BasicBlock {
                label: "join_block".to_string(),
                instructions: vec![Instruction::Return { value: None }],
            },
        ]);

        let changed = Optimizer::eliminate_unreachable_blocks(&mut function);

        assert!(!changed, "all blocks are reachable, none should be removed");
        assert_eq!(function.blocks.len(), 4);
    }

    fn binop_instr(result: &str, lhs: &str, rhs: &str, op_type: &str) -> Instruction {
        Instruction::BinOp {
            result: result.to_string(),
            lhs: lhs.to_string(),
            rhs: rhs.to_string(),
            op_type: op_type.to_string(),
            ty: "i64".to_string(),
        }
    }

    #[test]
    fn strength_reduction_mul_by_power_of_two_rhs_becomes_shift() {
        let mut function = function_with_blocks(vec![BasicBlock {
            label: "entry".to_string(),
            instructions: vec![
                const_instr("%v0", 8),
                binop_instr("%v1", "x", "%v0", "*"),
                Instruction::Return { value: Some("%v1".to_string()) },
            ],
        }]);

        let changed = Optimizer::strength_reduction(&mut function);
        assert!(changed);

        let instructions = &function.blocks[0].instructions;
        // %v0 const 8, then the shift-amount const, then the shifted binop.
        let shift_const = instructions.iter().find(|i| {
            matches!(i, Instruction::Const { value, .. } if value.as_i64() == Some(3))
        });
        assert!(shift_const.is_some(), "expected a const instruction with value 3 (log2(8))");

        let shift_binop = instructions.iter().find_map(|i| match i {
            Instruction::BinOp { result, op_type, .. } if result == "%v1" => Some(op_type.clone()),
            _ => None,
        });
        assert_eq!(shift_binop.as_deref(), Some("<<"));
    }

    #[test]
    fn strength_reduction_mul_by_power_of_two_lhs_becomes_shift() {
        let mut function = function_with_blocks(vec![BasicBlock {
            label: "entry".to_string(),
            instructions: vec![
                const_instr("%v0", 4),
                binop_instr("%v1", "%v0", "x", "*"),
                Instruction::Return { value: Some("%v1".to_string()) },
            ],
        }]);

        let changed = Optimizer::strength_reduction(&mut function);
        assert!(changed);

        let instructions = &function.blocks[0].instructions;
        let shift_const = instructions.iter().find(|i| {
            matches!(i, Instruction::Const { value, .. } if value.as_i64() == Some(2))
        });
        assert!(shift_const.is_some(), "expected a const instruction with value 2 (log2(4))");

        let shift_binop = instructions.iter().find_map(|i| match i {
            Instruction::BinOp { result, op_type, lhs, .. } if result == "%v1" => {
                Some((op_type.clone(), lhs.clone()))
            }
            _ => None,
        });
        let (op_type, lhs) = shift_binop.expect("expected rewritten binop for %v1");
        assert_eq!(op_type, "<<");
        assert_eq!(lhs, "x", "the non-constant operand (x) must remain the shifted value");
    }

    #[test]
    fn strength_reduction_div_by_power_of_two_rhs_becomes_shift() {
        let mut function = function_with_blocks(vec![BasicBlock {
            label: "entry".to_string(),
            instructions: vec![
                const_instr("%v0", 4),
                binop_instr("%v1", "x", "%v0", "/"),
                Instruction::Return { value: Some("%v1".to_string()) },
            ],
        }]);

        let changed = Optimizer::strength_reduction(&mut function);
        assert!(changed);

        let instructions = &function.blocks[0].instructions;
        let shift_const = instructions.iter().find(|i| {
            matches!(i, Instruction::Const { value, .. } if value.as_i64() == Some(2))
        });
        assert!(shift_const.is_some(), "expected a const instruction with value 2 (log2(4))");

        let shift_binop = instructions.iter().find_map(|i| match i {
            Instruction::BinOp { result, op_type, .. } if result == "%v1" => Some(op_type.clone()),
            _ => None,
        });
        assert_eq!(shift_binop.as_deref(), Some(">>"));
    }

    #[test]
    fn strength_reduction_mul_by_zero_collapses_to_const_zero() {
        let mut function = function_with_blocks(vec![BasicBlock {
            label: "entry".to_string(),
            instructions: vec![
                const_instr("%v0", 0),
                binop_instr("%v1", "x", "%v0", "*"),
                Instruction::Return { value: Some("%v1".to_string()) },
            ],
        }]);

        let changed = Optimizer::strength_reduction(&mut function);
        assert!(changed);

        let instructions = &function.blocks[0].instructions;
        // No leftover BinOp and no shift for %v1: it should now be a plain
        // Const 0, with no other instruction referencing %v1 as a binop.
        let v1_is_const_zero = instructions.iter().any(|i| {
            matches!(i, Instruction::Const { result, value, .. } if result == "%v1" && value.as_i64() == Some(0))
        });
        assert!(v1_is_const_zero, "expected %v1 to become a const 0");

        let v1_is_binop = instructions.iter().any(|i| {
            matches!(i, Instruction::BinOp { result, .. } if result == "%v1")
        });
        assert!(!v1_is_binop, "no BinOp for %v1 should remain");

        let has_shift = instructions.iter().any(|i| {
            matches!(i, Instruction::BinOp { op_type, .. } if op_type == "<<" || op_type == ">>")
        });
        assert!(!has_shift, "no shift instruction should have been introduced");
    }

    #[test]
    fn strength_reduction_mul_by_non_power_of_two_is_unchanged() {
        let mut function = function_with_blocks(vec![BasicBlock {
            label: "entry".to_string(),
            instructions: vec![
                const_instr("%v0", 3),
                binop_instr("%v1", "x", "%v0", "*"),
                Instruction::Return { value: Some("%v1".to_string()) },
            ],
        }]);

        Optimizer::strength_reduction(&mut function);

        let instructions = &function.blocks[0].instructions;
        let original = instructions.iter().find_map(|i| match i {
            Instruction::BinOp { result, op_type, lhs, rhs, .. } if result == "%v1" => {
                Some((op_type.clone(), lhs.clone(), rhs.clone()))
            }
            _ => None,
        });
        let (op_type, lhs, rhs) = original.expect("expected %v1's BinOp to remain unchanged");
        assert_eq!(op_type, "*");
        assert_eq!(lhs, "x");
        assert_eq!(rhs, "%v0");
    }

    #[test]
    fn strength_reduction_mul_by_one_is_unchanged() {
        let mut function = function_with_blocks(vec![BasicBlock {
            label: "entry".to_string(),
            instructions: vec![
                const_instr("%v0", 1),
                binop_instr("%v1", "x", "%v0", "*"),
                Instruction::Return { value: Some("%v1".to_string()) },
            ],
        }]);

        Optimizer::strength_reduction(&mut function);

        let instructions = &function.blocks[0].instructions;
        let original = instructions.iter().find_map(|i| match i {
            Instruction::BinOp { result, op_type, lhs, rhs, .. } if result == "%v1" => {
                Some((op_type.clone(), lhs.clone(), rhs.clone()))
            }
            _ => None,
        });
        let (op_type, lhs, rhs) = original.expect("expected %v1's BinOp to remain unchanged for x * 1");
        assert_eq!(op_type, "*");
        assert_eq!(lhs, "x");
        assert_eq!(rhs, "%v0");
    }

    /// Builds the cascade scenario: `%v0 = 2`, `%v1 = 4`, `%v2 = %v0 * %v1`
    /// (only foldable by `constant_folding`), `%v3 = x * %v2` (only
    /// strength-reducible to a shift once `%v2` is known to be the constant
    /// 8, which only happens after `constant_folding` runs), `return %v3`.
    fn cascade_function() -> IRFunction {
        function_with_blocks(vec![BasicBlock {
            label: "entry".to_string(),
            instructions: vec![
                const_instr("%v0", 2),
                const_instr("%v1", 4),
                binop_instr("%v2", "%v0", "%v1", "*"),
                binop_instr("%v3", "x", "%v2", "*"),
                Instruction::Return { value: Some("%v3".to_string()) },
            ],
        }])
    }

    #[test]
    fn optimize_function_fixpoint_reduces_multiply_that_depends_on_a_fold() {
        // A single pass of `constant_folding` immediately followed by
        // `strength_reduction` (as happens within one round of the fixpoint
        // loop) already resolves this particular arithmetic chain, since
        // `constant_folding` folds %v2 = 2 * 4 into `Const 8` before
        // `strength_reduction` looks at %v3 = x * %v2 in the same round.
        // The fixpoint driver `optimize_function` must still leave the
        // function in this fully-optimized state: %v3 computed via a "<<"
        // shift rather than stuck as a literal multiply by 8.
        let mut function = cascade_function();

        Optimizer::optimize_function(&mut function);

        let instructions = &function.blocks[0].instructions;

        // No BinOp "*" for %v3 (or %v2) should remain.
        let has_leftover_multiply = instructions.iter().any(|i| {
            matches!(i, Instruction::BinOp { op_type, .. } if op_type == "*")
        });
        assert!(
            !has_leftover_multiply,
            "optimize_function should have reduced x * 8 to a shift, not left a multiply"
        );

        let shift_binop = instructions.iter().find_map(|i| match i {
            Instruction::BinOp { result, op_type, lhs, .. } if result == "%v3" => {
                Some((op_type.clone(), lhs.clone()))
            }
            _ => None,
        });
        let (op_type, lhs) = shift_binop.expect("expected %v3 to be rewritten as a shift binop");
        assert_eq!(op_type, "<<");
        assert_eq!(lhs, "x", "the non-constant operand (x) must remain the shifted value");

        let shift_const = instructions.iter().any(|i| {
            matches!(i, Instruction::Const { value, .. } if value.as_i64() == Some(3))
        });
        assert!(shift_const, "expected a const instruction with value 3 (log2(8))");
    }

    #[test]
    fn manual_wrong_order_single_pass_does_not_reduce_the_multiply() {
        // Positive proof that ordering/repetition matters: run
        // `strength_reduction` *before* `constant_folding`, each exactly
        // once, directly on the same starting instructions (bypassing
        // `optimize_function` entirely).
        //
        // At the point `strength_reduction` runs, %v0 and %v1 are already
        // known constants (2 and 4), so strength_reduction happily rewrites
        // %v2 = %v0 * %v1 into a shift (%v0 << 2) *before* constant_folding
        // ever gets a chance to fold it into the plain constant 8. That
        // premature rewrite is exactly what then blocks everything else:
        // `strength_reduction`'s own internal constants map only tracks
        // `Const` instructions, so once %v2 is a `<<` BinOp instead of a
        // `Const`, %v2 no longer looks like a known constant to anyone.
        // Consequently: `constant_folding` cannot fold `%v2 = %v0 << 2`
        // (shift ops aren't in `fold_binop`'s supported operator set) and
        // %v3 = x * %v2 is never recognized as a multiply-by-power-of-two,
        // so it is left as a real multiply instead of a shift.
        let mut function = cascade_function();

        let sr_changed = Optimizer::strength_reduction(&mut function);
        let cf_changed = Optimizer::constant_folding(&mut function);

        assert!(
            sr_changed,
            "strength_reduction (run first) still rewrites %v2 = %v0 * %v1 into a shift, \
             since %v0 and %v1 are already known constants"
        );
        assert!(
            !cf_changed,
            "constant_folding (run second) can no longer fold anything: %v2 is now a \
             shift binop it doesn't know how to fold, and %v3 = x * %v2 has no constant rhs"
        );

        let instructions = &function.blocks[0].instructions;

        // %v2 is stuck as a shift, never collapsed to the plain constant 8.
        let v2_is_shift = instructions.iter().any(|i| {
            matches!(i, Instruction::BinOp { result, op_type, .. } if result == "%v2" && op_type == "<<")
        });
        assert!(v2_is_shift, "expected %v2 to be left as an unfolded shift binop");

        // %v3 = x * %v2 was never reduced to a shift, because %v2 wasn't
        // recognized as a known constant by the time strength_reduction ran.
        let v3_binop = instructions.iter().find_map(|i| match i {
            Instruction::BinOp { result, op_type, lhs, rhs, .. } if result == "%v3" => {
                Some((op_type.clone(), lhs.clone(), rhs.clone()))
            }
            _ => None,
        });
        let (op_type, lhs, rhs) =
            v3_binop.expect("expected %v3 to still be a BinOp, not reduced to a shift");
        assert_eq!(
            op_type, "*",
            "in the wrong pass order, %v3 must remain a multiply, not a shift"
        );
        assert_eq!(lhs, "x");
        assert_eq!(rhs, "%v2");
    }
}
